//! Paired-device store + human-approval gate + revocation (Hito 2 of the
//! secure-pairing MVP).
//!
//! After the Noise handshake (secure_channel) proves *who* a device is, this is
//! where we decide whether it's *allowed* and what it may do:
//! - A completed handshake only ever produces a **pending** pairing. It does not
//!   become a trusted device until a human explicitly approves it on the desktop
//!   — the final safety net against a raced/photographed QR.
//! - Trusted devices are persisted (atomic, crash-safe) with a pinned public key
//!   and a **scope**. Revocation = forgetting the pinned key.
//!
//! Scope is deliberately conservative: a fresh device gets read + small
//! approvals, and **dangerous actions are never granted by scope alone** — they
//! always require an on-device second factor (enforced at the action layer).
//!
//! Hito 2: the service + its tests stand alone; the production callers (commands,
//! the desktop approval dialog, the auth check on an incoming connection) arrive
//! in Hito 3, so dead-code is silenced until then.
#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// What a paired phone is allowed to do. Ordered least → most privilege.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceScope {
    /// Read status / receive notifications only.
    ReadOnly,
    /// Read + approve small, reversible actions. The default for a new device.
    ReadApprovals,
    /// May also inject prompts (drive the agent). Dangerous actions STILL need a
    /// second factor.
    Full,
}

impl DeviceScope {
    /// May this device send prompts that make the agent act?
    pub fn can_inject(self) -> bool {
        matches!(self, DeviceScope::Full)
    }

    /// May this device approve a small / reversible permission gate?
    pub fn can_approve_small(self) -> bool {
        matches!(self, DeviceScope::ReadApprovals | DeviceScope::Full)
    }

    /// Dangerous = prod / secrets / destructive. Never granted by scope alone —
    /// always requires an on-device second factor. Encoded as a hard `false` so
    /// no call site can accidentally authorize it from scope.
    pub fn can_approve_dangerous(self) -> bool {
        false
    }
}

/// A trusted, paired device.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedDevice {
    pub id: String,
    pub label: String,
    /// Pinned static public key (base64url) — this device's cryptographic identity.
    pub public_key: String,
    pub scope: DeviceScope,
    pub paired_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_ms: Option<i64>,
}

/// A handshake that completed but is awaiting the human's explicit approval on
/// the desktop. Ephemeral — never persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPairing {
    pub id: String,
    pub label: String,
    pub public_key: String,
    pub proposed_scope: DeviceScope,
    pub created_ms: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DevicesFile {
    #[serde(default)]
    devices: Vec<PairedDevice>,
}

pub struct PairedDevicesService {
    file_lock: Mutex<()>,
    pending: Mutex<Vec<PendingPairing>>,
}

impl PairedDevicesService {
    pub fn new() -> Self {
        Self {
            file_lock: Mutex::new(()),
            pending: Mutex::new(Vec::new()),
        }
    }

