//! D14: identity and perimeter per agent. Every agent the console runs has
//! an `agent_id` of its own (never the human's), a principal it acts on
//! behalf of (explicit, not inherited), a place it runs in and a network
//! perimeter. Session agents get a signed Agent Card (A2A-shaped) under the
//! console's ed25519 key — the same key that signs proof packets, so one
//! key id pins both "who witnessed" and "who was running". The card and the
//! identities of reviewers and MCP callers are witnessed in the ledger as
//! `agent_identity`, once per project.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

pub const CARD_TYPE: &str = "https://agent-console.dev/agent-card/v0.1";
pub const RUNS_IN: &[&str] = &["local", "worktree", "microvm", "cloud"];

/// A2A-shaped agent card: who this agent is, for whom, where, within what.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    #[serde(rename = "type")]
    pub card_type: String,
    /// `urn:agent-console:agent:<uuid>` for sessions; stable names for the
    /// reviewer (`urn:agent-console:agent:reviewer:<engine>`) and MCP callers
    /// (`urn:agent-console:agent:mcp:<caller>`).
    pub agent_id: String,
    pub name: String,
    /// session | reviewer | mcp
    pub kind: String,
    pub engine: Option<String>,
    pub model: Option<String>,
    /// On whose behalf: explicit, so sub-agents and reviewers never inherit
    /// the human silently.
    pub principal: Principal,
    /// local | worktree | microvm | cloud
    pub runs_in: String,
    pub cwd: Option<String>,
    /// Hosts the session may reach (exact or `.suffix`); empty = no
    /// perimeter declared.
    pub allowed_hosts: Vec<String>,
    pub budget_usd: Option<f64>,
    pub created_at_ms: i64,
    /// The console that minted the card.
    pub issuer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Principal {
    pub principal_type: String,
    pub principal_id: String,
}

