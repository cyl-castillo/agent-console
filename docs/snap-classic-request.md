# Snap Store: classic confinement request

Status: **draft — not yet submitted**. Tracks the two store-side steps that
only the account owner can do, plus the forum post text ready to paste.

## Why classic confinement

Agent Console's entire purpose is orchestrating binaries the *user* installed
on the host: it spawns the user's login shell in a PTY, runs `claude` /
`codex` / `node` / `git` from the host PATH inside arbitrary project
directories, and shares `~/.claude` state (settings.json hooks, session
transcripts) with the Claude Code CLI. Strict confinement cannot execute host
binaries — under strict the app is an empty shell. This is the same category
as the approved classic dev-tool snaps (`code`, `goland`, `clion`, …).

## Owner checklist (Carlos)

1. **Login**: `snap install snapcraft --classic` then `snapcraft login`
   (Ubuntu One account).
2. **Register the name**: `snapcraft register agent-console`.
3. **Request classic**: post the text below at
   <https://forum.snapcraft.io/c/store-requests> (title:
   `Classic confinement request: agent-console`). Reviews typically take days
   to a few weeks; they may ask follow-up questions on the thread.
4. **Once approved** — create the CI credential:
   `snapcraft export-login --acls package_access,package_push,package_update,package_release -`
   and save the output as the repo secret `SNAPCRAFT_STORE_CREDENTIALS`.
5. **First publish**: run the *Snap* workflow (Actions → Snap) with the
   current release tag and `publish: true`. Validate the build result on a
   real machine first (`snap install --classic --dangerous ./*.snap`) — the
   snapcraft.yaml is untested until then.

## Forum post (paste as-is)

---

**Title:** Classic confinement request: agent-console

Hello! I'm requesting classic confinement for `agent-console`
(<https://github.com/cyl-castillo/agent-console>, MIT licensed).

Agent Console is a desktop developer tool (Tauri/WebKitGTK): a minimalist
console for supervising AI coding agents (Claude Code, Codex CLIs) working
inside the user's own repositories. It embeds a PTY terminal that runs the
user's login shell, and its core function is launching and supervising
CLIs the user has installed on the host — `claude`, `codex`, `node`, `git` —
in arbitrary project directories chosen by the user. It also reads and
writes `~/.claude` (the Claude CLI's own config/hooks/session state), which
must be the same files the host CLI uses.

Why strict confinement is not viable:

- The app's primary function is executing host-installed binaries
  (`claude`, `codex`, the user's shell, `git`, `node`) — not bundleable:
  they are user-installed, user-updated tools (e.g. the Claude CLI updates
  through npm and carries the user's authentication).
- It must operate on arbitrary project directories the user opens (like an
  IDE), and share dotfile state (`~/.claude`, `~/.codex`) with those host
  tools.

This matches the supported classic category of IDEs / developer tools that
orchestrate host toolchains (`code`, `goland`, etc.).

The snap repacks the same .deb we publish on GitHub Releases (build
reproducible from CI: `.github/workflows/snap.yml` in the repo).

Thanks!

---

## Notes

- The snap build (`snap/snapcraft.yaml`) repacks the released `.deb` — no
  separate compilation, so the snap can't drift from the GitHub release.
- The in-app Tauri updater must eventually be disabled for snap builds
  (snapd auto-refreshes; two updaters would fight). Tracked as a follow-up:
  detect `$SNAP` at runtime and hide the update UI.
- `snapcraft.yaml` is **not yet build-validated** (authored on a machine
  without snapcraft/lxd). The first workflow run + a `--dangerous` local
  install is the validation gate before any store publish.
