use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Scope { Project, Global }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Effect { Allow, Deny, Ask }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredRule {
    pub scope: Scope,
    pub effect: Effect,
    pub raw: String,
    pub source: String,            // "agent-console" | "external"
    pub created_at_ms: Option<u64>,
    pub settings_path: String,     // where it lives (so the UI can show provenance)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsSnapshot {
    pub rules: Vec<StoredRule>,
    pub project_settings_path: Option<String>,
    pub global_settings_path: String,
}

pub fn snapshot(project_root: Option<&Path>) -> AppResult<PermissionsSnapshot> {
    let global = global_settings_path();
    let mut rules: Vec<StoredRule> = Vec::new();

    let project_settings = project_root.map(|r| r.join(".claude/settings.json"));
    if let Some(p) = &project_settings {
        let sidecar = sidecar_path_for(p);
        rules.extend(read_rules_from(p, Scope::Project, &sidecar)?);
    }
    let global_sidecar = sidecar_path_for(&global);
    rules.extend(read_rules_from(&global, Scope::Global, &global_sidecar)?);

    Ok(PermissionsSnapshot {
        rules,
        project_settings_path: project_settings.map(|p| p.to_string_lossy().to_string()),
        global_settings_path: global.to_string_lossy().to_string(),
    })
}

pub fn add_rule(
    project_root: Option<&Path>,
    scope: Scope,
    effect: Effect,
    raw: &str,
) -> AppResult<StoredRule> {
    let path = settings_path_for(scope, project_root)?;
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }

    let mut settings = read_settings(&path)?;
    let key = effect_key(effect);

    let perms = settings.get("permissions").cloned().unwrap_or(json!({}));
    let mut perms = if perms.is_object() { perms } else { json!({}) };
    let arr = perms.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let already = arr.iter().any(|v| v.as_str() == Some(raw));
    let mut new_arr = arr;
    if !already { new_arr.push(Value::String(raw.to_string())); }
    perms.as_object_mut().unwrap().insert(key.to_string(), Value::Array(new_arr));
    settings.as_object_mut().unwrap().insert("permissions".to_string(), perms);

    backup(&path)?;
    write_settings(&path, &settings)?;

    let created_at_ms = now_ms();
    let sidecar_path = sidecar_path_for(&path);
    write_sidecar_entry(&sidecar_path, raw, created_at_ms)?;

    Ok(StoredRule {
        scope,
        effect,
        raw: raw.to_string(),
        source: "agent-console".to_string(),
        created_at_ms: Some(created_at_ms),
        settings_path: path.to_string_lossy().to_string(),
    })
}

pub fn remove_rule(
    project_root: Option<&Path>,
    scope: Scope,
    effect: Effect,
    raw: &str,
) -> AppResult<()> {
    let path = settings_path_for(scope, project_root)?;
    if !path.exists() { return Ok(()); }
    let mut settings = read_settings(&path)?;
    let key = effect_key(effect);
    if let Some(arr) = settings.pointer_mut(&format!("/permissions/{key}"))
        .and_then(|v| v.as_array_mut())
    {
        arr.retain(|v| v.as_str() != Some(raw));
    }
    backup(&path)?;
    write_settings(&path, &settings)?;

    let sidecar_path = sidecar_path_for(&path);
    remove_sidecar_entry(&sidecar_path, raw)?;
    Ok(())
}

// --- helpers -----------------------------------------------------------------

fn effect_key(e: Effect) -> &'static str {
    match e { Effect::Allow => "allow", Effect::Deny => "deny", Effect::Ask => "ask" }
}

fn settings_path_for(scope: Scope, project_root: Option<&Path>) -> AppResult<PathBuf> {
    match scope {
        Scope::Project => project_root
            .map(|r| r.join(".claude/settings.json"))
            .ok_or_else(|| AppError::InvalidArgument("no project open".into())),
        Scope::Global => Ok(global_settings_path()),
    }
}

fn global_settings_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".claude/settings.json"))
        .unwrap_or_else(|| PathBuf::from(".claude/settings.json"))
}

fn read_settings(path: &Path) -> AppResult<Value> {
    if !path.exists() { return Ok(json!({})); }
    let txt = fs::read_to_string(path)?;
    let v: Value = serde_json::from_str(&txt).unwrap_or(json!({}));
    Ok(if v.is_object() { v } else { json!({}) })
}

fn write_settings(path: &Path, v: &Value) -> AppResult<()> {
    fs::write(path, serde_json::to_string_pretty(v).unwrap())?;
    Ok(())
}

fn backup(path: &Path) -> AppResult<()> {
    if !path.exists() { return Ok(()); }
    let bak = path.with_extension(format!("json.{}.bak", now_ms()));
    fs::copy(path, &bak)?;
    Ok(())
}

fn read_rules_from(path: &Path, scope: Scope, sidecar: &Path) -> AppResult<Vec<StoredRule>> {
    if !path.exists() { return Ok(Vec::new()); }
    let settings = read_settings(path)?;
    let sidecar_map = read_sidecar(sidecar);
    let mut out = Vec::new();
    for (key, effect) in [("allow", Effect::Allow), ("deny", Effect::Deny), ("ask", Effect::Ask)] {
        let Some(arr) = settings.pointer(&format!("/permissions/{key}")).and_then(|v| v.as_array()) else { continue };
        for v in arr {
            let Some(raw) = v.as_str() else { continue };
            let meta = sidecar_map.get(raw).cloned();
            out.push(StoredRule {
                scope,
                effect,
                raw: raw.to_string(),
                source: if meta.is_some() { "agent-console".into() } else { "external".into() },
                created_at_ms: meta,
                settings_path: path.to_string_lossy().to_string(),
            });
        }
    }
    Ok(out)
}

fn sidecar_path_for(settings_path: &Path) -> PathBuf {
    settings_path.with_file_name("agent-console-rules.json")
}

fn read_sidecar(path: &Path) -> std::collections::HashMap<String, u64> {
    let Ok(txt) = fs::read_to_string(path) else { return Default::default(); };
    let Ok(v) = serde_json::from_str::<Value>(&txt) else { return Default::default(); };
    let Some(obj) = v.as_object() else { return Default::default(); };
    obj.iter().filter_map(|(k, v)| v.as_u64().map(|n| (k.clone(), n))).collect()
}

fn write_sidecar_entry(path: &Path, raw: &str, ts_ms: u64) -> AppResult<()> {
    let mut obj = match fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    obj.insert(raw.to_string(), json!(ts_ms));
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    fs::write(path, serde_json::to_string_pretty(&Value::Object(obj)).unwrap())?;
    Ok(())
}

fn remove_sidecar_entry(path: &Path, raw: &str) -> AppResult<()> {
    if !path.exists() { return Ok(()); }
    let Ok(txt) = fs::read_to_string(path) else { return Ok(()); };
    let mut obj = match serde_json::from_str::<Value>(&txt) {
        Ok(Value::Object(m)) => m,
        _ => return Ok(()),
    };
    obj.remove(raw);
    fs::write(path, serde_json::to_string_pretty(&Value::Object(obj)).unwrap())?;
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
