//! The console's own assistant: one optional agent the human talks to in
//! plain language, that knows the product and drives it through the console's
//! MCP server — on the engine the human already uses, never a loop of ours.
//!
//! What it may do is decided here and enforced on the server, not in the
//! prompt: its identity ([`CALLER`]) is confined to [`TOOLS`], a read-and-
//! navigate subset of `console_mcp::TOOL_NAMES`. It cannot type into a
//! session, open a review, decline on someone's behalf or reach any action
//! `console_mcp::action_allowed` refuses. Every tool call lands in the ledger
//! under `agent:console-assistant`, and each turn is witnessed as its own
//! `agent_action` (without the human's words: a question may carry a secret).
//!
//! Engine parity: Claude (`--mcp-config` + allow-list), Codex (`-c` override
//! on `app-server` / `exec`) and OpenCode over ACP (`session/new`) all get the
//! same server. OpenCode's legacy `run` fallback has no way to attach it and
//! answers from the guide alone.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::services::agent_run::{self, RunSpec};
use crate::services::engine_runner::{ActivitySink, Engine, McpAttach, McpServerKind, ToolPolicy};

/// The identity the assistant acts under. Reserved: the console server
/// confines any caller with this name to [`TOOLS`], so claiming it from
/// outside can only narrow what an agent may do.
pub const CALLER: &str = "console-assistant";

/// The assistant's tools. Lockstep with `assistant_tools()` in hook-bridge
/// (what it is shown); this list is what the app accepts.
pub const TOOLS: &[&str] = &[
    "console_state",
    "list_actions",
    "invoke_action",
    "list_sessions",
    "session_events",
    "memory_search",
    "ledger_verify",
    "verification_status",
];

/// Whether `caller` may call `tool`: everyone but the assistant is governed
/// by the server's own list; the assistant only by [`TOOLS`].
pub fn tool_allowed(caller: &str, tool: &str) -> bool {
    caller != CALLER || TOOLS.contains(&tool)
}

/// A question is typed by a person: this is far beyond any real one and
/// keeps a pasted file from becoming a turn.
pub const MAX_MESSAGE_CHARS: usize = 4000;

/// An assistant turn reads state and navigates; silence this long is a hang.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

const GUIDE: &str = include_str!("assistant_guide.md");

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantReply {
    pub text: String,
    /// The engine's session id: pass it back to keep the conversation.
    pub session_id: Option<String>,
    pub tokens: u64,
    pub cost_usd: f64,
    /// False when the sidecar is missing: the answer came from the guide
    /// alone and the assistant could not look at or drive the console.
    pub tools_attached: bool,
    /// The turn's id in the learning log, when the human keeps one: what a
    /// rating refers to.
    pub log_id: Option<String>,
}

/// Trim, refuse empty and oversized input.
pub fn clean_message(raw: &str) -> AppResult<String> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(AppError::Other("Write a question first".into()));
    }
    if text.chars().count() > MAX_MESSAGE_CHARS {
        return Err(AppError::Other(format!(
            "That is longer than {MAX_MESSAGE_CHARS} characters — ask a shorter question"
        )));
    }
    Ok(text.to_string())
}

/// The turn's prompt. The first turn carries the role and the guide; a
/// resumed one only the message — the engine's session already holds the
/// rest. The human's text is fenced and labelled so it reads as a request,
/// not as instructions that outrank the role.
pub fn build_prompt(message: &str, first_turn: bool, tools_attached: bool) -> String {
    if !first_turn {
        return format!("The human says:\n\"\"\"\n{message}\n\"\"\"");
    }
    let tools = if tools_attached {
        "Before you answer anything about the current state, look: call `console_state` or `list_actions` instead of guessing. When the human asks to go somewhere or open something, do it with `invoke_action` and say what you did in one line."
    } else {
        "The console's tools are not available in this turn: answer from the guide, say so once, and tell the human where to click."
    };
    format!(
        "You are the built-in assistant of Agent Console, the app the human is using right now. \
Your only job is to help them use this app: answer how it works, find things, and operate it for them.\n\
{tools}\n\
Rules:\n\
- Answer in the language the human writes in, in a few short sentences. No preamble.\n\
- Never invent a panel, a setting or an action. If the guide and the tools do not show it, say you do not know.\n\
- You cannot edit files, run commands, type into a session, approve or deny anything. If asked, say where the human does it and take them there.\n\
- Do not read or modify the project's files; questions about the code belong in a session.\n\
- If something in the app looks broken or missing for what they want, say it plainly — that is useful.\n\n\
<guide>\n{GUIDE}\n</guide>\n\n\
The human says:\n\"\"\"\n{message}\n\"\"\""
    )
}

