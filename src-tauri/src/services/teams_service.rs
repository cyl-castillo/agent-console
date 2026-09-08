//! Microsoft Teams integration (read-only): see the chats and messages sent to
//! you and pull their text into the composer. Nothing is ever posted back.
//!
//! - Auth: OAuth 2.0 device code flow against Microsoft Entra. The user brings
//!   their own app registration (a multi-tenant *public* client — there is no
//!   client secret anywhere in this flow).
//! - Secrets: access + refresh tokens live in the OS keychain, never in the
//!   JSON config, never in a log. The pending `device_code` never leaves this
//!   process — the frontend only ever sees the user-facing code.
//! - API: Microsoft Graph v1.0 with delegated `Chat.Read` — the signed-in
//!   user's own chats only.

use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::error::{AppError, AppResult};

const KEYRING_SERVICE: &str = "agent-console:teams";
const KEYRING_ACCESS: &str = "access-token";
const KEYRING_REFRESH: &str = "refresh-token";

const GRAPH: &str = "https://graph.microsoft.com/v1.0";
const SCOPES: &str =
    "offline_access https://graph.microsoft.com/User.Read https://graph.microsoft.com/Chat.Read";

/// Refresh the access token when it has less than this long left — a request
/// in flight must not outlive the token it carries.
const EXPIRY_SLACK_MS: i64 = 60_000;

/// The non-secret half of the connection, persisted as JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamsConfig {
    /// Application (client) ID of the user's own app registration.
    pub client_id: String,
    /// Entra tenant: "organizations", a domain, or a tenant GUID.
    pub tenant: String,
    /// Display name of the signed-in account, filled after a successful login.
    #[serde(default)]
    pub account: String,
}

/// What the UI needs to decide "connect form" vs "chat list", without ever
/// shipping a token to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamsStatus {
    pub configured: bool,
    pub client_id: String,
    pub tenant: String,
    pub account: String,
}

/// First half of the device code flow: what the user must do.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLoginStart {
    /// The short code the user types at the verification URL.
    pub user_code: String,
    pub verification_uri: String,
    /// Microsoft's own human-readable instruction line.
    pub message: String,
    /// How often the frontend should call `poll_login`.
    pub interval_ms: u64,
    pub expires_in_secs: u64,
}

/// One `poll_login` outcome. "Pending" is a normal state, not an error.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum LoginPoll {
    Pending,
    Connected { account: String },
    Failed { message: String },
}

/// One chat from `/me/chats`, flattened for the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamsChat {
    pub id: String,
    /// Group topic, or the other members' names for 1:1/unnamed chats.
    pub title: String,
    /// "oneOnOne" | "group" | "meeting".
    pub chat_type: String,
    /// Plain-text snippet of the last message, if Graph sent a preview.
    pub last_preview: String,
    /// ISO timestamp of the last activity — display + sort currency.
    pub last_activity: String,
}

/// One message from a chat, flattened for the UI. Newest first, as Graph
/// returns them.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamsMessage {
    pub id: String,
    pub from: String,
    pub created: String,
    /// Plain text — HTML bodies are converted, system events filtered out.
    pub body: String,
}

// ---------------------------------------------------------------------------
// Config + keychain plumbing
// ---------------------------------------------------------------------------

fn config_path() -> AppResult<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| AppError::Other("cannot resolve data dir".into()))?
        .join("agent-console");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("teams.json"))
}

fn keyring_entry(account: &str) -> AppResult<Entry> {
    Entry::new(KEYRING_SERVICE, account).map_err(|e| AppError::Other(format!("keyring open: {e}")))
}

pub fn load_config() -> Option<TeamsConfig> {
    let path = config_path().ok()?;
    let txt = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&txt).ok()
}

fn save_config(cfg: &TeamsConfig) -> AppResult<()> {
    let json = serde_json::to_string_pretty(cfg).map_err(|e| AppError::Other(e.to_string()))?;
    std::fs::write(config_path()?, json)?;
    Ok(())
}

