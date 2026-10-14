// In-memory event bus standing in for `@tauri-apps/api/event`. Fixtures and
// stories push backend events (terminal output, hook status, approvals) into
// the same listeners the app registers in production.
export type UnlistenFn = () => void;
export interface Event<T> {
  event: string;
  id: number;
  payload: T;
}
type Handler = (e: Event<unknown>) => void;

const handlers = new Map<string, Set<Handler>>();
let nextId = 1;

export async function listen<T>(
  event: string,
  handler: (e: Event<T>) => void,
): Promise<UnlistenFn> {
  let set = handlers.get(event);
  if (!set) {
    set = new Set();
    handlers.set(event, set);
  }
  const h = handler as Handler;
  set.add(h);
  return () => {
    set?.delete(h);
  };
}

export async function once<T>(event: string, handler: (e: Event<T>) => void): Promise<UnlistenFn> {
  const off = await listen<T>(event, (e) => {
    off();
    handler(e);
  });
  return off;
}

export async function emit(event: string, payload?: unknown): Promise<void> {
  const set = handlers.get(event);
  if (!set) return;
  const e = { event, id: nextId++, payload };
  for (const h of [...set]) h(e);
}

export async function emitTo(_target: unknown, event: string, payload?: unknown): Promise<void> {
  return emit(event, payload);
}

/// Test hook: drop every listener (between stories).
export function resetEventBus(): void {
  handlers.clear();
}
