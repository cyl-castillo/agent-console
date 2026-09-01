//! Learning a terminal's resume handle from the CLI itself.
//!
//! Until now the only source of a session's resume id was the UserPromptSubmit
//! hook: the agent's own session id reached us because a prompt passed through
//! our bridge. When the hook does NOT fire — hooks not installed yet, a
//! directory the CLI doesn't consider trusted, an agent the user launched by
//! hand inside the PTY — the terminal ends up with no id at all, and a restart
//! silently starts a fresh conversation ("starting fresh (no session id)").
//!
//! `claude agents --json` (Claude Code 2.1.145) lists the live sessions with
//! their pid, which lets us close that gap WITHOUT guessing: our PTY knows the
//! pid of the shell it spawned, and the agent runs as a descendant of that
//! shell. A live agent whose process chain reaches this terminal's shell IS
//! this terminal's agent — proof, not a heuristic.
//!
//! The rules that keep it honest:
//! - a terminal adopts an id only when EXACTLY ONE live agent descends from it
//!   (a nested agent makes it ambiguous, and ambiguity yields nothing);
//! - an agent belongs to at most one terminal (a pid can't match two shells,
//!   but the check is explicit so a broken process table can't cross-bind);
//! - the id shape is validated at the parse boundary, because the payload
//!   describes another process and ends up in a shell command line.
//!
//! Windows has no cheap parent-pid probe, so `parent_of` reports nothing there
//! and this pass simply finds no matches: the hook remains the only source, as
//! before.

use crate::services::claude_cli::{self, LiveAgent};
use crate::services::proc;

/// A terminal and the live session id it should resume with.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermBinding {
    /// Frontend terminal-session id (`AGENT_CONSOLE_TERM_ID`).
    pub term_key: String,
    pub session_id: String,
}

/// Only TUI sessions carry a resume handle worth typing back into a terminal.
/// A CLI that doesn't report `kind` is accepted — absence is not evidence of a
/// non-interactive run.
fn is_interactive(agent: &LiveAgent) -> bool {
    agent.kind.as_deref().is_none_or(|k| k == "interactive")
}

/// Pure core: match live agents to terminals by process ancestry.
/// `parent_of` is injected so the whole rule set is testable against a fake
/// process tree, with no real processes involved.
pub fn match_agents_to_terms(
    agents: &[LiveAgent],
    terms: &[(String, u32)],
    parent_of: &dyn Fn(u32) -> Option<u32>,
) -> Vec<TermBinding> {
    let mut out = Vec::new();
    for (term_key, shell_pid) in terms {
        let mut matches = agents
            .iter()
            .filter(|a| is_interactive(a))
            .filter(|a| proc::descends_from(a.pid, *shell_pid, parent_of));
        // Exactly one, or nothing: `next()` twice is the cheapest way to say
        // "unique" without collecting.
        let Some(only) = matches.next() else { continue };
        if matches.next().is_some() {
            continue;
        }
        out.push(TermBinding {
            term_key: term_key.clone(),
            session_id: only.session_id.clone(),
        });
    }
    out
}