/// The access token is stored WITH its expiry so a restart doesn't force a
/// refresh round-trip: `"{expires_at_ms}|{token}"`.
fn encode_access(token: &str, expires_at_ms: i64) -> String {
    format!("{expires_at_ms}|{token}")
}

fn decode_access(raw: &str) -> Option<(i64, String)> {
    let (exp, tok) = raw.split_once('|')?;
    let exp: i64 = exp.parse().ok()?;
    if tok.is_empty() {
        return None;
    }
    Some((exp, tok.to_string()))
}

fn store_tokens(access: &str, expires_in_secs: i64, refresh: Option<&str>) -> AppResult<()> {
    let expires_at = now_ms() + expires_in_secs.max(0) * 1000;
    keyring_entry(KEYRING_ACCESS)?
        .set_password(&encode_access(access, expires_at))
        .map_err(|e| AppError::Other(format!("keyring set: {e}")))?;
    // Entra rotates refresh tokens; only overwrite when a new one arrived.
    if let Some(r) = refresh.filter(|r| !r.is_empty()) {
        keyring_entry(KEYRING_REFRESH)?
            .set_password(r)
            .map_err(|e| AppError::Other(format!("keyring set: {e}")))?;
    }
    Ok(())
}

fn read_access() -> Option<(i64, String)> {
    let raw = keyring_entry(KEYRING_ACCESS).ok()?.get_password().ok()?;
    decode_access(&raw)
}

fn read_refresh() -> Option<String> {
    keyring_entry(KEYRING_REFRESH).ok()?.get_password().ok()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn status() -> TeamsStatus {
    let cfg = load_config().unwrap_or_default();
    let connected = read_refresh().is_some();
    TeamsStatus {
        configured: !cfg.client_id.is_empty() && connected,
        client_id: cfg.client_id,
        tenant: cfg.tenant,
        account: cfg.account,
    }
}

/// Forget the connection entirely: config file + both keychain tokens.
pub fn disconnect() -> AppResult<()> {
    if let Ok(path) = config_path() {
        let _ = std::fs::remove_file(path);
    }
    for account in [KEYRING_ACCESS, KEYRING_REFRESH] {
        if let Ok(entry) = keyring_entry(account) {
            let _ = entry.delete_credential();
        }
    }
    *pending().lock().unwrap() = None;
    Ok(())
}

// ---------------------------------------------------------------------------
// Input validation — these values become request URLs/bodies.
// ---------------------------------------------------------------------------

/// Client IDs are GUIDs, exactly: 8-4-4-4-12 hex.
fn valid_client_id(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 36
        && id.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Tenant: "organizations"/"common"/"consumers", a domain, or a GUID. Empty
/// defaults to "organizations" (any work/school account).
fn normalize_tenant(raw: &str) -> AppResult<String> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok("organizations".into());
    }
    if t.len() > 100
        || !t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
    {
        return Err(AppError::InvalidArgument(
            "tenant must be a domain, a GUID, or 'organizations'".into(),
        ));
    }
    Ok(t.to_string())
}

/// Chat ids look like `19:...@thread.v2` / `19:uuid_uuid@unq.gbl.spaces`.
/// Charset-guarded because the id is embedded in a request path.
fn valid_chat_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '@' | '.' | '_' | '-'))
}

// ---------------------------------------------------------------------------
// Device code login
// ---------------------------------------------------------------------------

/// The half-open login. Kept in-process so the sensitive `device_code` never
/// crosses the IPC boundary; only one login can be pending at a time.
#[derive(Clone)]
struct PendingLogin {
    device_code: String,
    client_id: String,
    tenant: String,
}

fn pending() -> &'static Mutex<Option<PendingLogin>> {
    static PENDING: Mutex<Option<PendingLogin>> = Mutex::new(None);
    &PENDING
}

