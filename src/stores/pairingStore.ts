import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { DeviceScope, PairedDevice, PairingStartResult, PendingPairing } from "../types/domain";
import { useToastStore } from "./toastStore";

interface PairingState {
  /// Active pairing offer (QR) being shown, or null.
  offer: PairingStartResult | null;
  starting: boolean;
  pending: PendingPairing[];
  devices: PairedDevice[];
  error: string | null;

  startPairing: () => Promise<void>;
  clearOffer: () => void;
  refresh: () => Promise<void>;
  approve: (pendingId: string, scope?: DeviceScope) => Promise<void>;
  reject: (pendingId: string) => Promise<void>;
  revoke: (id: string) => Promise<void>;
  setScope: (id: string, scope: DeviceScope) => Promise<void>;
  simulateIncoming: () => Promise<void>;
}

export const usePairingStore = create<PairingState>((set, get) => ({
  offer: null,
  starting: false,
  pending: [],
  devices: [],
  error: null,

  startPairing: async () => {
    set({ starting: true, error: null });
    try {
      const offer = await ipc.pairingStart();
      set({ offer, starting: false });
    } catch (err) {
      set({ starting: false, error: err instanceof Error ? err.message : String(err) });
    }
  },

  clearOffer: () => set({ offer: null }),

  refresh: async () => {
    try {
      const [pending, devices] = await Promise.all([ipc.pairingPending(), ipc.devicesList()]);
      set({ pending, devices });
    } catch (err) {
      set({ error: err instanceof Error ? err.message : String(err) });
    }
  },

  approve: async (pendingId, scope) => {
    try {
      const dev = await ipc.pairingApprove(pendingId, scope);
      useToastStore.getState().show(`Paired "${dev.label}"`, "success");
      await get().refresh();
    } catch (err) {
      useToastStore.getState().show(
        `Pairing failed: ${err instanceof Error ? err.message : String(err)}`,
        "error",
      );
    }
  },

  reject: async (pendingId) => {
    await ipc.pairingReject(pendingId);
    await get().refresh();
  },

  revoke: async (id) => {
    await ipc.devicesRevoke(id);
    useToastStore.getState().show("Device revoked", "info");
    await get().refresh();
  },

  setScope: async (id, scope) => {
    await ipc.devicesSetScope(id, scope);
    await get().refresh();
  },

  simulateIncoming: async () => {
    await ipc.pairingSimulateIncoming("Test phone (dev)");
    await get().refresh();
  },
}));
