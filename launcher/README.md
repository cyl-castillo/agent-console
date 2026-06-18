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

**Automatic.** The `Release` workflow publishes this launcher to npm on every
`vX.Y.Z` tag, in a `publish-launcher` job that runs *after* all platform build
jobs finish (so the assets the launcher fetches already exist). The job sets the
launcher version to the tag, then publishes — skipping cleanly if that version
is already on npm (e.g. a re-cut tag). So the normal flow is just: cut the app
release (`release-bump` → tag `vX.Y.Z`); the launcher ships itself.

Requirement: a repo secret `NPM_TOKEN` — an npm **automation** token for an
account with publish rights to the `@cyl-castillo` scope. Add it with:

```sh
gh secret set NPM_TOKEN   # paste the token when prompted
```

The committed `version` here is only a dev default (used when running the
launcher straight from the repo); CI overrides it from the tag at publish time.

**Manual fallback** (if you ever need to publish outside CI): from `launcher/`,
`npm login` then `npm publish --access public` (scoped packages are private by
default; `--access public` makes the first publish public).
