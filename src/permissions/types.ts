// Permission rules — internal representation. The persisted form in
// settings.json is the `raw` string under permissions.{allow,deny,ask}.

export type Scope = "project" | "global";
export type Effect = "allow" | "deny" | "ask";
export type RiskLevel = "safe" | "moderate" | "broad" | "dangerous";

export interface PermissionRule {
  scope: Scope;
  effect: Effect;
  tool: string;
  pattern: string | null;
  raw: string;
  source: "agent-console" | "external";
  createdAtMs?: number;
}

export interface RuleSuggestion {
  label: string;
  rule: Omit<PermissionRule, "source" | "createdAtMs">;
  risk: RiskLevel;
  riskReason?: string;
  requiresConfirm: boolean;
  hardDeny?: boolean;
}

export interface ToolUseRequest {
  id: string;
  sessionDir: string;
  cwd: string;
  tool: string;
  input: Record<string, unknown>;
  ts: number;
}

export type ApprovalDecision = "allow" | "deny" | "ask";

export interface ApprovalResponse {
  id: string;
  decision: ApprovalDecision;
  reason?: string;
}
