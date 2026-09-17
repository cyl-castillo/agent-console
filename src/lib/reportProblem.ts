import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ipc } from "../ipc/tauri";

/// One-click "Report a problem": open a prefilled GitHub issue so a field
/// report costs the user seconds and still arrives with the diagnostics we
/// always need (version, platform, and — when launched from an error toast —
/// the exact error text). Every user becomes QA.

const NEW_ISSUE_URL = "https://github.com/cyl-castillo/agent-console/issues/new";

export interface ReportContext {
  version: string;
  userAgent: string;
  /// Error text when the report starts from an error toast.
  error?: string;
  /// The diagnostics bundle was copied to the clipboard before opening the
  /// issue; the template then asks for a paste instead of a description.
  diagnosticsOnClipboard?: boolean;
}

export function buildIssueUrl(ctx: ReportContext): string {
  const body = [
    "**What happened?**",
    "",
    ctx.error ? "```\n" + ctx.error + "\n```" : "<!-- describe the problem -->",
    "",
    "**Steps to reproduce**",
    "",
    "1. ",
    "",
    "**Expected**",
    "",
    "",
    "---",
    `- Agent Console: v${ctx.version || "unknown"}`,
    `- Platform: ${ctx.userAgent}`,
    ...(ctx.diagnosticsOnClipboard
      ? [
          "",
          "**Diagnostics** — already on your clipboard (About → Copy diagnostics). Paste below:",
          "",
        ]
      : []),
  ].join("\n");
  const params = new URLSearchParams({
    title: ctx.error ? `[bug] ${ctx.error.slice(0, 80)}` : "[bug] ",
    labels: "bug,field-report",
    body,
  });
  return `${NEW_ISSUE_URL}?${params.toString()}`;
}

/// Put the diagnostics bundle on the clipboard so the report can carry it
/// (it is far too large for a URL). Returns whether it got there — callers
/// word the issue template accordingly.
async function copyDiagnostics(): Promise<boolean> {
  try {
    const text = await ipc.diagnosticsBundle();
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
    await writeText(text);
    return true;
  } catch {
    return false;
  }
}

export async function reportProblem(error?: string): Promise<void> {
  const version = await getVersion().catch(() => "");
  const diagnosticsOnClipboard = await copyDiagnostics();
  const url = buildIssueUrl({
    version,
    userAgent: navigator.userAgent,
    error,
    diagnosticsOnClipboard,
  });
  try {
    await openUrl(url);
  } catch {
    // Opener unavailable (permission/platform edge) — let the webview try.
    window.open(url, "_blank", "noopener");
  }
}