fn client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("agent-console")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::Other(format!("http client: {e}")))
}

fn login_base(tenant: &str) -> String {
    format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0")
}

/// Start the device code flow. Persists client_id + tenant right away so a
/// reconnect pre-fills the form even if the login is never completed.
pub async fn begin_login(client_id: &str, tenant: &str) -> AppResult<DeviceLoginStart> {
    let client_id = client_id.trim();
    if !valid_client_id(client_id) {
        return Err(AppError::InvalidArgument(
            "client ID must be the app registration's GUID".into(),
        ));
    }
    let tenant = normalize_tenant(tenant)?;

    let resp = client()?
        .post(format!("{}/devicecode", login_base(&tenant)))
        .form(&[("client_id", client_id), ("scope", SCOPES)])
        .send()
        .await
        .map_err(|e| AppError::Other(format!("Microsoft login request failed: {e}")))?;
    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Other(format!("bad Microsoft login response: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Other(friendly_aad_error(
            body["error"].as_str().unwrap_or(""),
            body["error_description"].as_str().unwrap_or(""),
        )));
    }

    let device_code = body["device_code"].as_str().unwrap_or_default().to_string();
    let user_code = body["user_code"].as_str().unwrap_or_default().to_string();
    if device_code.is_empty() || user_code.is_empty() {
        return Err(AppError::Other(
            "Microsoft login response is missing the device code".into(),
        ));
    }

    save_config(&TeamsConfig {
        client_id: client_id.to_string(),
        tenant: tenant.clone(),
        account: load_config().map(|c| c.account).unwrap_or_default(),
    })?;
    *pending().lock().unwrap() = Some(PendingLogin {
        device_code,
        client_id: client_id.to_string(),
        tenant,
    });

    Ok(DeviceLoginStart {
        user_code,
        verification_uri: body["verification_uri"]
            .as_str()
            .unwrap_or("https://microsoft.com/devicelogin")
            .to_string(),
        message: body["message"].as_str().unwrap_or_default().to_string(),
        interval_ms: body["interval"].as_u64().unwrap_or(5) * 1000,
        expires_in_secs: body["expires_in"].as_u64().unwrap_or(900),
    })
}

/// One poll of the pending login. The frontend drives the cadence using the
/// `interval_ms` from `begin_login`.
pub async fn poll_login() -> AppResult<LoginPoll> {
    let Some(p) = pending().lock().unwrap().clone() else {
        return Ok(LoginPoll::Failed {
            message: "no login in progress — start the connection again".into(),
        });
    };

    let resp = client()?
        .post(format!("{}/token", login_base(&p.tenant)))
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", &p.client_id),
            ("device_code", &p.device_code),
        ])
        .send()
        .await
        .map_err(|e| AppError::Other(format!("Microsoft login request failed: {e}")))?;
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Other(format!("bad Microsoft login response: {e}")))?;

    if let Some(err) = body["error"].as_str().filter(|e| !e.is_empty()) {
        // slow_down is Entra telling us to stretch the interval; the frontend
        // cadence is fixed, so treating it as pending is enough.
        if err == "authorization_pending" || err == "slow_down" {
            return Ok(LoginPoll::Pending);
        }
        *pending().lock().unwrap() = None;
        return Ok(LoginPoll::Failed {
            message: friendly_aad_error(err, body["error_description"].as_str().unwrap_or("")),
        });
    }

    let access = body["access_token"].as_str().unwrap_or_default();
    if access.is_empty() {
        return Err(AppError::Other(
            "Microsoft login response has no access token".into(),
        ));
    }
    store_tokens(
        access,
        body["expires_in"].as_i64().unwrap_or(3600),
        body["refresh_token"].as_str(),
    )?;
    *pending().lock().unwrap() = None;

    // Confirmation for the connect form; the login already succeeded, so a
    // profile hiccup falls back to a generic label rather than failing.
    let account = fetch_display_name(access)
        .await
        .unwrap_or_else(|| "Microsoft account".into());
    let mut cfg = load_config().unwrap_or_default();
    cfg.account = account.clone();
    save_config(&cfg)?;
    Ok(LoginPoll::Connected { account })
}

