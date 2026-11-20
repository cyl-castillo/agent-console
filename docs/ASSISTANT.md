# Assistant (experimental)

Ask the console how it works, or tell it where to go, in your own words:
"where do I see what the agent did in the last turn?", "open the ledger",
"¿qué sesiones necesitan mi atención?". One optional agent answers and moves
around the app for you.

You can type to it or talk to it. The module is **off by default** and nothing
else in the console depends on it.

## Switch it on

Command palette → **Configure Modules** → **Assistant**. The panel appears
under **Work → Assistant**, and as the action `nav.assistant`.

## Talk to it

With voice mode on (Ctrl+Shift+V — see the README's Voice section; speech is
transcribed on this machine):

- **"Asistente, …" / "Assistant, …"** from anywhere, on the usual push-to-talk
  (hold Ctrl+Space). Everything after the word goes to the assistant in your
  own words. The word alone still just opens the panel.
- **Hold to talk** in the panel: the whole take goes to the assistant, with no
  command grammar in between.

The panel comes forward with the full answer and the console says its first
two sentences aloud. Take the mic again and it stops talking.

Speaking grants nothing typing does not: it is the same confined assistant,
and the fixed voice commands ("mandá", "sí", "abrí changes") keep working
without a model, exactly as before. With the module off, "asistente" is an
ordinary word again.

## What it is

- **Your own agent, not a new one.** Each question runs one headless turn of
  the project's default engine (Claude Code, Codex or OpenCode), read-only.
  It spends tokens from your plan like any other turn; the panel shows the
  running total. There is no extra account, key or network destination.
- **It knows the product.** The first turn carries a short guide to the
  console that ships inside the app; for anything about the current state it
  looks instead of guessing.
- **It acts through the console's own MCP server**, under its own identity
  (`agent:console-assistant`), never as you.

## What it may do

| It can | It cannot |
| --- | --- |
| Read where the console is, the sessions and their attention state | Type into a session or start one |
| List the actions and run the harmless ones (navigation, panels, refresh) | Run an action that kills, deletes, pushes or spends |
| Read a session's witnessed events and verification verdict | Approve, deny, review or decline anything |
| Verify the ledger's hash chain, search the project's memory | Edit files or run commands |

The limit is enforced in the app, not in the prompt: the assistant's identity
is confined to a fixed subset of the console's tools
(`assistant_service::TOOLS`), and `invoke_action` keeps the allow-list every
MCP agent has (`console_mcp::action_allowed`). When you ask for something it
cannot do, it says where you do it and takes you there.

## What the ledger records

- Every tool call the assistant makes, as `agent_action` under
  `agent:console-assistant`, with the action id it ran.
- That a turn ran and how it ended (`assistant_turn`). **Not your words**: a
  question can carry a secret, and the ledger is exported.

The conversation itself lives in memory and is gone when you close the app or
start a new one.

## Engines

| Engine | Tools attached through |
| --- | --- |
| Claude Code | `--mcp-config` plus an allow-list for the console server |
| Codex | a `-c mcp_servers.agent_console=…` override |
| OpenCode | `session/new` over ACP. On the legacy `opencode run` fallback the server cannot be attached: the assistant answers from its guide and the panel says so |

## Learning log (opt-in)

The **Learning** tab of the panel has one switch: *Keep a learning log of what
I ask*. It is **off by default**.

When it is on, each turn is appended to a file on this machine
(`<local data dir>/agent-console/assistant-log/<project>.jsonl`, the panel
shows the exact path): the question, the answer, the console tools it called,
whether it failed, the engine, the tokens, whether you spoke it, and your
rating. Every text goes through the same secret redaction as the ledger's
exports before it is written. Turns that fail are kept too — they are the
clearest sign of a gap.

- **Rate an answer** with *Helpful* / *Not helpful* under it (only replies
  that were logged can be rated).
- **Propose improvements** runs one turn of your own engine, with no console
  tools, over the last 80 turns of the log and returns up to eight gaps: in
  the product, in the assistant's guide, a missing tool, or voice. They are
  proposals to read. Nothing is filed, sent or changed.
- **Delete the log** removes the file and its proposals.

The log is **not the ledger** and is never exported with it: the ledger still
records only that a turn (`assistant_turn`) or a review of the log
(`assistant_propose`) ran, never your words. Nothing in the log leaves the
machine except as the prompt of that one turn of the engine you already use.

It is also the data that will tell, later, whether a model trained on how this
console is used is worth building. Nothing is trained today.

## Not in this phase

Stopping a turn in flight, a conversation that survives a restart, acting on a
session (sending a prompt), and applying a proposal automatically.
