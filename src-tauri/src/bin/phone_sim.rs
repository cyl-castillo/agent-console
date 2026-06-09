//! phone-sim — a reference phone client for the mobile voice companion.
//!
//! NOT the real mobile app (that lives in its own repo). This is a runnable
//! reference: it speaks the exact wire protocol (docs/pairing-protocol.md) over
//! a real TCP socket, so you can exercise the desktop end to end on one machine
//! and so the phone-app implementer has a working example to mirror.
//!
//! Usage (start the listener in agent-console first; it shows the address):
//!   phone-sim pair <addr> "<offer-uri-from-QR>"   # then approve on the desktop
//!   phone-sim say  <addr> "what's the build status?"
//!
//! State (its keypair + the pinned desktop key) is saved to
//! ~/.local/share/agent-console/phone-sim-state.json. A real app uses the OS
//! keystore, not a file — this is a dev tool.

use std::path::PathBuf;

use agent_console_lib::phone_protocol::{self, PhoneClient};
use agent_console_lib::ServerMessage;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn state_path() -> Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or("no data_local dir")?
        .join("agent-console");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("phone-sim-state.json"))
}

fn load_client() -> Result<PhoneClient> {
    let txt = std::fs::read_to_string(state_path()?)
        .map_err(|_| "no saved pairing — run `phone-sim pair <addr> <offer-uri>` first")?;
    let state = serde_json::from_str(&txt)?;
    Ok(PhoneClient::from_state(&state)?)
}

fn save_client(client: &PhoneClient) -> Result<()> {
    std::fs::write(state_path()?, serde_json::to_string_pretty(&client.to_state())?)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("pair") => {
            let addr = args.get(2).ok_or("usage: phone-sim pair <addr> <offer-uri>")?;
            let uri = args.get(3).ok_or("usage: phone-sim pair <addr> <offer-uri>")?;
            let offer = phone_protocol::parse_offer_uri(uri)?;
            let mut client = PhoneClient::new()?;
            let mut sock = tokio::net::TcpStream::connect(addr).await?;
            client.pair(&mut sock, &offer, "phone-sim (reference)").await?;
            save_client(&client)?;
            println!("paired — now approve \"phone-sim (reference)\" on the desktop, then use `say`.");
        }
        Some("say") => {
            let addr = args.get(2).ok_or("usage: phone-sim say <addr> <utterance>")?;
            let utterance = args.get(3).ok_or("usage: phone-sim say <addr> <utterance>")?;
            let client = load_client()?;
            let sock = tokio::net::TcpStream::connect(addr).await?;
            match client.say(sock, utterance).await? {
                ServerMessage::Say { text } => println!("🔊 {text}"),
                ServerMessage::Error { message } => eprintln!("error: {message}"),
                other => println!("{other:?}"),
            }
        }
        _ => {
            eprintln!("usage:\n  phone-sim pair <addr> <offer-uri>\n  phone-sim say  <addr> <utterance>");
            std::process::exit(2);
        }
    }
    Ok(())
}
