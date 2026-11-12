//! phone-sim — a reference client for the remote-decisions protocol (F8).
//!
//! NOT the real device app (that lives in its own repo). This is a runnable
//! reference: it speaks the exact wire protocol (docs/pairing-protocol.md)
//! over a real TCP socket, so you can exercise the desktop end to end on
//! one machine and so the app implementer has a working example to mirror.
//!
//! Usage (start the listener in agent-console → Trust → Remote devices, then
//! "Pair a device"; the QR's URI carries the address):
//!   phone-sim pair "<offer-uri-from-QR>" [addr]   # then approve on the desktop
//!   phone-sim pending                             # the decisions waiting
//!   phone-sim sessions                            # where each session stands
//!   phone-sim push-info                           # the key a browser subscribes with
//!   phone-sim decide <id> allow|deny ["reason"]
//!
//! Add `--ws` before the verb to dial the same port as a browser would (HTTP
//! upgrade to a WebSocket, then the identical protocol) — what the PWA does.
//!
//! State (its keypair, the pinned desktop key and the address) is saved to
//! ~/.local/share/agent-console/phone-sim-state.json. A real app uses the OS
//! keystore, not a file — this is a dev tool.

use std::path::PathBuf;

use agent_console_lib::phone_protocol::{self, PhoneClient};
use agent_console_lib::ServerMessage;
use tokio::io::{AsyncRead, AsyncWrite};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A dialled connection: the raw socket, or the same port behind a
/// WebSocket upgrade (`--ws`).
async fn dial(addr: &str, ws: bool) -> Result<Box<dyn Stream>> {
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    if ws {
        Ok(Box::new(agent_console_lib::ws_connect(tcp, addr).await?))
    } else {
        Ok(Box::new(tcp))
    }
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

fn state_path() -> Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or("no data_local dir")?
        .join("agent-console");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("phone-sim-state.json"))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    #[serde(flatten)]
    phone: phone_protocol::PhoneState,
    addr: String,
}

fn load() -> Result<(PhoneClient, String)> {
    let txt = std::fs::read_to_string(state_path()?)
        .map_err(|_| "no saved pairing — run `phone-sim pair <offer-uri>` first")?;
    let s: Saved = serde_json::from_str(&txt)?;
    Ok((PhoneClient::from_state(&s.phone)?, s.addr))
}

fn save(client: &PhoneClient, addr: &str) -> Result<()> {
    let s = Saved {
        phone: client.to_state(),
        addr: addr.to_string(),
    };
    std::fs::write(state_path()?, serde_json::to_string_pretty(&s)?)?;
    Ok(())
}

fn print_reply(r: ServerMessage) {
    match r {
        ServerMessage::Decisions { items, .. } if items.is_empty() => println!("nothing pending"),
        ServerMessage::Decisions { items, truncated } => {
            if truncated {
                println!("(more wait on the desktop — showing the newest)");
            }
            for d in items {
                let allow = if d.remote_allow {
                    "allow|deny"
                } else {
                    "deny only (changes the machine: keyboard decides)"
                };
                println!(
                    "{}  [{} · {}]  {}: {}\n    session {} · {}\n    evidence: verification {} · last check {}",
                    d.id,
                    d.tier,
                    d.tier_label,
                    d.tool,
                    d.summary,
                    d.session.as_deref().unwrap_or("?"),
                    allow,
                    d.evidence.verification.as_deref().unwrap_or("none"),
                    d.evidence
                        .last_check
                        .map(|c| format!(
                            "`{}` {} (exit {})",
                            c.command,
                            c.status,
                            c.exit_code.map(|x| x.to_string()).unwrap_or("?".into())
                        ))
                        .unwrap_or_else(|| "none".into()),
                );
            }
        }
        ServerMessage::Decided { id } => println!("recorded: {id}"),
        ServerMessage::Sessions { items } if items.is_empty() => println!("no sessions"),
        ServerMessage::Sessions { items } => {
            for s in items {
                println!(
                    "{:<10} {:<8} {}{}  ({})",
                    s.state,
                    s.agent.as_deref().unwrap_or("?"),
                    s.name,
                    if s.pending > 0 {
                        format!(" · {} waiting", s.pending)
                    } else {
                        String::new()
                    },
                    s.term_id
                );
            }
        }
        ServerMessage::PushInfo { vapid_public_key } => {
            println!("applicationServerKey: {vapid_public_key}")
        }
        ServerMessage::Subscribed => {
            println!("subscribed: the desktop will knock when decisions wait")
        }
        ServerMessage::Unsubscribed => println!("unsubscribed"),
        ServerMessage::Error { message } => eprintln!("error: {message}"),
        ServerMessage::Pong => println!("pong"),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().collect();
    let ws = args.iter().any(|a| a == "--ws");
    args.retain(|a| a != "--ws");
    match args.get(1).map(String::as_str) {
        Some("pair") => {
            let uri = args
                .get(2)
                .ok_or("usage: phone-sim pair <offer-uri> [addr]")?;
            let (offer, addr_in_uri) = phone_protocol::parse_offer_uri(uri)?;
            let addr = args
                .get(3)
                .cloned()
                .or(addr_in_uri)
                .ok_or("the offer carries no address — pass it as the second argument")?;
            let mut client = PhoneClient::new()?;
            let mut sock = dial(&addr, ws).await?;
            let sas = client
                .pair(&mut sock, &offer, "phone-sim (reference)")
                .await?;
            save(&client, &addr)?;
            println!(
                "paired — the desktop must show these four words on the pending pairing: {sas}\napprove \"phone-sim (reference)\" there only if they match, then `phone-sim pending`."
            );
        }
        Some("pending") => {
            let (client, addr) = load()?;
            print_reply(client.pending(dial(&addr, ws).await?).await?);
        }
        Some("sessions") => {
            let (client, addr) = load()?;
            print_reply(client.sessions(dial(&addr, ws).await?).await?);
        }
        Some("push-info") => {
            let (client, addr) = load()?;
            print_reply(client.push_info(dial(&addr, ws).await?).await?);
        }
        Some("decide") => {
            let id = args
                .get(2)
                .ok_or("usage: phone-sim decide <id> allow|deny [reason]")?;
            let decision = args
                .get(3)
                .ok_or("usage: phone-sim decide <id> allow|deny [reason]")?;
            let reason = args.get(4).map(String::as_str);
            let (client, addr) = load()?;
            print_reply(
                client
                    .decide(dial(&addr, ws).await?, id, decision, reason)
                    .await?,
            );
        }
        _ => {
            eprintln!(
                "usage:\n  phone-sim [--ws] pair <offer-uri> [addr]\n  phone-sim [--ws] pending\n  phone-sim [--ws] sessions\n  phone-sim [--ws] push-info\n  phone-sim [--ws] decide <id> allow|deny [reason]"
            );
            std::process::exit(2);
        }
    }
    Ok(())
}
