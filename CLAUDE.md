# Agent Console — instructions for coding agents

Agent Console is a Tauri 2 desktop app (Rust backend in `src-tauri/`, React 19 +
TypeScript + Zustand frontend in `src/`) that runs coding agents (Claude Code,
Codex, OpenCode) and keeps a hash-chained ledger of what they did (Testigo).
The audience is teams that must answer "what did the agent do, and who approved
it". Every change is judged against that: correct, auditable, least privilege.

`AGENTS.md` mirrors this file for other engines. Change both together.

## Verify before you say done

Run what CI runs (`.github/workflows/ci.yml`), in this order, and report the
first failing line rather than a summary:

| Changed | Run |
| --- | --- |
| anything in `src/` | `npm run format:check && npm run lint && npm run build && npm test` |
| CSS or tokens | `npm run lint:css && npm run tokens && npm run check:contrast` |
| anything in `src-tauri/` | `cd src-tauri && cargo fmt --check && cargo clippy --workspace --lib --bins -- -D warnings && cargo test --workspace` |
| a `#[tauri::command]` | `scripts/check-ipc-sync.sh` |
| a panel, story or shell layout | `npm run build-storybook && npm run test:visual` |

Do not pipe a gate through `grep`/`tail` and chain with `&&`: the exit code
becomes the filter's and a failure passes silently.

GUI behaviour cannot be proven by these gates (WebKitGTK passes them and still
breaks on a real desktop). Say plainly "needs a human GUI check" and list what
to click; never claim a GUI change is verified.

## Contracts that span files

A change to one side without the others is a bug even when everything compiles.

- **Tauri command**: handler in `src-tauri/src/commands/<area>.rs` → registered
  in `generate_handler!` in `src-tauri/src/lib.rs` → typed wrapper in
  `src/ipc/tauri.ts` → shared shape in `src/types/domain.ts` → a fixture in
  `src/dev/fixtures/defaults.ts` (stories and visual tests run without Rust).
  Commands are `#[tauri::command(async)]` or `async fn`; a sync command needs a
  line in `scripts/ipc-sync-allowlist.txt` with the reason.
- **Action id**: `src/actions/ids.ts` + the palette registry in
  `src/stores/paletteStore.ts` + a row in `docs/ACTIONS.md`. Ids are a public
  contract (palette, voice, `window.__agentConsole`, the console MCP server):
  add, never rename or remove.
- **Ledger event kind**: emitted through `TestigoService` only → mapped in
  `src-tauri/src/services/aat_export.rs` (and the mapping table in the export
  README it writes) → shown in the Proof timeline. The ledger is append-only
  and hash-chained; never rewrite, reorder or delete records, and never write
  it from the frontend.
- **Engine parity**: a feature that talks to an agent states what it does for
  Claude, Codex and OpenCode (`src/agents/profiles.ts`,
  `src-tauri/src/services/engine_runner.rs`). "Not supported on X" is an
  acceptable answer only when written down in the PR and visible in the UI.

## Rules that are not negotiable

- **Secrets** live in the OS keychain (`vault_service`, `keyring`). Never in
  JSON under the data dir, never in logs, ledger payloads, error strings,
  fixtures or tests. Anything that leaves the machine goes through `redact`.
- **Anything derived from a model or a path that becomes a filename, argv or
  shell string is hostile**: sanitize with `fs_names`, confine with
  `path_guard`, pass long or free text over stdin, spawn through
  `services::proc` with an argument vector — never `sh -c` with interpolation.
- **Persisted state is crash-safe**: write to a temp file, rename, keep a
  `.bak`, tolerate a corrupt file by falling back (pattern:
  `sessions_service.rs`, skill `persisted-project-store`). Tests isolate the
  data dir with `XDG_DATA_HOME` in a single test fn.
- **Nothing listens beyond `127.0.0.1`** without a token and an explicit user
  action. New network egress is a product decision, not an implementation
  detail: call it out.
- **The human decides**: no auto-approve path that the user did not configure,
  no countdown that approves by itself, no retry of something an agent
  declined. A decision the console takes on someone's behalf is recorded in
  the ledger with its policy.
- **No model output rendered as raw HTML.** Agent prose goes through the
  allowlist in `src/lib/markdownPolicy.ts`; anything else is sanitized
  (DOMPurify) before it reaches the DOM.
- **UI** uses the primitives in `src/ui` and the generated tokens
  (`tokens/` → `npm run tokens`); no raw hex colours, no new global CSS
  classes, no `backdrop-filter`. Text must reach WCAG AA in both themes.
  Every interactive element is reachable by keyboard and has a name.

## Git

- Every commit is signed off (`git commit -s`); CI rejects the PR otherwise.
- Stage files by name. Never `git add -A` or `git add .`: the root holds
  untracked personal notes (`*-2026-*.md`, `reports/`, `research_notes/`).
- One change per PR; a second bug found on the way is a second PR.
- Never merge, tag or release on your own. Open the PR and report.
- Third-party GitHub Actions are pinned by commit SHA.

## Where things are

- `src/app/v2/` shell (rail, inbox, sessions, work, proof, settings) —
  the only UI. `src/ui/` primitives. `src/stores/` one Zustand store per area.
- `src-tauri/src/services/` one service per concern; `commands/` are thin.
- `src-tauri/hook-bridge/` the sidecar binary (agent hooks and the console
  MCP server); `npm run sidecar` builds it.
- `docs/` is the public site (agent-console.dev), including the mobile
  companion in `docs/app/`. `TESTIGO.md` is the ledger spec.
- `.claude/agents/` reviewers and `.claude/skills/enterprise-review/` the
  review procedure. Use them before proposing a release.
