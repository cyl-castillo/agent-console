use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use keyring::Entry;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

const KEYRING_SERVICE: &str = "agent-console.vault";
const PROJECT_FILE: &str = "vault.json";
const GLOBAL_FILE: &str = "agent-console-vault.json";
const VAULT_MD: &str = "VAULT.md";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Scope {
    #[serde(rename = "project")]
    Project,
    #[serde(rename = "global")]
    Global,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultEntryMeta {
    pub key: String,
    pub scope: Scope,
    /// One-line description for the agent ("Database password for staging").
    pub description: String,
    /// True = stored in OS keychain, never on disk. False = plain `value`.
    pub secret: bool,
    /// Present only when `secret == false`.
    pub value: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoreFile {
    entries: Vec<VaultEntryMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultEntryView {
    pub key: String,
    pub scope: Scope,
    pub description: String,
    pub secret: bool,
    pub has_value: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl From<&VaultEntryMeta> for VaultEntryView {
    fn from(m: &VaultEntryMeta) -> Self {
        Self {
            key: m.key.clone(),
            scope: m.scope.clone(),
            description: m.description.clone(),
            secret: m.secret,
            has_value: m.secret || m.value.is_some(),
            created_at_ms: m.created_at_ms,
            updated_at_ms: m.updated_at_ms,
        }
    }
}

/// Returns every entry in the project + global vaults (no values).
pub fn list(project_root: Option<&Path>) -> AppResult<Vec<VaultEntryView>> {
    let mut out: Vec<VaultEntryView> = Vec::new();
    if let Some(root) = project_root {
        let store = load_store(&project_path(root))?;
        out.extend(store.entries.iter().map(VaultEntryView::from));
    }
    let store = load_store(&global_path()?)?;
    out.extend(store.entries.iter().map(VaultEntryView::from));
    out.sort_by_key(|a| a.key.to_lowercase());
    Ok(out)
}

/// Insert or update an entry. For secrets, `value` is required on create.
/// On update with `value: None`, only the metadata changes.
pub fn upsert(
    project_root: Option<&Path>,
    scope: Scope,
    key: String,
    description: String,
    secret: bool,
    value: Option<String>,
) -> AppResult<VaultEntryView> {
    let key = key.trim().to_string();
    validate_key(&key)?;
    let path = store_path(project_root, &scope)?;
    let mut store = load_store(&path)?;
    let now = now_ms();

    let existing = store.entries.iter().position(|e| e.key == key);
    let (created_at, updated_at) = match existing {
        Some(i) => (store.entries[i].created_at_ms, now),
        None => (now, now),
    };

    // Persist value: secret → keyring; non-secret → JSON.
    let stored_value = if secret {
        if let Some(v) = value.as_ref() {
            keyring_entry(&scope, &key)?
                .set_password(v)
                .map_err(|e| AppError::Other(format!("keyring set: {e}")))?;
        } else if existing.is_none() {
            return Err(AppError::InvalidArgument(
                "secret entries require a value on create".into(),
            ));
        }
        None
    } else {
        // Migrating from secret → plain: drop any prior keyring entry.
        if let Ok(entry) = keyring_entry(&scope, &key) {
            let _ = entry.delete_credential();
        }
        value
            .clone()
            .or_else(|| existing.and_then(|i| store.entries[i].value.clone()))
    };

    let entry = VaultEntryMeta {
        key: key.clone(),
        scope: scope.clone(),
        description,
        secret,
        value: stored_value,
        created_at_ms: created_at,
        updated_at_ms: updated_at,
    };

    if let Some(i) = existing {
        store.entries[i] = entry.clone();
    } else {
        store.entries.push(entry.clone());
    }
    save_store(&path, &store)?;
    regenerate_vault_md(project_root, &scope, &store)?;
    Ok(VaultEntryView::from(&entry))
}

pub fn delete(project_root: Option<&Path>, scope: Scope, key: &str) -> AppResult<()> {
    let path = store_path(project_root, &scope)?;
    let mut store = load_store(&path)?;
    let before = store.entries.len();
    store.entries.retain(|e| e.key != key);
    if store.entries.len() == before {
        return Ok(()); // nothing to do
    }
    // Best-effort: remove from keyring too. Ignore errors (might be a plaintext entry).
    if let Ok(entry) = keyring_entry(&scope, key) {
        let _ = entry.delete_credential();
    }
    save_store(&path, &store)?;
    regenerate_vault_md(project_root, &scope, &store)?;
    Ok(())
}

/// Decrypt + return one value. Used by the UI's "reveal" action.
pub fn get_value(project_root: Option<&Path>, scope: Scope, key: &str) -> AppResult<String> {
    let path = store_path(project_root, &scope)?;
    let store = load_store(&path)?;
    let entry = store
        .entries
        .iter()
        .find(|e| e.key == key)
        .ok_or_else(|| AppError::NotFound(format!("vault entry '{key}'")))?;
    if entry.secret {
        keyring_entry(&scope, key)?
            .get_password()
            .map_err(|e| AppError::Other(format!("keyring get: {e}")))
    } else {
        Ok(entry.value.clone().unwrap_or_default())
    }
}

/// Build the {KEY: value} map used to seed new PTYs. Project entries win
/// over global on key collision.
pub fn env_for_spawn(project_root: Option<&Path>) -> AppResult<BTreeMap<String, String>> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();

    // Global first, then project overrides.
    let global = load_store(&global_path()?)?;
    for e in &global.entries {
        if let Ok(v) = read_value(e, Scope::Global) {
            out.insert(e.key.clone(), v);
        }
    }
    if let Some(root) = project_root {
        let p = load_store(&project_path(root))?;
        for e in &p.entries {
            if let Ok(v) = read_value(e, Scope::Project) {
                out.insert(e.key.clone(), v);
            }
        }
    }
    Ok(out)
}

fn read_value(e: &VaultEntryMeta, scope: Scope) -> AppResult<String> {
    if e.secret {
        keyring_entry(&scope, &e.key)?
            .get_password()
            .map_err(|e| AppError::Other(format!("keyring get: {e}")))
    } else {
        Ok(e.value.clone().unwrap_or_default())
    }
}

fn store_path(project_root: Option<&Path>, scope: &Scope) -> AppResult<PathBuf> {
    match scope {
        Scope::Project => {
            let root = project_root.ok_or_else(|| {
                AppError::InvalidArgument("project scope requires an open project".into())
            })?;
            Ok(project_path(root))
        }
        Scope::Global => global_path(),
    }
}

fn project_path(root: &Path) -> PathBuf {
    root.join(".claude").join(PROJECT_FILE)
}

fn global_path() -> AppResult<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| AppError::Other("no home dir".into()))?;
    Ok(home.join(".claude").join(GLOBAL_FILE))
}

fn load_store(path: &Path) -> AppResult<StoreFile> {
    if !path.exists() {
        return Ok(StoreFile::default());
    }
    let raw = fs::read_to_string(path)?;
    let store: StoreFile =
        serde_json::from_str(&raw).map_err(|e| AppError::Other(format!("vault parse: {e}")))?;
    Ok(store)
}

fn save_store(path: &Path, store: &StoreFile) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(store)
        .map_err(|e| AppError::Other(format!("vault serialize: {e}")))?;
    fs::write(path, raw)?;
    Ok(())
}

