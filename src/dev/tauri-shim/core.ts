// Browser stand-in for `@tauri-apps/api/core`. Storybook and the visual tests
// alias the real package here (see .storybook/main.ts) so the whole frontend
// renders in a plain Chromium with fixture data instead of a Rust backend.
import { invokeFixture } from "../fixtures/registry";

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return (await invokeFixture(cmd, args ?? {})) as T;
}

export function convertFileSrc(path: string): string {
  return path;
}

export function isTauri(): boolean {
  return false;
}
