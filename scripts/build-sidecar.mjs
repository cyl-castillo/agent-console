// Build the hook-bridge sidecar and place it where Tauri's externalBin
// expects it: src-tauri/binaries/hook-bridge-<target-triple>[.exe].
//
// Runs as part of beforeDevCommand/beforeBuildCommand. Tauri exports
// TAURI_ENV_TARGET_TRIPLE for cross builds (macOS aarch64/x64 on one runner);
// without it we build for the host and read the triple from `rustc -vV`.
// Node here is a BUILD-time tool only — the whole point of the sidecar is
// that end users don't need node at runtime.

import { execSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const triple =
  process.env.TAURI_ENV_TARGET_TRIPLE ||
  /host: (\S+)/.exec(execSync("rustc -vV", { encoding: "utf8" }))[1];

const explicitTarget = Boolean(process.env.TAURI_ENV_TARGET_TRIPLE);
const args = ["build", "-p", "hook-bridge", "--release"];
if (explicitTarget) args.push("--target", triple);

console.log(`[sidecar] cargo ${args.join(" ")}`);
execSync(`cargo ${args.join(" ")}`, { cwd: "src-tauri", stdio: "inherit" });

const exe = triple.includes("windows") ? ".exe" : "";
const built = join(
  "src-tauri",
  "target",
  ...(explicitTarget ? [triple] : []),
  "release",
  `hook-bridge${exe}`,
);
const destDir = join("src-tauri", "binaries");
mkdirSync(destDir, { recursive: true });
const dest = join(destDir, `hook-bridge-${triple}${exe}`);
copyFileSync(built, dest);
console.log(`[sidecar] ${built} -> ${dest}`);