    fn dir() -> AppResult<PathBuf> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| AppError::Other("no data_local dir".into()))?
            .join("agent-console");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("paired-devices.json"))
    }
    fn bak_path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("paired-devices.json.bak"))
    }
    fn tmp_path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("paired-devices.json.tmp"))
    }

    /// A missing/empty file is a legitimate empty state; a parse failure on an
    /// existing file is an error (so we never mistake unreadable trust data for
    /// "no devices" and silently grant nothing / overwrite it), recovering from
    /// the `.bak` first.
    fn load_file() -> AppResult<DevicesFile> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(DevicesFile::default());
        }
        let txt = fs::read_to_string(&path)
            .map_err(|e| AppError::Other(format!("read paired-devices.json: {e}")))?;
        if txt.trim().is_empty() {
            return Ok(DevicesFile::default());
        }
        match serde_json::from_str::<DevicesFile>(&txt) {
            Ok(f) => Ok(f),
            Err(e) => {
                if let Ok(bak) = Self::bak_path() {
                    if let Ok(btxt) = fs::read_to_string(&bak) {
                        if let Ok(f) = serde_json::from_str::<DevicesFile>(&btxt) {
                            return Ok(f);
                        }
                    }
                }
                Err(AppError::Other(format!("parse paired-devices.json: {e}")))
            }
        }
    }

    /// Atomic write: temp file → back up current good file → rename over target.
    fn write_file(file: &DevicesFile) -> AppResult<()> {
        let path = Self::path()?;
        let json = serde_json::to_string_pretty(file)
            .map_err(|e| AppError::Other(format!("serialize: {e}")))?;
        let tmp = Self::tmp_path()?;
        fs::write(&tmp, json.as_bytes())?;
        if path.exists() {
            if let Ok(bak) = Self::bak_path() {
                let _ = fs::copy(&path, &bak);
            }
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    // ---- Trusted device store ----

    pub fn list_devices(&self) -> AppResult<Vec<PairedDevice>> {
        let _g = self.file_lock.lock().unwrap();
        Ok(Self::load_file()?.devices)
    }

    /// Look up a device by its pinned key — the authn entry point for an
    /// incoming connection (Hito 3+).
    pub fn find_by_key(&self, public_key: &str) -> AppResult<Option<PairedDevice>> {
        let _g = self.file_lock.lock().unwrap();
        Ok(Self::load_file()?
            .devices
            .into_iter()
            .find(|d| d.public_key == public_key))
    }

    /// Revoke = forget the pinned key. A future handshake from it matches no
    /// device and is rejected at the auth layer.
    pub fn revoke(&self, id: &str) -> AppResult<()> {
        let _g = self.file_lock.lock().unwrap();
        let mut file = Self::load_file()?;
        file.devices.retain(|d| d.id != id);
        Self::write_file(&file)
    }

    pub fn set_scope(&self, id: &str, scope: DeviceScope) -> AppResult<()> {
        let _g = self.file_lock.lock().unwrap();
        let mut file = Self::load_file()?;
        let dev = file
            .devices
            .iter_mut()
            .find(|d| d.id == id)
            .ok_or_else(|| AppError::NotFound(format!("device {id}")))?;
        dev.scope = scope;
        Self::write_file(&file)
    }

    pub fn touch(&self, id: &str, now_ms: i64) -> AppResult<()> {
        let _g = self.file_lock.lock().unwrap();
        let mut file = Self::load_file()?;
        if let Some(dev) = file.devices.iter_mut().find(|d| d.id == id) {
            dev.last_seen_ms = Some(now_ms);
            Self::write_file(&file)?;
        }
        Ok(())
    }

    // ---- Human-approval gate ----

    /// Register a completed handshake as awaiting desktop approval. Returns the
    /// pending id. Does NOT trust the device yet.
    pub fn propose_pairing(
        &self,
        label: &str,
        public_key: &str,
        proposed_scope: DeviceScope,
        now_ms: i64,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let mut pending = self.pending.lock().unwrap();
        pending.push(PendingPairing {
            id: id.clone(),
            label: label.to_string(),
            public_key: public_key.to_string(),
            proposed_scope,
            created_ms: now_ms,
        });
        id
    }

    pub fn pending(&self) -> Vec<PendingPairing> {
        self.pending.lock().unwrap().clone()
    }

    /// Human approved on the desktop: move the pending pairing into the trusted
    /// store and persist it. Re-pairing an existing key replaces that record
    /// (rotation) rather than creating a duplicate.
    pub fn approve_pairing(&self, pending_id: &str, now_ms: i64) -> AppResult<PairedDevice> {
        let proposal = {
            let mut pending = self.pending.lock().unwrap();
            let idx = pending
                .iter()
                .position(|p| p.id == pending_id)
                .ok_or_else(|| AppError::NotFound(format!("pending pairing {pending_id}")))?;
            pending.remove(idx)
        };

        let _g = self.file_lock.lock().unwrap();
        let mut file = Self::load_file()?;
        // Replace any existing device sharing this key (re-pair / key rotation).
        file.devices.retain(|d| d.public_key != proposal.public_key);
        let device = PairedDevice {
            id: uuid::Uuid::new_v4().to_string(),
            label: proposal.label,
            public_key: proposal.public_key,
            scope: proposal.proposed_scope,
            paired_at_ms: now_ms,
            last_seen_ms: None,
        };
        file.devices.push(device.clone());
        Self::write_file(&file)?;
        Ok(device)
    }

    /// Human rejected (or the QR was raced by an attacker): drop the pending
    /// pairing without trusting anything.
    pub fn reject_pairing(&self, pending_id: &str) {
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|p| p.id != pending_id);
    }
}

