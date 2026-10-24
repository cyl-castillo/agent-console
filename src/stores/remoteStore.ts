import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ipc } from "../ipc/tauri";
import type { DeviceScope, PairingStartResult, RemoteStatus } from "../types/domain";

/// F8: the remote-decisions listener and the devices allowed to answer from
/// afar. Re-read when the Rust side announces a change (`remote://changed`):
/// a pairing arrived, a device was approved, revoked or re-scoped, the
/// listener started or stopped.
interface RemoteState {
  status: RemoteStatus | null;
  /// The pairing QR on screen, until it expires or is dismissed.
  offer: PairingStartResult | null;
  loading: boolean;
  error: string | null;
  load: () => Promise<void>;
  startServer: (bind: "loopback" | "lan", port?: number) => Promise<void>;
  stopServer: () => Promise<void>;
  startPairing: () => Promise<void>;
  clearOffer: () => void;
  approve: (pendingId: string, scope?: DeviceScope) => Promise<void>;
  reject: (pendingId: string) => Promise<void>;
  revoke: (id: string) => Promise<void>;
  setScope: (id: string, scope: DeviceScope) => Promise<void>;
}

export const useRemoteStore = create<RemoteState>((set, get) => ({
  status: null,
  offer: null,
  loading: false,
  error: null,
  load: async () => {
    set({ loading: true, error: null });
    try {
      const status = await ipc.remoteStatus();
      set({ status, loading: false });
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },
  startServer: async (bind, port) => {
    const status = await ipc.remoteServerStart(bind, port);
    set({ status, error: null });
  },
  stopServer: async () => {
    const status = await ipc.remoteServerStop();
    set({ status, offer: null });
  },
  startPairing: async () => {
    const offer = await ipc.remotePairingStart();
    set({ offer });
    // The QR is one-time and short-lived: it leaves the screen with its TTL.
    window.setTimeout(
      () => {
        if (get().offer === offer) set({ offer: null });
      },
      Math.max(0, offer.expiresAtMs - Date.now()),
    );
  },
  clearOffer: () => set({ offer: null }),
  approve: async (pendingId, scope) => {
    await ipc.remoteApprove(pendingId, scope);
    set({ offer: null });
    await get().load();
  },
  reject: async (pendingId) => {
    await ipc.remoteReject(pendingId);
    await get().load();
  },
  revoke: async (id) => {
    await ipc.remoteRevoke(id);
    await get().load();
  },
  setScope: async (id, scope) => {
    await ipc.remoteSetScope(id, scope);
    await get().load();
  },
}));

export async function attachRemoteListeners(): Promise<UnlistenFn> {
  void useRemoteStore.getState().load();
  const unlisten = await listen("remote://changed", () => {
    void useRemoteStore.getState().load();
  });
  return unlisten;
}