/// Abandon the pending login (user closed the form).
pub fn cancel_login() {
    *pending().lock().unwrap() = None;
}

async fn fetch_display_name(access: &str) -> Option<String> {
    let resp = client()
        .ok()?
        .get(format!("{GRAPH}/me"))
        .bearer_auth(access)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let me: serde_json::Value = resp.json().await.ok()?;
    me["displayName"]
        .as_str()
        .or_else(|| me["userPrincipalName"].as_str())
        .map(str::to_string)
}

/// Translate Entra's error soup into one actionable line. AADSTS90094 is the
/// verdict this whole feature gates on, so it gets its own message.
fn friendly_aad_error(code: &str, description: &str) -> String {
    if description.contains("AADSTS90094") || description.contains("AADSTS65001") {
        return "Your organization requires admin approval for this app (AADSTS90094). \
                Ask your IT admin to approve it — it only reads your own chats."
            .into();
    }
    match code {
        "authorization_declined" | "access_denied" => {
            "The sign-in was declined. Start the connection again to retry.".into()
        }
        "expired_token" => "The code expired before the sign-in finished. Connect again.".into(),
        "invalid_client" | "unauthorized_client" => {
            "Microsoft rejected the client ID — check the app registration \
             (multi-tenant, 'Allow public client flows' enabled)."
                .into()
        }
        "invalid_grant" => "The sign-in could not be completed. Connect again.".into(),
        _ => {
            // First line only: AADSTS descriptions end with correlation IDs
            // and timestamps that mean nothing to the user.
            let first = description.lines().next().unwrap_or("").trim();
            if first.is_empty() {
                format!("Microsoft login failed ({code})")
            } else {
                format!("Microsoft login failed: {first}")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Graph calls (read-only)
// ---------------------------------------------------------------------------

/// A valid access token: the cached one if it still has slack, otherwise a
/// refresh round-trip. The "reconnect" error is the UI's cue to show the
/// connect form again.
async fn access_token() -> AppResult<String> {
    if let Some((exp, tok)) = read_access() {
        if now_ms() + EXPIRY_SLACK_MS < exp {
            return Ok(tok);
        }
    }
    let refresh = read_refresh().ok_or_else(|| {
        AppError::InvalidArgument("Teams is not connected — connect it first".into())
    })?;
    let cfg = load_config()
        .filter(|c| !c.client_id.is_empty())
        .ok_or_else(|| AppError::InvalidArgument("Teams is not configured".into()))?;

    let resp = client()?
        .post(format!("{}/token", login_base(&cfg.tenant)))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &cfg.client_id),
            ("refresh_token", &refresh),
            ("scope", SCOPES),
        ])
        .send()
        .await
        .map_err(|e| AppError::Other(format!("Microsoft token refresh failed: {e}")))?;
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Other(format!("bad Microsoft token response: {e}")))?;
    let access = body["access_token"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if access.is_empty() {
        // The refresh token died (revoked, expired, password change). Drop it
        // so status() flips to unconfigured and the UI offers to reconnect.
        if let Ok(entry) = keyring_entry(KEYRING_REFRESH) {
            let _ = entry.delete_credential();
        }
        return Err(AppError::Other(
            "The Teams session expired — connect again.".into(),
        ));
    }
    store_tokens(
        &access,
        body["expires_in"].as_i64().unwrap_or(3600),
        body["refresh_token"].as_str(),
    )?;
    Ok(access)
}

/// Your chats, most recently active first.
pub async fn list_chats() -> AppResult<Vec<TeamsChat>> {
    let token = access_token().await?;
    let own_account = load_config().map(|c| c.account).unwrap_or_default();
    let resp = client()?
        .get(format!("{GRAPH}/me/chats"))
        .bearer_auth(&token)
        .query(&[
            ("$expand", "members,lastMessagePreview"),
            ("$top", "50"),
            ("$orderby", "lastMessagePreview/createdDateTime desc"),
        ])
        .send()
        .await
        .map_err(|e| AppError::Other(format!("Teams request failed: {e}")))?;
    let parsed: RawChatsResp = check_graph(resp).await?;
    Ok(parsed
        .value
        .into_iter()
        .map(|c| flatten_chat(&own_account, c))
        .collect())
}

/// Messages of one chat, newest first. System events (member added, calls)
/// are filtered out — the panel is about what people wrote.
pub async fn list_messages(chat_id: &str) -> AppResult<Vec<TeamsMessage>> {
    if !valid_chat_id(chat_id) {
        return Err(AppError::InvalidArgument(
            "that doesn't look like a chat id".into(),
        ));
    }
    let token = access_token().await?;
    let resp = client()?
        .get(format!("{GRAPH}/me/chats/{chat_id}/messages"))
        .bearer_auth(&token)
        .query(&[("$top", "50")])
        .send()
        .await
        .map_err(|e| AppError::Other(format!("Teams request failed: {e}")))?;
    let parsed: RawMsgsResp = check_graph(resp).await?;
    Ok(parsed
        .value
        .into_iter()
        .filter_map(flatten_message)
        .collect())
}

/// Shared Graph response gate: 401 → reconnect cue, other failures → status.
async fn check_graph<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> AppResult<T> {
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError::Other(
            "Microsoft rejected the token (401) — try connecting again".into(),
        ));
    }
    if !resp.status().is_success() {
        return Err(AppError::Other(format!(
            "Microsoft Graph returned {}",
            resp.status()
        )));
    }
    resp.json()
        .await
        .map_err(|e| AppError::Other(format!("bad Microsoft Graph response: {e}")))
}

