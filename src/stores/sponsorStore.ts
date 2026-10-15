import { create } from "zustand";
import { ipc } from "../ipc/tauri";
import { tierFor, type DecisionTier } from "../lib/decisions";
import { useApprovalStore } from "./approvalStore";
import { useDecisionStatsStore } from "./decisionStatsStore";
import { useLiveStatusStore } from "./liveStatusStore";
import { useSessionStore } from "./sessionStore";
import { useTerminalsStore, type TerminalSession } from "./terminalsStore";
import { useToastStore } from "./toastStore";
import type { ApprovalRequest, Sponsor } from "../types/domain";

export const WEEK_MS = 7 * 24 * 3600 * 1000;

const RANK: Record<DecisionTier, number> = { low: 0, medium: 1, high: 2 };

/// This week's spend as the sessions report it: the cumulative cost of every
/// session created in the last seven days, from the CLI's own status line.
/// Jobs and rooms report through their sessions too; headless runs (advisor,
/// reviewer) are not counted — the ledger has their cost per event.
export function weekSpend(
  sessions: Pick<TerminalSession, "id" | "createdAtMs">[],
  byTerm: Record<string, { costUsd?: number } | undefined>,
  now = Date.now(),
): number {
  let usd = 0;
  for (const s of sessions) {
    if (now - s.createdAtMs > WEEK_MS) continue;
    usd += byTerm[s.id]?.costUsd ?? 0;
  }
  return Math.round(usd * 100) / 100;
}

/// May the sponsor policy decide this request alone? Only when the policy
/// is on, the tier is within the threshold, and the week's budget (if any)
/// is not spent. Never a deny: what the policy does not approve, the human
/// sees.
export function shouldAutoApprove(
  sponsor: Sponsor | null,
  tier: DecisionTier,
  spentThisWeek: number,
): { yes: boolean; why: string } {
  if (!sponsor?.autoApprove) return { yes: false, why: "policy off" };
  if (RANK[tier] > RANK[sponsor.riskThreshold])
    return { yes: false, why: `${tier} tier is above the ${sponsor.riskThreshold} threshold` };
  if (sponsor.weeklyBudgetUsd > 0 && spentThisWeek >= sponsor.weeklyBudgetUsd)
    return { yes: false, why: "weekly budget reached" };
  return { yes: true, why: `${tier} tier within the ${sponsor.riskThreshold} threshold` };
}

interface SponsorState {
  project: string | null;
  sponsor: Sponsor | null;
  /// Decisions the policy took this week (local counter, by project+week).
  autoApproved: number;
  load: (projectRoot: string) => Promise<void>;
  save: (patch: Partial<Sponsor>) => Promise<void>;
  /// Consider a freshly queued request: approve it under the policy or
  /// leave it for the human. Returns true when the policy took it.
  consider: (req: ApprovalRequest) => Promise<boolean>;
}

const KEY = "agent-console:sponsor-auto";
const weekKey = (project: string) => `${project}:${Math.floor(Date.now() / WEEK_MS)}`;

function loadCount(project: string): number {
  try {
    const raw = localStorage.getItem(KEY);
    const m = raw ? (JSON.parse(raw) as Record<string, number>) : {};
    return m[weekKey(project)] ?? 0;
  } catch {
    return 0;
  }
}
function saveCount(project: string, n: number) {
  try {
    localStorage.setItem(KEY, JSON.stringify({ [weekKey(project)]: n }));
  } catch {
    /* ignore */
  }
}

export const useSponsorStore = create<SponsorState>((set, get) => ({
  project: null,
  sponsor: null,
  autoApproved: 0,
  load: async (projectRoot) => {
    try {
      const sponsor = await ipc.sponsorGet(projectRoot);
      set({ project: projectRoot, sponsor, autoApproved: loadCount(projectRoot) });
    } catch (e) {
      console.warn("[sponsor] load failed:", e);
      set({ project: projectRoot, sponsor: null });
    }
  },
  save: async (patch) => {
    const { project, sponsor } = get();
    if (!project) return;
    const next: Sponsor = {
      objective: "",
      owner: "",
      weeklyBudgetUsd: 0,
      riskThreshold: "low",
      autoApprove: false,
      ...sponsor,
      ...patch,
    };
    try {
      set({ sponsor: await ipc.sponsorSet(project, next) });
    } catch (e) {
      useToastStore.getState().show(`Couldn't save the sponsor: ${String(e)}`, "error");
    }
  },
  consider: async (req) => {
    const { project, sponsor } = get();
    if (!project || !sponsor?.autoApprove) return false;
    if (useSessionStore.getState().project?.root !== project) return false;
    const sessions = useTerminalsStore.getState().sessions;
    const session = sessions.find((s) => s.id === req.termId);
    const tier = tierFor(req, { allowedHosts: session?.allowedHosts }).tier;
    const spent = weekSpend(sessions, useLiveStatusStore.getState().byTerm);
    const verdict = shouldAutoApprove(sponsor, tier, spent);
    if (!verdict.yes) return false;
    const policy = `sponsor:${sponsor.owner || "unnamed"}:${sponsor.riskThreshold}`;
    const ok = await useApprovalStore
      .getState()
      .decide(req.id, "allow", `sponsor policy: ${verdict.why}`, policy);
    if (ok) {
      useDecisionStatsStore.getState().record(tier, "allow");
      const n = get().autoApproved + 1;
      set({ autoApproved: n });
      saveCount(project, n);
    }
    return ok;
  },
}));

/// Watch the approval queue: every request that arrives is offered to the
/// policy once. Returns the unsubscribe.
export function attachSponsorWatcher(): () => void {
  const seen = new Set<string>();
  return useApprovalStore.subscribe((s) => {
    for (const req of s.queue) {
      if (seen.has(req.id)) continue;
      seen.add(req.id);
      void useSponsorStore.getState().consider(req);
    }
    if (seen.size > 500) seen.clear();
  });
}
