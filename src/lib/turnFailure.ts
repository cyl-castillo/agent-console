/// Reading Claude's StopFailure reason.
///
/// Claude Code 2.1.78+ fires `StopFailure` instead of `Stop` when a turn ends
/// because the API refused it, and it names the reason with a small enum. Until
/// we listened for it the app had nothing: the turn just stopped producing
/// output, the reason scrolled past in the terminal, and an expired login was
/// indistinguishable from a slow model.
///
/// The enum is the CLI's, not ours, so this maps only what it documents today
/// and passes anything else through as "the API refused it" — a reason we don't
/// recognize is still a reason worth showing.
const REASONS: Record<string, string> = {
  authentication_failed: "the login was refused",
  oauth_org_not_allowed: "your organization doesn't allow this account",
  account_on_hold: "the account is on hold",
  billing_error: "billing was refused",
  rate_limit: "the usage limit was reached",
  overloaded: "the model was overloaded",
  invalid_request: "the request was rejected",
  model_not_found: "the model wasn't found",
  server_error: "the API returned a server error",
  max_output_tokens: "the response hit its output limit",
  unknown: "the API refused it",
};

/// Reasons a fresh interactive login actually fixes. `account_on_hold` and
/// `billing_error` are refusals about the account itself — pointing at the
/// login flow there would send the user somewhere that can't help.
const FIXED_BY_LOGIN = new Set(["authentication_failed", "oauth_org_not_allowed"]);

export interface TurnFailure {
  /// One sentence for a toast or an OS notification.
  message: string;
  /// Should we point at the "Fix Claude login" flow?
  needsLogin: boolean;
}

/// Turn a StopFailure event into something worth reading. `sessionName` is the
/// terminal's name when we could bind the event to one (the hook tags events
/// with AGENT_CONSOLE_TERM_ID), so several running agents don't blur together.
export function describeTurnFailure(
  error?: string | null,
  sessionName?: string | null,
): TurnFailure {
  const reason = (error && REASONS[error]) || REASONS.unknown;
  const who = sessionName ? `${sessionName}: the turn stopped` : "The turn stopped";
  const needsLogin = !!error && FIXED_BY_LOGIN.has(error);
  const remedy = needsLogin ? ' Run "Fix Claude login" from the command palette.' : "";
  return { message: `${who} — ${reason}.${remedy}`, needsLogin };
}