fn flatten_chat(own_account: &str, c: RawChat) -> TeamsChat {
    let members = c.members.unwrap_or_default();
    let others: Vec<String> = members
        .iter()
        .filter_map(|m| m.display_name.clone())
        .filter(|n| !n.is_empty() && n != own_account)
        .collect();
    let title = match c.topic.filter(|t| !t.is_empty()) {
        Some(t) => t,
        None if !others.is_empty() => others.join(", "),
        None => "Chat".into(),
    };
    let (last_preview, preview_ts) = match c.last_message_preview {
        Some(p) => {
            let text = p
                .body
                .map(|b| plain_body(b.content_type.as_deref(), &b.content.unwrap_or_default()))
                .unwrap_or_default();
            (
                truncate_chars(&text.replace('\n', " "), 140),
                p.created_date_time,
            )
        }
        None => (String::new(), None),
    };
    TeamsChat {
        id: c.id,
        title,
        chat_type: c.chat_type.unwrap_or_default(),
        last_preview,
        last_activity: preview_ts.or(c.last_updated_date_time).unwrap_or_default(),
    }
}

/// None = not a human message (system event, empty body) — dropped upstream.
fn flatten_message(m: RawMsg) -> Option<TeamsMessage> {
    if m.message_type.as_deref() != Some("message") {
        return None;
    }
    let body = m
        .body
        .map(|b| plain_body(b.content_type.as_deref(), &b.content.unwrap_or_default()))
        .unwrap_or_default();
    if body.trim().is_empty() {
        return None;
    }
    let from = m
        .from
        .and_then(|f| {
            f.user
                .and_then(|u| u.display_name)
                .or_else(|| f.application.and_then(|a| a.display_name))
        })
        .unwrap_or_else(|| "unknown".into());
    Some(TeamsMessage {
        id: m.id,
        from,
        created: m.created_date_time.unwrap_or_default(),
        body,
    })
}

