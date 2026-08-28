import type { ILinkHandler } from "@xterm/xterm";

/// OSC 8 hyperlinks in the terminal (`\x1b]8;;https://…\x07 label \x1b]8;;\x07`).
///
/// Agent CLIs emit these: Codex 0.150 renders markdown links as clickable
/// labels, and Claude marks up docs/PR URLs the same way. xterm parses them
/// with no help from us — but with no `linkHandler` configured its fallback is
/// a trap: a native `confirm("…WARNING: This link could potentially be
/// dangerous")` followed by `window.open()`, which the Tauri webview blocks.
/// The click reads as "scary dialog, then nothing happens".
///
/// So we own the behaviour: only http(s) survives, the target is shown before
/// the click, and it opens in the user's real browser — never inside the app's
/// own webview, which would navigate the console away from itself.

/// The URL a link is safe to open, or null. `new URL` is the parser (no regex
/// guessing), and the scheme allowlist is what keeps `javascript:`/`file:`
/// payloads written by a remote process out of the webview.
export function safeTerminalUrl(text: string): string | null {
  const trimmed = text.trim();
  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    return null;
  }
  return url.protocol === "http:" || url.protocol === "https:" ? trimmed : null;
}

/// What the hover hint needs to draw itself: the target, at the cursor.
export interface TerminalLinkHint {
  url: string;
  x: number;
  y: number;
}

export interface TerminalLinkDeps {
  /// Opens the URL outside the webview (Tauri's opener plugin in the app).
  open: (url: string) => Promise<unknown>;
  /// Called with the hint to show, or null to hide it.
  onHover: (hint: TerminalLinkHint | null) => void;
  /// A link the CLI marked up that we refuse to open (non-http scheme).
  onBlocked?: (text: string) => void;
  /// The open call itself failed (no browser, plugin error).
  onError?: (url: string) => void;
}

/// xterm's `linkHandler` for OSC 8 links. `allowNonHttpProtocols` stays off,
/// so xterm never even offers us a non-http link; `safeTerminalUrl` is the
/// second belt, because that default is xterm's to change, not ours.
export function createTerminalLinkHandler(deps: TerminalLinkDeps): ILinkHandler {
  return {
    activate(_event, text) {
      // The hint describes a link the user is no longer hovering the moment
      // the click lands; drop it first so it can't outlive the interaction.
      deps.onHover(null);
      const url = safeTerminalUrl(text);
      if (!url) {
        deps.onBlocked?.(text);
        return;
      }
      Promise.resolve(deps.open(url)).catch(() => deps.onError?.(url));
    },
    hover(event, text) {
      const url = safeTerminalUrl(text);
      deps.onHover(url ? { url, x: event.clientX, y: event.clientY } : null);
    },
    leave() {
      deps.onHover(null);
    },
  };
}