/// Ask the CLI which sessions are live and bind them to the terminals they
/// actually run in. Costs one short-lived `claude agents --json` per pass for
/// ALL terminals, so the caller can poll it without the cost scaling with the
/// number of open sessions.
pub fn reconcile(terms: &[(String, u32)]) -> Vec<TermBinding> {
    if terms.is_empty() {
        return Vec::new();
    }
    let agents = claude_cli::live_agents();
    if agents.is_empty() {
        return Vec::new();
    }
    match_agents_to_terms(&agents, terms, &proc::parent_of)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(pid: u32, id: &str) -> LiveAgent {
        LiveAgent {
            pid,
            session_id: id.to_string(),
            kind: Some("interactive".into()),
        }
    }

    /// Fake process tree: child → parent.
    fn tree(pairs: &'static [(u32, u32)]) -> impl Fn(u32) -> Option<u32> {
        move |pid| pairs.iter().find(|(c, _)| *c == pid).map(|(_, p)| *p)
    }

    #[test]
    fn binds_an_agent_to_the_terminal_whose_shell_it_descends_from() {
        // 100 = shell of term-a, 200 = shell of term-b; each has one agent, and
        // term-b's agent sits one wrapper deeper.
        let parent_of = tree(&[(101, 100), (201, 250), (250, 200)]);
        let agents = [agent(101, "sess-a"), agent(201, "sess-b")];
        let terms = [("term-a".to_string(), 100), ("term-b".to_string(), 200)];
        let bindings = match_agents_to_terms(&agents, &terms, &parent_of);
        assert_eq!(
            bindings,
            vec![
                TermBinding {
                    term_key: "term-a".into(),
                    session_id: "sess-a".into()
                },
                TermBinding {
                    term_key: "term-b".into(),
                    session_id: "sess-b".into()
                },
            ]
        );
    }

    #[test]
    fn an_unrelated_agent_binds_to_nothing() {
        // The user's own terminal outside the app: same machine, other tree.
        let parent_of = tree(&[(900, 800)]);
        let agents = [agent(900, "sess-elsewhere")];
        let terms = [("term-a".to_string(), 100)];
        assert!(match_agents_to_terms(&agents, &terms, &parent_of).is_empty());
    }

    #[test]
    fn two_agents_under_one_terminal_are_ambiguous_so_neither_binds() {
        // A nested agent (one spawned from inside the other) must never make us
        // pick: resuming the wrong conversation is worse than not resuming.
        let parent_of = tree(&[(101, 100), (102, 101)]);
        let agents = [agent(101, "outer"), agent(102, "inner")];
        let terms = [("term-a".to_string(), 100)];
        assert!(match_agents_to_terms(&agents, &terms, &parent_of).is_empty());
    }

    #[test]
    fn non_interactive_agents_are_ignored() {
        // A headless `claude -p` run from inside the terminal has no resume
        // handle we want to type back into it.
        let parent_of = tree(&[(101, 100), (102, 100)]);
        let agents = [
            agent(101, "tui"),
            LiveAgent {
                pid: 102,
                session_id: "headless".into(),
                kind: Some("print".into()),
            },
        ];
        let terms = [("term-a".to_string(), 100)];
        let bindings = match_agents_to_terms(&agents, &terms, &parent_of);
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].session_id, "tui");
    }

    #[test]
    fn a_cli_that_does_not_report_kind_still_binds() {
        let parent_of = tree(&[(101, 100)]);
        let agents = [LiveAgent {
            pid: 101,
            session_id: "sess".into(),
            kind: None,
        }];
        let terms = [("term-a".to_string(), 100)];
        assert_eq!(match_agents_to_terms(&agents, &terms, &parent_of).len(), 1);
    }

    #[test]
    fn an_unknown_parent_chain_binds_nothing() {
        // Platform without a parent probe (Windows): every lookup is None, so
        // the pass finds nothing instead of falling back to a guess.
        let agents = [agent(101, "sess")];
        let terms = [("term-a".to_string(), 100)];
        assert!(match_agents_to_terms(&agents, &terms, &|_| None).is_empty());
    }

    #[test]
    fn a_cycle_in_the_process_table_terminates() {
        let parent_of = tree(&[(101, 102), (102, 101)]);
        let agents = [agent(101, "sess")];
        let terms = [("term-a".to_string(), 100)];
        assert!(match_agents_to_terms(&agents, &terms, &parent_of).is_empty());
    }

    #[test]
    fn a_deep_chain_beyond_the_bound_does_not_bind() {
        // Guard rail, not a real shape: a chain longer than MAX_ANCESTOR_DEPTH
        // stops the walk rather than searching forever.
        let deep: Vec<(u32, u32)> = (1..40).map(|n| (1000 + n, 1000 + n - 1)).collect();
        let parent_of = move |pid: u32| deep.iter().find(|(c, _)| *c == pid).map(|(_, p)| *p);
        let agents = [agent(1039, "sess")];
        let terms = [("term-a".to_string(), 1000)];
        assert!(match_agents_to_terms(&agents, &terms, &parent_of).is_empty());
    }

    #[test]
    fn no_terminals_means_no_cli_call() {
        // `reconcile` must not spawn anything when there is nothing to bind —
        // it runs on a timer, and an idle app should stay idle.
        assert!(reconcile(&[]).is_empty());
    }
}
