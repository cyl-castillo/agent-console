import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

/// System notifications for the two moments that matter while you're in
/// another window: the agent needs you (approval) or finished a turn. Gated on
/// focus at the CALL SITE via `windowIsFocused()` — when you're already looking
/// at the app, an OS notification is just noise.

let granted: boolean | null = null;

async function ensurePermission(): Promise<boolean> {
  if (granted !== null) return granted;
  try {
    granted = await isPermissionGranted();
    if (!granted) {
      granted = (await requestPermission()) === "granted";
    }
  } catch {
    granted = false;
  }
  return granted;
}

export function windowIsFocused(): boolean {
  try {
    return document.hasFocus();
  } catch {
    return true; // fail closed: assume focused → no notification spam
  }
}

/// Fire-and-forget system notification. Errors are swallowed — a failed
/// notification must never break the flow that triggered it.
export function notify(title: string, body: string): void {
  void (async () => {
    if (!(await ensurePermission())) return;
    try {
      sendNotification({ title, body: body.slice(0, 180) });
    } catch {
      /* ignore */
    }
  })();
}