fn plain_body(content_type: Option<&str>, content: &str) -> String {
    match content_type {
        Some("html") => html_to_text(content),
        _ => content.trim().to_string(),
    }
}

/// Teams message bodies are simple HTML (p/div/br, entities, the odd emoji
/// tag). This flattens them to readable plain text: block-closers become
/// newlines, entities are decoded, everything else is stripped. Deliberately
/// not a real HTML parser — the output feeds a composer, not a renderer.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                let mut tag = String::new();
                for t in chars.by_ref() {
                    if t == '>' {
                        break;
                    }
                    tag.push(t);
                }
                let closing = tag.starts_with('/');
                let name: String = tag
                    .trim_start_matches('/')
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_ascii_lowercase();
                if name == "br"
                    || (closing
                        && matches!(
                            name.as_str(),
                            "p" | "div" | "li" | "tr" | "blockquote" | "h1" | "h2" | "h3"
                        ))
                {
                    out.push('\n');
                }
            }
            '&' => {
                let mut entity = String::new();
                let mut terminated = false;
                while let Some(&n) = chars.peek() {
                    if n == ';' {
                        chars.next();
                        terminated = true;
                        break;
                    }
                    if entity.len() >= 10 || n == '&' || n == '<' {
                        break;
                    }
                    entity.push(n);
                    chars.next();
                }
                match decode_entity(&entity) {
                    Some(ch) if terminated => out.push(ch),
                    _ => {
                        out.push('&');
                        out.push_str(&entity);
                        if terminated {
                            out.push(';');
                        }
                    }
                }
            }
            _ => out.push(c),
        }
    }
    // Tidy: no trailing space before newlines, at most one blank line in a row.
    let mut lines: Vec<&str> = out.lines().map(str::trim_end).collect();
    while lines.first().is_some_and(|l| l.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let mut tidy: Vec<&str> = Vec::with_capacity(lines.len());
    for l in lines {
        if l.is_empty() && tidy.last().is_some_and(|p| p.is_empty()) {
            continue;
        }
        tidy.push(l);
    }
    tidy.join("\n")
}

