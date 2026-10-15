//! D11: a sponsor per product. The human who answers for a project states
//! the objective, a weekly budget, the risk threshold up to which the
//! console may decide alone, and whether it may. The policy is persisted
//! per project and every change is witnessed (`sponsor_set`); every
//! decision the policy takes is an `approval_decision` under the actor
//! `sponsor`, with the policy named — the human only decides what crosses
//! the threshold, and the ledger shows which ones they did not.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

pub const RISK_LEVELS: &[&str] = &["low", "medium", "high"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Sponsor {
    /// What this product is trying to achieve this week — the agents read it.
    pub objective: String,
    /// Who answers for it (a name; the ledger actor stays `sponsor`).
    pub owner: String,
    /// 0 = no budget declared.
    pub weekly_budget_usd: f64,
    /// Decisions up to this tier may be taken by the policy: low | medium | high.
    pub risk_threshold: String,
    /// Master switch for the policy.
    pub auto_approve: bool,
}

impl Default for Sponsor {
    fn default() -> Self {
        Sponsor {
            objective: String::new(),
            owner: String::new(),
            weekly_budget_usd: 0.0,
            risk_threshold: "low".into(),
            auto_approve: false,
        }
    }
}

impl Sponsor {
    pub fn validate(mut self) -> AppResult<Self> {
        if !RISK_LEVELS.contains(&self.risk_threshold.as_str()) {
            return Err(AppError::Other(format!(
                "riskThreshold must be one of {}",
                RISK_LEVELS.join("|")
            )));
        }
        if !self.weekly_budget_usd.is_finite() || self.weekly_budget_usd < 0.0 {
            return Err(AppError::Other("weeklyBudgetUsd must be ≥ 0".into()));
        }
        self.objective = self.objective.trim().chars().take(2000).collect();
        self.owner = self.owner.trim().chars().take(120).collect();
        Ok(self)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SponsorFile {
    #[serde(default)]
    by_project: HashMap<String, Sponsor>,
}

#[derive(Default)]
pub struct SponsorService {
    inner: Mutex<Option<HashMap<String, Sponsor>>>,
}

impl SponsorService {
    pub fn new() -> Self {
        Self::default()
    }

    fn path() -> AppResult<PathBuf> {
        Ok(dirs::data_local_dir()
            .ok_or_else(|| AppError::Other("no data_local dir".into()))?
            .join("agent-console")
            .join("sponsors.json"))
    }

    fn load() -> HashMap<String, Sponsor> {
        let Ok(path) = Self::path() else {
            return HashMap::new();
        };
        let read = |p: &Path| -> Option<SponsorFile> {
            serde_json::from_str(&fs::read_to_string(p).ok()?).ok()
        };
        read(&path)
            .or_else(|| read(&path.with_extension("json.bak")))
            .unwrap_or_default()
            .by_project
    }

    fn map(inner: &mut Option<HashMap<String, Sponsor>>) -> &mut HashMap<String, Sponsor> {
        inner.get_or_insert_with(Self::load)
    }

    pub fn get(&self, project_root: &str) -> Sponsor {
        let mut inner = self.inner.lock();
        Self::map(&mut inner)
            .get(project_root)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set(&self, project_root: &str, sponsor: Sponsor) -> AppResult<Sponsor> {
        let sponsor = sponsor.validate()?;
        let mut inner = self.inner.lock();
        let map = Self::map(&mut inner);
        map.insert(project_root.to_string(), sponsor.clone());
        let file = SponsorFile {
            by_project: map.clone(),
        };
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(&file)
            .map_err(|e| AppError::Other(format!("serialize sponsors: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        if path.exists() {
            let _ = fs::copy(&path, path.with_extension("json.bak"));
        }
        fs::write(&tmp, body)?;
        fs::rename(&tmp, &path)?;
        Ok(sponsor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_round_trips_per_project() {
        let _env = crate::test_support::lock_env();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-sponsor-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let svc = SponsorService::new();
        assert_eq!(svc.get("/p"), Sponsor::default());
        let s = svc
            .set(
                "/p",
                Sponsor {
                    objective: "  ship billing v2 ".into(),
                    owner: "Carlos".into(),
                    weekly_budget_usd: 40.0,
                    risk_threshold: "low".into(),
                    auto_approve: true,
                },
            )
            .unwrap();
        assert_eq!(s.objective, "ship billing v2");
        assert_eq!(SponsorService::new().get("/p"), s, "persisted on disk");
        assert!(svc
            .set(
                "/p",
                Sponsor {
                    risk_threshold: "yolo".into(),
                    ..Default::default()
                }
            )
            .is_err());
        assert!(svc
            .set(
                "/p",
                Sponsor {
                    weekly_budget_usd: -1.0,
                    ..Default::default()
                }
            )
            .is_err());
        std::fs::remove_dir_all(&base).ok();
    }
}