impl Principal {
    pub fn local_human() -> Self {
        Principal {
            principal_type: "human".into(),
            principal_id: "urn:agent-console:user:local".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SignedAgentCard {
    pub card: AgentCard,
    /// ed25519 over the RFC 8785 canonical JSON of `card`, base64.
    pub signature: String,
    pub key_id: String,
    pub public_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionIdentityInput {
    pub term_id: String,
    pub agent_id: String,
    pub name: String,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub runs_in: Option<String>,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    pub budget_usd: Option<f64>,
}

pub fn runs_in_or_default(v: Option<&str>) -> AppResult<String> {
    match v {
        None => Ok("local".into()),
        Some(s) if RUNS_IN.contains(&s) => Ok(s.to_string()),
        Some(s) => Err(AppError::Other(format!(
            "runsIn must be one of {}, not {s}",
            RUNS_IN.join("|")
        ))),
    }
}

/// Normalise a host list: lowercase, trimmed, no scheme/port, deduped.
pub fn normalize_hosts(hosts: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for h in hosts {
        let mut s = h.trim().to_ascii_lowercase();
        if let Some(i) = s.find("://") {
            s = s[i + 3..].to_string();
        }
        if let Some(i) = s.find('/') {
            s.truncate(i);
        }
        if let Some(i) = s.rfind(':') {
            if s[i + 1..].chars().all(|c| c.is_ascii_digit()) {
                s.truncate(i);
            }
        }
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Is `host` inside a declared perimeter? Reference semantics the webview's
/// `lib/perimeter.ts` mirrors (the approval gate runs there); the tests
/// below pin them.
#[allow(dead_code)]
/// Exact match, or a `.suffix`
/// entry (`.github.com` covers `api.github.com`; `github.com` covers only
/// itself and `www`-less exact). An empty perimeter allows nothing here —
/// callers treat "no perimeter declared" before asking.
pub fn host_allowed(host: &str, allowed: &[String]) -> bool {
    let h = host.trim().to_ascii_lowercase();
    allowed.iter().any(|a| {
        let a = a.trim().to_ascii_lowercase();
        if let Some(suffix) = a.strip_prefix('.') {
            h == suffix || h.ends_with(&format!(".{suffix}"))
        } else {
            h == a
        }
    })
}

pub fn card_for_session(
    input: &SessionIdentityInput,
    now_ms: i64,
    issuer: &str,
) -> AppResult<AgentCard> {
    if !input.agent_id.starts_with("urn:agent-console:agent:") {
        return Err(AppError::Other(
            "agentId must be a urn:agent-console:agent: id".into(),
        ));
    }
    Ok(AgentCard {
        card_type: CARD_TYPE.into(),
        agent_id: input.agent_id.clone(),
        name: input.name.clone(),
        kind: "session".into(),
        engine: input.agent.clone(),
        model: input.model.clone(),
        principal: Principal::local_human(),
        runs_in: runs_in_or_default(input.runs_in.as_deref())?,
        cwd: input.cwd.clone(),
        allowed_hosts: normalize_hosts(&input.allowed_hosts),
        budget_usd: input.budget_usd,
        created_at_ms: now_ms,
        issuer: issuer.to_string(),
    })
}

pub fn card_for_reviewer(
    engine: &str,
    model: Option<&str>,
    now_ms: i64,
    issuer: &str,
) -> AgentCard {
    AgentCard {
        card_type: CARD_TYPE.into(),
        agent_id: format!("urn:agent-console:agent:reviewer:{engine}"),
        name: format!("reviewer:{engine}"),
        kind: "reviewer".into(),
        engine: Some(engine.into()),
        model: model.map(String::from),
        principal: Principal::local_human(),
        runs_in: "local".into(),
        cwd: None,
        allowed_hosts: Vec::new(),
        budget_usd: None,
        created_at_ms: now_ms,
        issuer: issuer.to_string(),
    }
}

pub fn card_for_mcp_caller(caller: &str, now_ms: i64, issuer: &str) -> AgentCard {
    AgentCard {
        card_type: CARD_TYPE.into(),
        agent_id: format!("urn:agent-console:agent:mcp:{caller}"),
        name: caller.to_string(),
        kind: "mcp".into(),
        engine: None,
        model: None,
        principal: Principal::local_human(),
        runs_in: "local".into(),
        cwd: None,
        allowed_hosts: Vec::new(),
        budget_usd: None,
        created_at_ms: now_ms,
        issuer: issuer.to_string(),
    }
}

fn canonical(card: &AgentCard) -> AppResult<Vec<u8>> {
    serde_jcs::to_vec(card).map_err(|e| AppError::Other(format!("canonicalize card: {e}")))
}

fn key_id(vk: &VerifyingKey) -> String {
    let d = Sha256::digest(vk.as_bytes());
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Sign with an injected seed (tests, headless).
pub fn sign_with_seed(card: AgentCard, seed: &[u8; 32]) -> AppResult<SignedAgentCard> {
    let sk = SigningKey::from_bytes(seed);
    let vk = sk.verifying_key();
    let sig = sk.sign(&canonical(&card)?);
    Ok(SignedAgentCard {
        card,
        signature: B64.encode(sig.to_bytes()),
        key_id: key_id(&vk),
        public_key: B64.encode(vk.as_bytes()),
    })
}

/// Sign with the console's key (the proof-packet key in the OS keychain).
pub fn sign(card: AgentCard) -> AppResult<SignedAgentCard> {
    let seed = crate::services::testigo_export::load_or_create_seed()?;
    sign_with_seed(card, &seed)
}

pub fn verify(signed: &SignedAgentCard) -> AppResult<bool> {
    let pk = B64
        .decode(&signed.public_key)
        .map_err(|e| AppError::Other(format!("public key: {e}")))?;
    let vk = VerifyingKey::from_bytes(
        &pk.try_into()
            .map_err(|_| AppError::Other("public key length".into()))?,
    )
    .map_err(|e| AppError::Other(format!("public key: {e}")))?;
    if key_id(&vk) != signed.key_id {
        return Ok(false);
    }
    let sig = B64
        .decode(&signed.signature)
        .map_err(|e| AppError::Other(format!("signature: {e}")))?;
    let sig = Signature::from_bytes(
        &sig.try_into()
            .map_err(|_| AppError::Other("signature length".into()))?,
    );
    Ok(vk.verify(&canonical(&signed.card)?, &sig).is_ok())
}

/// What the ledger keeps for an identity: the card plus its signature when
/// it has one (session cards) — compact, and verifiable against the
/// published key.
pub fn ledger_payload(card: &AgentCard, signed: Option<&SignedAgentCard>) -> Value {
    let mut v = json!({ "card": card });
    if let Some(s) = signed {
        v["signature"] = json!(s.signature);
        v["keyId"] = json!(s.key_id);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> SessionIdentityInput {
        SessionIdentityInput {
            term_id: "t1".into(),
            agent_id: "urn:agent-console:agent:00000000-0000-4000-8000-000000000001".into(),
            name: "fix auth".into(),
            agent: Some("claude".into()),
            model: Some("opus".into()),
            cwd: Some("/p".into()),
            runs_in: Some("worktree".into()),
            allowed_hosts: vec![
                "https://api.GitHub.com/v3".into(),
                ".npmjs.org".into(),
                "github.com".into(),
                "github.com".into(),
            ],
            budget_usd: Some(10.0),
        }
    }

    #[test]
    fn a_session_card_is_signed_and_verifies_until_tampered() {
        let card = card_for_session(&input(), 1, "agent-console 0.89.0").unwrap();
        assert_eq!(card.principal, Principal::local_human());
        assert_eq!(
            card.allowed_hosts,
            vec!["api.github.com", ".npmjs.org", "github.com"]
        );
        let signed = sign_with_seed(card, &[7u8; 32]).unwrap();
        assert!(verify(&signed).unwrap());
        let mut tampered = signed.clone();
        tampered.card.allowed_hosts.push(".evil.example".into());
        assert!(!verify(&tampered).unwrap());
        let mut wrong_key = signed.clone();
        wrong_key.key_id = "0".repeat(64);
        assert!(!verify(&wrong_key).unwrap());
    }

    #[test]
    fn runs_in_is_a_closed_set_and_ids_are_urns() {
        assert!(runs_in_or_default(Some("microvm")).is_ok());
        assert!(runs_in_or_default(Some("laptop")).is_err());
        assert_eq!(runs_in_or_default(None).unwrap(), "local");
        let mut bad = input();
        bad.agent_id = "t1".into();
        assert!(card_for_session(&bad, 1, "x").is_err());
    }

    #[test]
    fn perimeter_matches_exact_or_suffix() {
        let allowed = vec![".github.com".to_string(), "npmjs.org".to_string()];
        assert!(host_allowed("api.github.com", &allowed));
        assert!(host_allowed("github.com", &allowed));
        assert!(host_allowed("NPMJS.org", &allowed));
        assert!(!host_allowed("registry.npmjs.org", &allowed));
        assert!(!host_allowed("evil.example", &allowed));
        assert!(!host_allowed("github.com.evil.example", &allowed));
    }

    #[test]
    fn reviewers_and_mcp_callers_have_their_own_ids_on_behalf_of_the_human() {
        let r = card_for_reviewer("codex", None, 1, "x");
        let m = card_for_mcp_caller("claude-desktop", 1, "x");
        assert_eq!(r.agent_id, "urn:agent-console:agent:reviewer:codex");
        assert_eq!(m.agent_id, "urn:agent-console:agent:mcp:claude-desktop");
        assert_eq!(r.principal.principal_type, "human");
        assert_ne!(r.agent_id, r.principal.principal_id);
    }
}