fn decode_entity(e: &str) -> Option<char> {
    match e {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            let num = e.strip_prefix('#')?;
            let code = match num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// Char-safe truncation with an ellipsis — previews, not prose.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// --- Graph response shapes (only the fields we read) ---

#[derive(Deserialize)]
struct RawChatsResp {
    #[serde(default)]
    value: Vec<RawChat>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawChat {
    id: String,
    topic: Option<String>,
    chat_type: Option<String>,
    last_updated_date_time: Option<String>,
    members: Option<Vec<RawMember>>,
    last_message_preview: Option<RawPreview>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMember {
    display_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPreview {
    created_date_time: Option<String>,
    body: Option<RawBody>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawBody {
    content_type: Option<String>,
    content: Option<String>,
}

#[derive(Deserialize)]
struct RawMsgsResp {
    #[serde(default)]
    value: Vec<RawMsg>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMsg {
    id: String,
    message_type: Option<String>,
    created_date_time: Option<String>,
    from: Option<RawFrom>,
    body: Option<RawBody>,
}

#[derive(Deserialize)]
struct RawFrom {
    user: Option<RawUser>,
    application: Option<RawUser>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawUser {
    display_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_id_must_be_a_guid() {
        assert!(valid_client_id("12345678-abcd-ef01-2345-6789abcdef01"));
        assert!(!valid_client_id("12345678-abcd-ef01-2345-6789abcdef0"));
        assert!(!valid_client_id("12345678abcdef0123456789abcdef012345"));
        assert!(!valid_client_id("1234567g-abcd-ef01-2345-6789abcdef01"));
        assert!(!valid_client_id(""));
    }

    #[test]
    fn tenant_normalization() {
        assert_eq!(normalize_tenant("").unwrap(), "organizations");
        assert_eq!(normalize_tenant("  ").unwrap(), "organizations");
        assert_eq!(normalize_tenant("contoso.com").unwrap(), "contoso.com");
        assert_eq!(
            normalize_tenant("12345678-abcd-ef01-2345-6789abcdef01").unwrap(),
            "12345678-abcd-ef01-2345-6789abcdef01"
        );
        assert!(normalize_tenant("bad tenant").is_err());
        assert!(normalize_tenant("evil/../path").is_err());
        assert!(normalize_tenant(&"x".repeat(101)).is_err());
    }

    #[test]
    fn chat_id_charset_guard() {
        assert!(valid_chat_id("19:abc-def_123@thread.v2"));
        assert!(valid_chat_id("19:uuid_uuid@unq.gbl.spaces"));
        assert!(!valid_chat_id(""));
        assert!(!valid_chat_id("19:x/../../admin"));
        assert!(!valid_chat_id("19:x?y=1"));
        assert!(!valid_chat_id(&"x".repeat(201)));
    }

    #[test]
    fn access_token_encoding_round_trips() {
        let raw = encode_access("tok-abc", 1234567);
        assert_eq!(decode_access(&raw), Some((1234567, "tok-abc".into())));
        assert_eq!(decode_access("junk"), None);
        assert_eq!(decode_access("notanumber|tok"), None);
        assert_eq!(decode_access("123|"), None);
        // Tokens may themselves contain '|' (they're opaque) — split on the first.
        assert_eq!(decode_access("5|a|b"), Some((5, "a|b".into())));
    }

    #[test]
    fn aad_errors_become_actionable_messages() {
        let admin = friendly_aad_error(
            "invalid_grant",
            "AADSTS90094: The grant requires admin permission. Trace ID: xyz",
        );
        assert!(admin.contains("admin approval"), "{admin}");
        assert!(friendly_aad_error("expired_token", "").contains("expired"));
        assert!(friendly_aad_error("access_denied", "").contains("declined"));
        assert!(friendly_aad_error("invalid_client", "").contains("client ID"));
        // Unknown code: first description line only, no correlation noise.
        let other = friendly_aad_error("weird_code", "Something broke.\nTrace ID: abc");
        assert!(other.contains("Something broke."));
        assert!(!other.contains("Trace ID"));
        assert_eq!(
            friendly_aad_error("weird_code", ""),
            "Microsoft login failed (weird_code)"
        );
    }

    #[test]
    fn html_to_text_flattens_teams_bodies() {
        assert_eq!(
            html_to_text("<p>hola <b>mundo</b></p><p>chau</p>"),
            "hola mundo\nchau"
        );
        assert_eq!(html_to_text("a<br>b<br/>c"), "a\nb\nc");
        assert_eq!(
            html_to_text("x &amp; y &lt;z&gt; &quot;q&quot; &#65; &#x42;"),
            "x & y <z> \"q\" A B"
        );
        // Unknown/unterminated entities pass through literally.
        assert_eq!(html_to_text("AT&T &unknown; a&b"), "AT&T &unknown; a&b");
        // Blank-line runs collapse; edges are trimmed.
        assert_eq!(
            html_to_text("<div></div><div></div><div>solo</div><div></div>"),
            "solo"
        );
        assert_eq!(html_to_text(""), "");
    }

    #[test]
    fn chat_flattening_derives_titles() {
        let chat = |topic: Option<&str>, members: Vec<&str>| RawChat {
            id: "19:x@thread.v2".into(),
            topic: topic.map(str::to_string),
            chat_type: Some("group".into()),
            last_updated_date_time: Some("2026-09-08T12:00:00Z".into()),
            members: Some(
                members
                    .into_iter()
                    .map(|n| RawMember {
                        display_name: Some(n.to_string()),
                    })
                    .collect(),
            ),
            last_message_preview: Some(RawPreview {
                created_date_time: Some("2026-09-08T13:00:00Z".into()),
                body: Some(RawBody {
                    content_type: Some("html".into()),
                    content: Some("<p>hola<br>equipo</p>".into()),
                }),
            }),
        };
        // Topic wins when present.
        let c = flatten_chat("Carlos", chat(Some("Sprint 12"), vec!["Carlos", "Ana"]));
        assert_eq!(c.title, "Sprint 12");
        // No topic: other members, self excluded; preview is one flat line.
        let c = flatten_chat("Carlos", chat(None, vec!["Carlos", "Ana", "Luis"]));
        assert_eq!(c.title, "Ana, Luis");
        assert_eq!(c.last_preview, "hola equipo");
        assert_eq!(c.last_activity, "2026-09-08T13:00:00Z");
        // Nobody else (self-chat): generic label, preview timestamp missing
        // falls back to the chat's lastUpdated.
        let mut solo = chat(None, vec!["Carlos"]);
        solo.last_message_preview = None;
        let c = flatten_chat("Carlos", solo);
        assert_eq!(c.title, "Chat");
        assert_eq!(c.last_activity, "2026-09-08T12:00:00Z");
    }

    #[test]
    fn message_flattening_filters_system_events() {
        let msg = |mtype: &str, body: Option<&str>| RawMsg {
            id: "m1".into(),
            message_type: Some(mtype.into()),
            created_date_time: Some("2026-09-08T13:00:00Z".into()),
            from: Some(RawFrom {
                user: Some(RawUser {
                    display_name: Some("Ana".into()),
                }),
                application: None,
            }),
            body: body.map(|b| RawBody {
                content_type: Some("html".into()),
                content: Some(b.to_string()),
            }),
        };
        let m = flatten_message(msg("message", Some("<p>dale</p>"))).unwrap();
        assert_eq!(m.from, "Ana");
        assert_eq!(m.body, "dale");
        // System events and empty bodies are dropped.
        assert!(flatten_message(msg("systemEventMessage", Some("<p>x</p>"))).is_none());
        assert!(flatten_message(msg("message", Some("  "))).is_none());
        assert!(flatten_message(msg("message", None)).is_none());
        // Bot/app senders still get a name.
        let mut bot = msg("message", Some("ping"));
        bot.from = Some(RawFrom {
            user: None,
            application: Some(RawUser {
                display_name: Some("CI Bot".into()),
            }),
        });
        assert_eq!(flatten_message(bot).unwrap().from, "CI Bot");
    }

    #[test]
    fn preview_truncation_is_char_safe() {
        assert_eq!(truncate_chars("corto", 10), "corto");
        assert_eq!(truncate_chars("ññññññ", 4), "ñññ…");
        assert_eq!(truncate_chars("", 5), "");
    }

    /// One test fn — it mutates the process-global XDG_DATA_HOME.
    #[test]
    fn config_round_trip_without_touching_keychain() {
        let _env = crate::test_support::lock_env();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data = std::env::temp_dir().join(format!("ac-teams-{}-{nanos}", std::process::id()));
        let prev = std::env::var("XDG_DATA_HOME").ok();
        std::env::set_var("XDG_DATA_HOME", &data);

        let run = || -> AppResult<()> {
            assert!(load_config().is_none());
            save_config(&TeamsConfig {
                client_id: "12345678-abcd-ef01-2345-6789abcdef01".into(),
                tenant: "organizations".into(),
                account: "Carlos".into(),
            })?;
            let cfg = load_config().expect("config should round-trip");
            assert_eq!(cfg.client_id, "12345678-abcd-ef01-2345-6789abcdef01");
            assert_eq!(cfg.account, "Carlos");
            // The config file must never contain anything token-shaped.
            let raw = std::fs::read_to_string(config_path()?)?;
            assert!(!raw.to_lowercase().contains("token"));
            Ok(())
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
        match prev {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
        std::fs::remove_dir_all(&data).ok();
        match result {
            Ok(inner) => inner.unwrap(),
            Err(p) => std::panic::resume_unwind(p),
        }
    }
}
