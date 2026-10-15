// Fixture registry behind the browser `invoke` shim. A fixture is either a
// plain value (returned by structured clone) or a function of the command's
// args. Scenarios layer overrides on top of the defaults; unknown commands
// resolve to `null` and are logged once so a missing fixture is visible in the
// story's console rather than a silent blank panel.
import { defaultFixtures } from "./defaults";

export type FixtureFn = (args: Record<string, unknown>) => unknown;
export type Fixture = FixtureFn | unknown;
export type FixtureMap = Record<string, Fixture>;

let active: FixtureMap = { ...defaultFixtures };
const warned = new Set<string>();
/// Every command invoked, in order — stories and tests assert on it.
export const invokeLog: { cmd: string; args: Record<string, unknown> }[] = [];

export function applyFixtures(overrides: FixtureMap = {}): void {
  active = { ...defaultFixtures, ...overrides };
  invokeLog.length = 0;
}

/// The current fixture for a command (specs read, extend and set it back).
export function getFixture(cmd: string): Fixture {
  return active[cmd];
}

/// Change one fixture at runtime (specs drive the backend side mid-story).
export function setFixture(cmd: string, value: Fixture): void {
  active[cmd] = value;
}

export function resetFixtures(): void {
  applyFixtures();
}

export async function invokeFixture(cmd: string, args: Record<string, unknown>): Promise<unknown> {
  invokeLog.push({ cmd, args });
  if (!(cmd in active)) {
    if (!warned.has(cmd)) {
      warned.add(cmd);
      console.warn(`[fixture] no fixture for "${cmd}" — returning null`);
    }
    return null;
  }
  const f = active[cmd];
  const value = typeof f === "function" ? await (f as FixtureFn)(args) : f;
  if (value instanceof Error) throw value;
  return value === undefined ? undefined : structuredClone(value);
}
