// Runs once per Playwright invocation (not per worker): start every sweep
// from an empty output folder so missing-fixture / external-request logs
// only reflect this run.
import { mkdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export default function globalSetup() {
  const out = join(dirname(fileURLToPath(import.meta.url)), "out");
  rmSync(out, { recursive: true, force: true });
  mkdirSync(join(out, "a11y"), { recursive: true });
}
