#!/usr/bin/env node
// Agent Console launcher.
//
// Agent Console is a Tauri desktop app: a native binary per platform, not a JS
// package. This launcher is the thin npm front door — `npx @cyl-castillo/agent-
// console` (or a global install) downloads the right native artifact for the
// current OS/arch from the matching GitHub Release, caches it, and launches it.
// Pure Node, zero dependencies, so npx stays fast and there's nothing to audit.

import { spawn, spawnSync } from "node:child_process";
import { createWriteStream } from "node:fs";
import { chmod, mkdir, readFile, rm, stat } from "node:fs/promises";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO = "cyl-castillo/agent-console";

const here = path.dirname(fileURLToPath(import.meta.url));
const pkg = JSON.parse(await readFile(path.join(here, "..", "package.json"), "utf8"));

// The release tag tracks the launcher's own version, so installing a pinned
// launcher version gets that exact app build. AGENT_CONSOLE_VERSION overrides
// it (e.g. "latest" or "v0.30.0") for testing.
const versionArg = process.env.AGENT_CONSOLE_VERSION || pkg.version;
const tag = versionArg === "latest"
  ? "latest"
  : versionArg.startsWith("v") ? versionArg : `v${versionArg}`;

// Per-platform artifact selection. Linux ships a portable AppImage we can run
// directly; macOS ships a .app bundle we extract and open; Windows ships an
// installer we hand off to. Keyed by `${process.platform}-${process.arch}`.
function selectArtifact(version) {
  const v = version.replace(/^v/, "");
  const key = `${process.platform}-${process.arch}`;
  switch (key) {
    case "linux-x64":
      return { kind: "appimage", asset: `Agent.Console_${v}_amd64.AppImage` };
    case "darwin-arm64":
      return { kind: "app", asset: `Agent.Console_aarch64.app.tar.gz` };
    case "darwin-x64":
      return { kind: "app", asset: `Agent.Console_x64.app.tar.gz` };
    case "win32-x64":
      return { kind: "installer", asset: `Agent.Console_${v}_x64-setup.exe` };
    default:
      return null;
  }
}

function cacheDir() {
  const base = process.platform === "win32"
    ? (process.env.LOCALAPPDATA || path.join(os.homedir(), "AppData", "Local"))
    : process.platform === "darwin"
      ? path.join(os.homedir(), "Library", "Caches")
      : (process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache"));
  return path.join(base, "agent-console");
}

function log(msg) {
  process.stderr.write(`${msg}\n`);
}

function die(msg) {
  log(`agent-console: ${msg}`);
  process.exit(1);
}

async function exists(p) {
  try { await stat(p); return true; } catch { return false; }
}

// Resolve "latest" to a concrete tag so the cache key and asset URLs are stable.
async function resolveTag(t) {
  if (t !== "latest") return t;
  const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
    headers: { "user-agent": "agent-console-launcher", accept: "application/vnd.github+json" },
  });
  if (!res.ok) die(`could not resolve latest release (HTTP ${res.status})`);
  const body = await res.json();
  if (!body.tag_name) die("latest release has no tag");
  return body.tag_name;
}

async function download(url, dest) {
  const res = await fetch(url, {
    headers: { "user-agent": "agent-console-launcher", accept: "application/octet-stream" },
    redirect: "follow",
  });
  if (!res.ok || !res.body) {
    die(`download failed (HTTP ${res.status}) for ${url}`);
  }
  const total = Number(res.headers.get("content-length")) || 0;
  let seen = 0;
  let lastPct = -1;
  const body = Readable.fromWeb(res.body);
  body.on("data", (chunk) => {
    if (!total) return;
    seen += chunk.length;
    const pct = Math.floor((seen / total) * 100);
    if (pct !== lastPct && pct % 5 === 0) {
      lastPct = pct;
      process.stderr.write(`\rDownloading… ${pct}%`);
    }
  });
  const partial = `${dest}.part`;
  await pipeline(body, createWriteStream(partial));
  if (total) process.stderr.write("\rDownloading… 100%\n");
  // Atomic-ish: only put the final name in place once the bytes are all there.
  const { rename } = await import("node:fs/promises");
  await rename(partial, dest);
}

function run(cmd, args, opts = {}) {
  const child = spawn(cmd, args, { stdio: "inherit", ...opts });
  child.on("exit", (code) => process.exit(code ?? 0));
  child.on("error", (err) => die(`failed to launch: ${err.message}`));
}

async function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--help") || argv.includes("-h")) {
    process.stdout.write(
      `Agent Console launcher\n\n` +
      `Usage: agent-console [options]\n\n` +
      `  -h, --help        Show this help\n` +
      `  -v, --version     Show launcher version\n` +
      `      --force       Re-download even if cached\n` +
      `      --path        Print the cached artifact path and exit (no launch)\n\n` +
      `Env:\n` +
      `  AGENT_CONSOLE_VERSION   Release tag to fetch (default: ${pkg.version}; "latest" allowed)\n`,
    );
    return;
  }
  if (argv.includes("--version") || argv.includes("-v")) {
    process.stdout.write(`${pkg.version}\n`);
    return;
  }
  const force = argv.includes("--force");
  const printPath = argv.includes("--path");

  const resolvedTag = await resolveTag(tag);
  const artifact = selectArtifact(resolvedTag);
  if (!artifact) {
    die(
      `no prebuilt artifact for ${process.platform}/${process.arch}. ` +
      `See https://github.com/${REPO}/releases/${resolvedTag === "latest" ? "latest" : `tag/${resolvedTag}`}`,
    );
  }

  const dir = path.join(cacheDir(), resolvedTag);
  await mkdir(dir, { recursive: true });
  const dest = path.join(dir, artifact.asset);
  const url = `https://github.com/${REPO}/releases/download/${resolvedTag}/${artifact.asset}`;

  if (force && await exists(dest)) await rm(dest, { force: true });
  if (!await exists(dest)) {
    log(`Fetching Agent Console ${resolvedTag} (${process.platform}/${process.arch})…`);
    await download(url, dest);
  }

  if (artifact.kind === "appimage") {
    await chmod(dest, 0o755);
    if (printPath) { process.stdout.write(`${dest}\n`); return; }
    run(dest, argv.filter((a) => !a.startsWith("--")));
    return;
  }

  if (artifact.kind === "app") {
    // Extract the .app bundle next to the tarball (cached), then `open` it.
    const appDir = path.join(dir, "app");
    const appBundle = path.join(appDir, "Agent Console.app");
    if (force || !await exists(appBundle)) {
      await mkdir(appDir, { recursive: true });
      const r = spawnSync("tar", ["-xzf", dest, "-C", appDir], { stdio: "inherit" });
      if (r.status !== 0) die("failed to extract the .app bundle");
      // The download isn't notarized; clear quarantine so Gatekeeper opens it.
      spawnSync("xattr", ["-dr", "com.apple.quarantine", appBundle], { stdio: "ignore" });
    }
    if (printPath) { process.stdout.write(`${appBundle}\n`); return; }
    run("open", [appBundle]);
    return;
  }

  if (artifact.kind === "installer") {
    if (printPath) { process.stdout.write(`${dest}\n`); return; }
    log("Launching the installer…");
    run(dest, []);
    return;
  }
}

main().catch((err) => die(err?.message || String(err)));