impl Default for PairedDevicesService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn isolated_data_dir() {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir()
            .join(format!("ac-devices-test-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);
    }

    #[test]
    fn scope_semantics() {
        assert!(!DeviceScope::ReadOnly.can_inject());
        assert!(!DeviceScope::ReadOnly.can_approve_small());
        assert!(DeviceScope::ReadApprovals.can_approve_small());
        assert!(!DeviceScope::ReadApprovals.can_inject());
        assert!(DeviceScope::Full.can_inject());
        // Dangerous is never granted by scope alone — at any level.
        for s in [DeviceScope::ReadOnly, DeviceScope::ReadApprovals, DeviceScope::Full] {
            assert!(!s.can_approve_dangerous(), "{s:?} must not self-grant dangerous");
        }
    }

    /// One test fn (mutates the process-global XDG_DATA_HOME) covering the
    /// approval gate, persistence, scope changes and revocation end to end.
    #[test]
    fn approval_gate_store_and_revocation() {
        let _env = crate::test_support::lock_env();
        isolated_data_dir();
        let svc = PairedDevicesService::new();
        let now = 1_700_000_000_000;

        // Fresh: empty, not an error.
        assert!(svc.list_devices().unwrap().is_empty());

        // 1. A completed handshake only proposes — it does NOT grant trust.
        let pid = svc.propose_pairing("Carlos iPhone", "PUBKEY_A", DeviceScope::ReadApprovals, now);
        assert_eq!(svc.pending().len(), 1);
        assert!(svc.list_devices().unwrap().is_empty(), "not trusted until approved");
        assert!(svc.find_by_key("PUBKEY_A").unwrap().is_none());

        // 2. Reject drops it without trusting anything.
        let pid2 = svc.propose_pairing("Rogue", "PUBKEY_EVIL", DeviceScope::Full, now);
        svc.reject_pairing(&pid2);
        assert!(svc.find_by_key("PUBKEY_EVIL").unwrap().is_none());
        assert_eq!(svc.pending().len(), 1, "only the rejected one is gone");

        // 3. Human approval commits + persists the device.
        let dev = svc.approve_pairing(&pid, now).unwrap();
        assert_eq!(dev.scope, DeviceScope::ReadApprovals);
        assert!(svc.pending().is_empty(), "approved pairing leaves the queue");
        assert_eq!(svc.find_by_key("PUBKEY_A").unwrap().unwrap().id, dev.id);

        // Persisted across a fresh service instance.
        let svc2 = PairedDevicesService::new();
        assert_eq!(svc2.list_devices().unwrap().len(), 1);

        // 4. Elevate scope (e.g. to let it drive the agent).
        svc.set_scope(&dev.id, DeviceScope::Full).unwrap();
        assert!(svc.find_by_key("PUBKEY_A").unwrap().unwrap().scope.can_inject());

        // 5. Re-pairing the same key replaces, never duplicates.
        let pid3 = svc.propose_pairing("Carlos iPhone (re)", "PUBKEY_A", DeviceScope::ReadOnly, now + 1);
        svc.approve_pairing(&pid3, now + 1).unwrap();
        let all = svc.list_devices().unwrap();
        assert_eq!(all.len(), 1, "same key must not create a second device");
        assert_eq!(all[0].scope, DeviceScope::ReadOnly);

        // 6. Revoke = forget the key.
        let id = all[0].id.clone();
        svc.revoke(&id).unwrap();
        assert!(svc.list_devices().unwrap().is_empty());
        assert!(svc.find_by_key("PUBKEY_A").unwrap().is_none(), "revoked key matches nothing");
    }
}
