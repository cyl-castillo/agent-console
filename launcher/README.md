# @cyl-castillo/agent-console

Launcher for [Agent Console](https://github.com/cyl-castillo/agent-console) — a
minimalist AI-native desktop console for directing coding agents.

Agent Console is a native desktop app (Tauri: Rust + a webview), not a
JavaScript program. This package is a thin, dependency-free launcher: it
downloads the correct native build for your platform from the project's GitHub
Releases, caches it, and runs it.

## Use

```sh
# Run without installing anything globally:
npx @cyl-castillo/agent-console

# …or install the `agent-console` command globally:
npm install -g @cyl-castillo/agent-console
agent-console
```

First run downloads the native build (~80–100 MB) and caches it; later runs
launch from the cache instantly.

| Platform        | What it fetches & does                                   |
| --------------- | -------------------------------------------------------- |
| Linux x64       | Portable `.AppImage` — made executable and run directly  |
| macOS arm64/x64 | `.app` bundle — extracted, un-quarantined, opened        |
| Windows x64     | Installer (`-setup.exe`) — handed off to run             |

Other OS/arch combinations aren't prebuilt; the launcher points you to the
[Releases page](https://github.com/cyl-castillo/agent-console/releases).

### Options

```
agent-console --help       Show help
agent-console --version    Launcher version
agent-console --force      Re-download even if cached
agent-console --path       Print the cached artifact path and exit (no launch)
```

### Environment

- `AGENT_CONSOLE_VERSION` — release tag to fetch. Defaults to this package's
  version; set to `latest` to track the newest release, or `v0.30.0` to pin.

Cache location: `~/.cache/agent-console` (Linux), `~/Library/Caches/agent-console`
(macOS), `%LOCALAPPDATA%\agent-console` (Windows).

## Releasing (maintainers)

The launcher version mirrors the app version, and a launcher version only works
once the matching GitHub Release exists (it fetches assets from the tag of the
same version). So:

1. Cut the app release first (tag `vX.Y.Z`, wait for all platform assets to
   upload — see the repo's `release-bump` flow).
2. Bump `launcher/package.json` `version` to the same `X.Y.Z`.
3. From `launcher/`: `npm publish --access public` (scoped packages are private
   by default; `--access public` makes the first publish public).

Requires `npm login` (or an `NPM_TOKEN`) for the `@cyl-castillo` scope.