/// Everything one assistant turn needs.
pub struct AskSpec<'a> {
    pub project_root: &'a Path,
    /// The hook-bridge sidecar; `None` when this build shipped without it.
    pub bridge: Option<&'a Path>,
    pub engine: Engine,
    /// `None` = the user's configured default for that engine.
    pub model: Option<&'a str>,
    pub resume: Option<&'a str>,
    pub message: &'a str,
    pub on_activity: Option<&'a ActivitySink<'a>>,
}

/// Run one turn. Blocking: call from `spawn_blocking`.
pub fn ask(spec: &AskSpec) -> AppResult<AssistantReply> {
    let message = clean_message(spec.message)?;
    // The engine starts in the project folder, with whatever the folder
    // hooks into it: the assistant waits for the human's word like the rest.
    let permit = crate::services::trust_service::launch_permit(spec.project_root)?;
    let project = spec.project_root.to_string_lossy();
    let mcp = spec.bridge.map(|bridge| McpAttach {
        bridge,
        project: &project,
        job: "",
        caller: CALLER,
        server: McpServerKind::Console,
    });
    let tools_attached = mcp.is_some();
    let prompt = build_prompt(&message, spec.resume.is_none(), tools_attached);
    let out = agent_run::run_attached(
        &RunSpec {
            permit: &permit,
            engine: spec.engine,
            cwd: spec.project_root,
            prompt: &prompt,
            // Read-only: the engine may not edit or run anything; what the
            // assistant does to the console goes through the server above.
            policy: ToolPolicy::ReadOnly,
            model: spec.model,
            resume: spec.resume,
            idle_timeout: IDLE_TIMEOUT,
            on_activity: spec.on_activity,
        },
        mcp,
    )?;
    Ok(AssistantReply {
        text: out.text.trim().to_string(),
        session_id: out.session_id,
        tokens: out.tokens,
        cost_usd: out.cost_usd,
        tools_attached,
        log_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::console_mcp;

    #[test]
    fn the_assistant_is_confined_to_its_tools_and_nobody_else_is() {
        for tool in console_mcp::TOOL_NAMES {
            assert!(tool_allowed("some-agent", tool));
        }
        for tool in ["send_prompt", "new_session", "decline", "request_review"] {
            assert!(!tool_allowed(CALLER, tool), "{tool} must stay out");
        }
        for tool in TOOLS {
            assert!(tool_allowed(CALLER, tool));
        }
    }

    #[test]
    fn every_assistant_tool_exists_on_the_server() {
        for tool in TOOLS {
            assert!(
                console_mcp::TOOL_NAMES.contains(tool),
                "{tool} is not a console tool"
            );
        }
    }

    #[test]
    fn messages_are_trimmed_and_bounded() {
        assert_eq!(clean_message("  hola  ").unwrap(), "hola");
        assert!(clean_message("   ").is_err());
        assert!(clean_message(&"á".repeat(MAX_MESSAGE_CHARS)).is_ok());
        assert!(clean_message(&"á".repeat(MAX_MESSAGE_CHARS + 1)).is_err());
    }

    #[test]
    fn the_first_turn_carries_role_and_guide_and_a_resumed_one_only_the_message() {
        let first = build_prompt("¿dónde veo el ledger?", true, true);
        assert!(first.contains("built-in assistant of Agent Console"));
        assert!(first.contains("<guide>"));
        assert!(first.contains("`console_state`"));
        assert!(first.ends_with("\"\"\"\n¿dónde veo el ledger?\n\"\"\""));

        let next = build_prompt("gracias", false, true);
        assert!(!next.contains("<guide>"));
        assert!(next.contains("gracias"));
    }

    #[test]
    fn without_the_sidecar_the_prompt_says_the_tools_are_missing() {
        let p = build_prompt("hola", true, false);
        assert!(p.contains("tools are not available"));
        assert!(!p.contains("Before you answer anything about the current state"));
    }

    #[test]
    fn the_guide_names_only_tools_the_assistant_has() {
        for tool in console_mcp::TOOL_NAMES {
            if !TOOLS.contains(tool) {
                assert!(
                    !GUIDE.contains(&format!("`{tool}`")),
                    "the guide offers {tool}, which the assistant cannot call"
                );
            }
        }
    }
}
