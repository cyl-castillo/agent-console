# Agent Console

[![Testigo: intent-to-proof](https://img.shields.io/badge/Testigo-intent--to--proof-2ea44f?logo=github)](https://github.com/cyl-castillo/testigo)

A minimalist, AI-native console for directing agents inside a repository.

Not an IDE. Not a chat client. A small terminal-first desktop app that pairs the Claude Code CLI with an integrated shell, a git diff viewer, and per-turn snapshots — so you can drive an agent through real work without losing control.

Every session is witnessed: agent-console is the reference implementation of
[**Testigo**](https://github.com/cyl-castillo/testigo), an open intent-to-proof
protocol — prompts, human approvals, tool results and per-turn diffs land in a
hash-chained evidence ledger, exportable as signed proof packets anyone can
verify in a browser.

```
┌──────────────────────────────────────────────────────────────────────┐
│ my-repo · javascript · vite · /home/me/code/my-repo                  │
├──────────┬─────────────────────────────────────┬─────────────────────┤
│ Files    │ [Terminal] [Changes (3)]            │ Agent               │
│          │ ─────────────────────────────────── │ ─────────────────── │
│ src/     │ $ npm test                          │ you ↶restore        │
│  api.ts  │ ✓ 14 passed                         │ "fix the failing    │
│  app.ts  │                                     │  user-auth test"    │
│ tests/   │                                     │                     │
│ README   │                                     │ ▸ Read tests/auth   │
│          │                                     │ ✓ Read tests/auth   │
│          │                                     │ ✓ Edit src/api.ts   │
│          │                                     │ ✓ Bash: npm test    │
│          │                                     │                     │
│          │                                     │ claude: fixed —     │
│          │                                     │ token was expired   │
│          │                                     │                     │
│          │                                     │ [type a task…]      │
└──────────┴─────────────────────────────────────┴─────────────────────┘
```

## Vision

Agent Console is **not trying to replace VS Code**. It is designed as an **AI-native development console** focused on supervising coding agents through terminal, diffs, and controlled workflows.

The center of the app is the chat, the terminal, and the git diff — not a code editor. You don't read code here; you direct the agent and verify its work.

Principles:

- **Terminal-first, agent-first, diff-first.**
- **Lightweight** — Tauri + Rust, not Electron.
- **Cross-platform** by design (Linux/macOS/Windows), Linux-first in practice.
- **Human control mandatory** before risky actions.
- **Local-first, fast, developer-centric.** No accounts, no cloud sync, no marketplace.

## The core

The center of the screen is deliberately small, and it hasn't grown since v0.1:

- Open any local repository; a built-in PTY terminal that auto-runs your agent
  (Claude Code or Codex, per session) and also runs `npm test`, `vim`, anything.
- Git diff viewer with per-file revert; per-turn working-tree snapshots with
  one-click restore.
- **Per-tool approval modal** with keyboard flow, live countdown, and durable
  audit trail — the human decides before anything risky runs.
- Multiple sessions per project (isolated worktrees optional), resumable across
  restarts, with the agent conversation rebound automatically.

## The workbench (the control plane)

Around that core sits a side panel of nine workbench tabs — all local, all
suggest-only, none of them able to block the core:

- **Tasks** — your Jira queue as one-click seeded agent sessions, plus an
  agenda of due dates and scheduled runs.
- **Proof** — every session witnessed into a hash-chained evidence ledger
  ([Testigo](https://github.com/cyl-castillo/testigo)); export signed proof
  packets anyone verifies in a browser.
- **Coach** — the agent's playbook: skills generated from your project
  (Advisor), curated from your actual usage (Learning).
- **Room** — you plus N agents (mixed engines) on one problem; working rooms
  edit on an isolated branch you review and merge (see below).
- **Schedule** — agentic jobs on a clock, physically restricted to plan mode.
- **Trust** — permissions + secrets vault: what the agent can touch.
- **Add-ons** — plugins + MCP servers.
- **Notes** and **Context** (CLAUDE.md & memories); export/import and voice
  input (push-to-talk + spoken approvals) live in the command palette.

## Rooms & cowork

A **room** is a shared conversation about one problem between you and N agents
(Claude and/or Codex). In a **working room** (the "let them edit" toggle) the
agents edit code in an isolated git worktree on a `room/<id>` branch, committing
one checkpoint per turn — your own files stay untouched until you review and merge.

**Cowork with human colleagues** happens over the git remote you already use — no
realtime sync, no extra infra. From a live working room:

- **Share / open MR** — pushes the `room/<id>` branch to the remote (with the full
  conversation committed alongside as `.room/<id>.md`, so reviewers see the diff
  *and* the reasoning) and hands back a ready-to-open MR/PR link (GitHub/GitLab,
  SSH or HTTPS).
- **Sync colleague work** — fetches that branch and merges a colleague's commits
  back into the live worktree so the next turn builds on top. It refuses on a dirty
  worktree and aborts cleanly on conflict (reporting the files), so the
  auto-committing turn loop never runs on a half-merged tree.

The loop: room produces work → **Share** (out for review, with context) → colleague
reviews/extends on the platform → **Sync** (back in) → next turn continues. If a
colleague pushed first, Share tells you to Sync, then Share again.

A room **reopened in a later session** re-attaches its live worktree from the
surviving `room/<id>` branch, so both **Share** and **Sync** work again — the full
cowork loop survives across sessions. If the worktree can't be remounted (e.g. the
branch was deleted), the room comes back read-only but **Share** still works by
branch name; you'll see a one-line notice.

## Installing

- **Linux**: AppImage / `.deb` / `.rpm` from
  [Releases](https://github.com/cyl-castillo/agent-console/releases/latest), or
  on Arch: `yay -S agent-console-bin`.
- **Windows**: the `-setup.exe` from Releases. SmartScreen may warn about an
  unknown publisher (the binaries aren't code-signed yet) — "More info → Run
  anyway".
- **macOS**: download the `.dmg`, drag the app to Applications, then **one
  extra step** — the app isn't notarized by Apple yet, so Gatekeeper blocks it
  with a misleading *"Agent Console is damaged and can't be opened"* message.
  It isn't damaged. Either run:

  ```bash
  xattr -cr "/Applications/Agent Console.app"
  ```

  and open it normally, or try to open it once and then allow it under
  **System Settings → Privacy & Security → "Open Anyway"**. This is needed
  only on first launch; updates keep working normally afterwards.

## Requirements

- **Claude Code CLI** installed and authenticated:
  `npm install -g @anthropic-ai/claude-code` then `claude` once to log in.
- **Node.js ≥ 20** (the bundled PreToolUse hook is a Node script).
- **git** ≥ 2.30.
- Rust toolchain (only for building from source).
- Linux: `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libsoup-3.0-dev`, `librsvg2-dev`, `libayatana-appindicator3-dev`, `libjavascriptcoregtk-4.1-dev`, `libssl-dev`.

## Development

```bash
# Install JS deps
npm install

# Run in dev (Vite + Tauri side-by-side)
npm run tauri dev

# Type-check frontend
npx tsc --noEmit

# Cargo check backend
cd src-tauri && cargo check
```

The dev process spawns the Rust app, opens a Tauri window, and hot-reloads the React side via Vite. First build downloads ~250 crates and takes 2–4 minutes; subsequent runs are seconds.

## Building a release

```bash
npm run tauri build
```

Output on Linux (Ubuntu 24.04):

```
src-tauri/target/release/bundle/
├── deb/Agent Console_0.1.0_amd64.deb       # install with: sudo apt install ./*.deb
├── appimage/agent-console_0.1.0_amd64.AppImage
└── rpm/Agent Console-0.1.0-1.x86_64.rpm
```

The `.deb` declares `nodejs` as a runtime dep (needed for the PreToolUse hook). On other distros, AppImage works as a self-contained binary.

macOS and Windows builds use the same command; bundle output varies (`.dmg` / `.msi`).

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+1` | Switch to Terminal tab |
| `Ctrl+2` | Switch to Changes tab |
| `Ctrl+3` | Switch to Preview tab |
| `Ctrl+B` | Toggle workspace sidebar |
| `Ctrl+J` | Toggle side panel |
| `Ctrl+P` | Open command palette |
| `Ctrl+/` | Show keyboard shortcuts |
| `Ctrl+T` | New session (passes through to the shell when focused inside the terminal) |
| `Ctrl+Tab` | Next live session |
| `Ctrl+Shift+Tab` | Previous live session |
| `Ctrl+]` | Next live session (passes through to the shell when focused inside the terminal) |
| `Ctrl+[` | Previous live session (passes through as `Esc` when focused inside the terminal — e.g. vim) |
| `Ctrl+W` | Close active session (passes through as delete-word when focused inside the terminal) |
| `Ctrl+K` | Focus the chat input |
| `Ctrl+L` | Clear the terminal (only when not focused inside terminal — the shell still gets `Ctrl+L` natively there) |
| `Ctrl+R` | Refresh git changes |
| `Esc`    | Deny pending agent action / dismiss modal |
| `Ctrl+Enter` | Approve pending agent action |
| `Enter`  | Send chat message |
| `Shift+Enter` | New line in chat input |

## Safety model

Two layers, both on by default:

1. **Per-tool approval (front line).**
   Before the agent runs `Edit`/`Write`/`Bash`/etc., a modal shows the tool name + input. You approve or deny. A session-wide "approve all" toggle is available if you trust the current task.

2. **Per-turn snapshot (safety net).**
   Right before each user message is dispatched, the app builds a git commit capturing the full working tree (including untracked files) under `refs/agent-console/<turn-id>`. If a turn goes sideways, click `↶ restore` next to your message — it `read-tree --reset -u`s the working tree back to that state. HEAD is never moved; your branches are untouched.

For non-git repos, only layer 1 applies — there is no snapshot to make.

## Architecture

```
┌─────────────────────────────────────────────┐
│ React + TS + Zustand                        │
│  ┌─────────┬─────────────┬─────────────┐    │
│  │FileTree │Terminal/Diff│  AgentChat  │    │
│  └─────────┴─────────────┴─────────────┘    │
│            ▲                                │
│            │ invoke() + events              │
└────────────┼────────────────────────────────┘
┌────────────┼────────────────────────────────┐
│ Rust (Tauri 2)                              │
│            ▼                                │
│  ┌──────────────┐  ┌─────────────────────┐ │
│  │project_mgr   │  │terminal_runner (PTY)│ │
│  ├──────────────┤  ├─────────────────────┤ │
│  │git_service   │  │snapshot_service     │ │
│  ├──────────────┤  ├─────────────────────┤ │
│  │projects_svc  │  │permission_bridge    │ │
│  └──────────────┘  └─────────────────────┘ │
│  ┌─────────────────────────────────────┐    │
│  │agent_session                         │   │
│  │  spawns: claude -p                   │   │
│  │   --input/output-format stream-json  │   │
│  │   --permission-mode default          │   │
│  │   --settings <hooks.json>            │   │
│  │   env AGENT_CONSOLE_HOOK_DIR=...     │   │
│  └─────────────────────────────────────┘   │
└─────────────────────────────────────────────┘
        │
        ▼
   PreToolUse hook (bundled Node script)
        │
        ▼  req-*.json / res-*.json
   shared dir polled by permission_bridge
```

## Roadmap

Shipped from earlier roadmaps: per-tool decision history (the Proof ledger),
multi-terminal sessions, worktree management, multi-agent rooms, MCP
configuration UI, Jira integration, auto-update, workbench consolidation
(16 tabs → 9 groups).

Near term:

- File viewer pane when clicking a file in the tree
- Watchdog on agent CPU/cost runaway
- Testigo protocol v0.2 items (see [TESTIGO.md](./TESTIGO.md))

Not on the list:

- Code editing UI (use your editor)
- VS Code-style extension marketplace
- Cloud sync of conversations

## Status

**Early preview**, releasing continuously (see
[releases](https://github.com/cyl-castillo/agent-console/releases)) with
auto-update on all three platforms (on Linux, the AppImage self-updates;
deb/rpm get a download prompt). Single user, local only, no telemetry. Not in
distribution channels (Snap/Flatpak/Homebrew/MS Store) yet. APIs and event
names may still change between minor releases.

## Support

If Agent Console is useful to you and you'd like to help shape where it goes, you can sponsor the project. Sponsorships fund focused time on the roadmap above — more tools, better safety, broader platforms.

- [GitHub Sponsors](https://github.com/sponsors/cyl-castillo)
- [Buy Me a Coffee](https://www.buymeacoffee.com/cylcastillo)

Non-financial support also counts: a ⭐ on the repo, a thoughtful issue, a workflow shared in [Discussions](https://github.com/cyl-castillo/agent-console/discussions).

## License

MIT — see [LICENSE](./LICENSE).