fn keyring_entry(scope: &Scope, key: &str) -> AppResult<Entry> {
    let service = format!(
        "{KEYRING_SERVICE}.{}",
        match scope {
            Scope::Project => "project",
            Scope::Global => "global",
        }
    );
    Entry::new(&service, key).map_err(|e| AppError::Other(format!("keyring open: {e}")))
}

fn validate_key(key: &str) -> AppResult<()> {
    if key.is_empty() {
        return Err(AppError::InvalidArgument("key cannot be empty".into()));
    }
    if !key.chars().next().unwrap().is_ascii_alphabetic() && !key.starts_with('_') {
        return Err(AppError::InvalidArgument(
            "key must start with a letter or underscore".into(),
        ));
    }
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(AppError::InvalidArgument(
            "key may only contain letters, digits, and underscores".into(),
        ));
    }
    Ok(())
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Rewrite the VAULT.md index next to the JSON store. Lists keys +
/// descriptions only — never values. This is what the agent reads to
/// discover what's available before asking the user.
fn regenerate_vault_md(
    project_root: Option<&Path>,
    scope: &Scope,
    store: &StoreFile,
) -> AppResult<()> {
    let dir = match scope {
        Scope::Project => match project_root {
            Some(root) => root.join(".claude"),
            None => return Ok(()),
        },
        Scope::Global => match dirs::home_dir() {
            Some(h) => h.join(".claude"),
            None => return Ok(()),
        },
    };
    fs::create_dir_all(&dir)?;
    let path = dir.join(VAULT_MD);

    let mut md = String::new();
    md.push_str("# Vault\n\n");
    md.push_str("These environment variables are injected automatically into ");
    md.push_str("every terminal Agent Console spawns. Use them in shell commands ");
    md.push_str("with `$KEY` instead of asking the user for the value.\n\n");
    if store.entries.is_empty() {
        md.push_str("_(no entries)_\n");
    } else {
        md.push_str("| Key | Description | Kind |\n");
        md.push_str("|-----|-------------|------|\n");
        for e in &store.entries {
            let kind = if e.secret { "secret" } else { "config" };
            let desc = e.description.replace('|', "\\|");
            md.push_str(&format!("| `${}` | {} | {} |\n", e.key, desc, kind));
        }
    }
    fs::write(&path, md)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fresh project root per test. Keys use an AC_TEST_ prefix so the
    /// best-effort keyring cleanup in upsert/delete can never collide with a
    /// real credential on a developer machine.
    fn temp_root(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ac-vault-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn project_entries(root: &Path) -> Vec<VaultEntryView> {
        list(Some(root))
            .unwrap()
            .into_iter()
            .filter(|v| v.scope == Scope::Project)
            .collect()
    }

    #[test]
    fn key_validation_rejects_bad_names() {
        for bad in ["", "1KEY", "has-dash", "has space", "KÉ", "a.b"] {
            assert!(validate_key(bad).is_err(), "{bad:?} should be rejected");
        }
        for good in ["_PRIVATE", "DB_URL", "a1"] {
            assert!(validate_key(good).is_ok(), "{good:?} should be accepted");
        }
    }

    #[test]
    fn plain_entry_roundtrip_and_vault_md_never_leaks_values() {
        let root = temp_root("plain");
        upsert(
            Some(&root),
            Scope::Project,
            "AC_TEST_DB_URL".into(),
            "Staging database".into(),
            false,
            Some("postgres://leak-canary".into()),
        )
        .unwrap();

        let mine = project_entries(&root);
        assert_eq!(mine.len(), 1);
        assert!(mine[0].has_value);
        assert!(!mine[0].secret);
        assert_eq!(
            get_value(Some(&root), Scope::Project, "AC_TEST_DB_URL").unwrap(),
            "postgres://leak-canary"
        );

        // The agent-facing index lists keys and descriptions — never values.
        let md = fs::read_to_string(root.join(".claude").join(VAULT_MD)).unwrap();
        assert!(md.contains("$AC_TEST_DB_URL"));
        assert!(md.contains("Staging database"));
        assert!(!md.contains("leak-canary"), "VAULT.md leaked a value:\n{md}");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn update_preserves_created_at_and_keeps_old_value_when_none_given() {
        let root = temp_root("update");
        let first = upsert(
            Some(&root),
            Scope::Project,
            "AC_TEST_KEY".into(),
            "v1".into(),
            false,
            Some("original".into()),
        )
        .unwrap();
        let second = upsert(
            Some(&root),
            Scope::Project,
            "AC_TEST_KEY".into(),
            "v2".into(),
            false,
            None,
        )
        .unwrap();

        assert_eq!(second.created_at_ms, first.created_at_ms);
        assert!(second.updated_at_ms >= first.updated_at_ms);
        assert_eq!(second.description, "v2");
        // Metadata-only update must not wipe the stored value.
        assert_eq!(
            get_value(Some(&root), Scope::Project, "AC_TEST_KEY").unwrap(),
            "original"
        );
        assert_eq!(project_entries(&root).len(), 1, "update must not duplicate");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn delete_removes_entry_and_is_idempotent() {
        let root = temp_root("delete");
        upsert(
            Some(&root),
            Scope::Project,
            "AC_TEST_GONE".into(),
            "bye".into(),
            false,
            Some("x".into()),
        )
        .unwrap();

        delete(Some(&root), Scope::Project, "AC_TEST_GONE").unwrap();
        assert!(project_entries(&root).is_empty());
        let md = fs::read_to_string(root.join(".claude").join(VAULT_MD)).unwrap();
        assert!(md.contains("no entries"));

        // Deleting again (or a key that never existed) is a quiet no-op.
        delete(Some(&root), Scope::Project, "AC_TEST_GONE").unwrap();
        delete(Some(&root), Scope::Project, "AC_TEST_NEVER").unwrap();

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn get_value_unknown_key_is_not_found() {
        let root = temp_root("notfound");
        let err = get_value(Some(&root), Scope::Project, "AC_TEST_MISSING").unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn secret_create_without_value_is_rejected_before_anything_persists() {
        let root = temp_root("secreterr");
        let err = upsert(
            Some(&root),
            Scope::Project,
            "AC_TEST_SECRET".into(),
            "needs a value".into(),
            true,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, AppError::InvalidArgument(_)));
        assert!(project_entries(&root).is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn project_scope_without_project_is_rejected() {
        let err = upsert(
            None,
            Scope::Project,
            "AC_TEST_NOPROJ".into(),
            String::new(),
            false,
            Some("x".into()),
        )
        .unwrap_err();
        assert!(matches!(err, AppError::InvalidArgument(_)));
    }

    /// One test fn — it mutates the process-global HOME so the Global scope
    /// lands in a temp dir instead of the developer's real ~/.claude.
    #[test]
    fn env_for_spawn_project_overrides_global() {
        let _env = crate::test_support::lock_env();
        let fake_home = temp_root("home");
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &fake_home);

        let run = || -> AppResult<()> {
            let root = temp_root("envspawn");
            upsert(
                None,
                Scope::Global,
                "AC_TEST_SHARED".into(),
                String::new(),
                false,
                Some("from-global".into()),
            )?;
            upsert(
                None,
                Scope::Global,
                "AC_TEST_GLOBAL_ONLY".into(),
                String::new(),
                false,
                Some("global-only".into()),
            )?;
            upsert(
                Some(&root),
                Scope::Project,
                "AC_TEST_SHARED".into(),
                String::new(),
                false,
                Some("from-project".into()),
            )?;

            let env = env_for_spawn(Some(&root))?;
            assert_eq!(
                env.get("AC_TEST_SHARED").map(String::as_str),
                Some("from-project"),
                "project entry must win the key collision"
            );
            assert_eq!(
                env.get("AC_TEST_GLOBAL_ONLY").map(String::as_str),
                Some("global-only")
            );
            fs::remove_dir_all(&root).ok();
            Ok(())
        };
        let result = run();

        match prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        fs::remove_dir_all(&fake_home).ok();
        result.unwrap();
    }
}
