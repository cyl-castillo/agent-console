import { create } from "zustand";
import { ipc } from "../ipc/tauri";
import type { PolicyStatus } from "../types/domain";
import { useSessionStore } from "./sessionStore";

/// F6: the project's approval policy as the file holds it, with what it
/// compiles to per engine. Loaded when a project opens; saved and applied
/// from the Trust tab.
interface PolicyState {
  projectRoot: string | null;
  status: PolicyStatus | null;
  loading: boolean;
  error: string | null;
  load: (projectRoot: string) => Promise<void>;
  save: (policyJson: string) => Promise<void>;
  apply: () => Promise<PolicyStatus | null>;
}

export const usePolicyStore = create<PolicyState>((set, get) => ({
  projectRoot: null,
  status: null,
  loading: false,
  error: null,
  load: async (projectRoot) => {
    set({ projectRoot, loading: true, error: null });
    try {
      const status = await ipc.policyStatus(projectRoot);
      if (get().projectRoot !== projectRoot) return;
      set({ status, loading: false });
    } catch (e) {
      if (get().projectRoot !== projectRoot) return;
      set({ loading: false, error: String(e) });
    }
  },
  save: async (policyJson) => {
    const root = get().projectRoot;
    if (!root) return;
    set({ error: null });
    try {
      const status = await ipc.policySave(root, policyJson);
      if (get().projectRoot === root) set({ status });
    } catch (e) {
      set({ error: String(e) });
      throw e;
    }
  },
  apply: async () => {
    const root = get().projectRoot;
    if (!root) return null;
    set({ error: null });
    try {
      const status = await ipc.policyApply(root);
      if (get().projectRoot === root) set({ status });
      return status;
    } catch (e) {
      set({ error: String(e) });
      throw e;
    }
  },
}));

export function attachPolicyListener(): () => void {
  const unsub = useSessionStore.subscribe((s, prev) => {
    const root = s.project?.root ?? null;
    if (root && root !== (prev.project?.root ?? null)) void usePolicyStore.getState().load(root);
    if (!root) usePolicyStore.setState({ projectRoot: null, status: null });
  });
  const root = useSessionStore.getState().project?.root;
  if (root) void usePolicyStore.getState().load(root);
  return unsub;
}
