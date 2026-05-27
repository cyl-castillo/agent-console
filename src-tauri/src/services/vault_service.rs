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
    out.sort_by(|a, b| a.key.to_lowercase().cmp(&b.key.to_lowercase()));
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
            keyring_entry(&scope, &key)?.set_password(v)
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
        value.clone().or_else(|| existing.and_then(|i| store.entries[i].value.clone()))
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
    let entry = store.entries.iter().find(|e| e.key == key)
        .ok_or_else(|| AppError::NotFound(format!("vault entry '{key}'")))?;
    if entry.secret {
        keyring_entry(&scope, key)?.get_password()
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
        keyring_entry(&scope, &e.key)?.get_password()
            .map_err(|e| AppError::Other(format!("keyring get: {e}")))
    } else {
        Ok(e.value.clone().unwrap_or_default())
    }
}

fn store_path(project_root: Option<&Path>, scope: &Scope) -> AppResult<PathBuf> {
    match scope {
        Scope::Project => {
            let root = project_root
                .ok_or_else(|| AppError::InvalidArgument("project scope requires an open project".into()))?;
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
    let store: StoreFile = serde_json::from_str(&raw)
        .map_err(|e| AppError::Other(format!("vault parse: {e}")))?;
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
    if !key.chars().next().unwrap().is_ascii_alphabetic() && key.chars().next().unwrap() != '_' {
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
