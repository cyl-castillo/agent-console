# Agent Console

A minimalist, AI-native console for directing agents inside a repository.

Not an IDE. Not a chat client. A small terminal-first desktop app that pairs the Claude Code CLI with an integrated shell, a git diff viewer, and per-turn snapshots — so you can drive an agent through real work without losing control.

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

## What's in v0.1

- Open any local repository (folder picker, recents list).
- Built-in PTY terminal that runs `npm test`, `vim`, `git status`, anything.
- Git diff viewer with badges (M/A/D/U), per-file revert, revert-all.
- Chat panel powered by `claude -p` running inside the repo with full tool access.
- **Per-tool approval modal** for `Bash`, `Edit`, `Write`, `MultiEdit`, `NotebookEdit` via a PreToolUse hook. Safe reads (`Read`, `Grep`, `Glob`, `LS`, `WebFetch`, `WebSearch`) pass through.
- **Per-turn snapshot** of the working tree (captures tracked + untracked), with one-click restore from the user message bubble.
- **Auto-switch** to Changes tab the first time the agent mutates files in a turn.
- Markdown rendering for assistant text (headings, lists, code blocks, inline code).
- Keyboard shortcuts (terminal-first navigation).
- Persisted "recent projects" list across launches.

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

Near term:

- Per-tool decision history ("show me everything claude has done this session")
- Multi-terminal tabs
- Markdown click-to-insert from chat (`open the file referenced in this block`)
- File viewer pane when clicking a file in the tree
- Watchdog on agent CPU/cost runaway

Maybe:

- Multi-agent panes (sub-agents in parallel)
- Worktree management — branch off + agent per worktree
- MCP server configuration UI
- GitHub issue/PR integration

Not on the list:

- Code editing UI (use your editor)
- VS Code-style extension marketplace
- Cloud sync of conversations

## Status

**Early preview · v0.1.** Single user, local only, no telemetry, no auto-update. Suitable for daily personal use and demos. Not packaged for distribution channels (Snap/Flatpak/Homebrew/MS Store) yet. APIs and event names may still change between minor releases.

## Support

If Agent Console is useful to you and you'd like to help shape where it goes, you can sponsor the project. Sponsorships fund focused time on the roadmap above — more tools, better safety, broader platforms.

- [GitHub Sponsors](https://github.com/sponsors/cyl-castillo)
- [Buy Me a Coffee](https://www.buymeacoffee.com/cylcastillo)

Non-financial support also counts: a ⭐ on the repo, a thoughtful issue, a workflow shared in [Discussions](https://github.com/cyl-castillo/agent-console/discussions).

## License

MIT — see [LICENSE](./LICENSE).
