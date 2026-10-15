use tauri::State;

use crate::error::AppResult;
use crate::services::identity_service::{self, SessionIdentityInput, SignedAgentCard};
use crate::state::AppState;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn issuer() -> String {
    format!("agent-console {}", env!("CARGO_PKG_VERSION"))
}

/// D14: mint and sign a session's Agent Card and witness it in the ledger
/// (once per project per agent id). Called by the webview when the PTY
/// spawns. The signature uses the console's keychain key; a machine without
/// a keychain still gets the card witnessed, unsigned, and an error here
/// never blocks the spawn (the caller treats it as best-effort).
#[tauri::command(async)]
pub fn identity_card(
    project_root: String,
    session: SessionIdentityInput,
    state: State<'_, AppState>,
) -> AppResult<SignedAgentCard> {
    let ts = now_ms();
    let card = identity_service::card_for_session(&session, ts, &issuer())?;
    let signed = identity_service::sign(card.clone())?;
    let _ = state.testigo.on_agent_identity(
        &project_root,
        ts,
        &card.agent_id,
        Some(&session.term_id),
        identity_service::ledger_payload(&card, Some(&signed)),
    );
    Ok(signed)
}

/// Verify a card someone hands you (offline: the key travels inside).
#[tauri::command(async)]
pub fn identity_verify(card: SignedAgentCard) -> AppResult<bool> {
    identity_service::verify(&card)
}
