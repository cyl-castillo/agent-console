import { useEffect, useState } from "react";
import { confirmDialog } from "../stores/confirmStore";
import { useRemoteStore } from "../stores/remoteStore";
import { useToastStore } from "../stores/toastStore";
import { Chip } from "../ui/Chip";
import type { DeviceScope } from "../types/domain";

const SCOPES: { value: DeviceScope; label: string; hint: string }[] = [
  { value: "readOnly", label: "read only", hint: "sees the queue, answers nothing" },
  {
    value: "readApprovals",
    label: "approvals",
    hint: "allows low and medium; dangerous actions stay at the keyboard",
  },
  { value: "full", label: "full", hint: "same as approvals today; prompts are not remote yet" },
];

function scopeLabel(s: DeviceScope): string {
  return SCOPES.find((x) => x.value === s)?.label ?? s;
}

/// F8: who may answer decisions from a paired device, and where the desktop
/// listens. Pairing is a QR shown here and a click here — the device never
/// becomes trusted on its own. Loopback by default; the LAN is a typed choice
/// and goes to the ledger like every other act on this section.
export function RemoteDevicesSection() {
  const status = useRemoteStore((s) => s.status);
  const offer = useRemoteStore((s) => s.offer);
  const error = useRemoteStore((s) => s.error);
  const load = useRemoteStore((s) => s.load);
  const startServer = useRemoteStore((s) => s.startServer);
  const stopServer = useRemoteStore((s) => s.stopServer);
  const startPairing = useRemoteStore((s) => s.startPairing);
  const clearOffer = useRemoteStore((s) => s.clearOffer);
  const approve = useRemoteStore((s) => s.approve);
  const reject = useRemoteStore((s) => s.reject);
  const revoke = useRemoteStore((s) => s.revoke);
  const setScope = useRemoteStore((s) => s.setScope);
  const showToast = useToastStore((s) => s.show);
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!offer) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [offer]);

  const run = async (label: string, f: () => Promise<void>) => {
    setBusy(true);
    try {
      await f();
    } catch (e) {
      showToast(`${label}: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };

  const start = async (bind: "loopback" | "lan") => {
    if (bind === "lan") {
      const ok = await confirmDialog({
        title: "Listen on the LAN",
        message:
          "Every device on this network will be able to reach the listener. Only devices you pair here can speak to it (Noise, pinned keys), and a dangerous action is never approved from a device. The choice is recorded in the ledger.",
        confirmLabel: "Listen on the LAN",
        danger: true,
      });
      if (!ok) return;
    }
    await run("Couldn't start the listener", () => startServer(bind));
  };

  const running = status?.running ?? false;
  const secondsLeft = offer ? Math.max(0, Math.ceil((offer.expiresAtMs - now) / 1000)) : 0;

  return (
    <section className="wb-section" data-testid="remote-devices">
      <div className="wb-section-title">
        remote devices
        <Chip tone={running ? "ok" : "neutral"} mono data-testid="remote-state">
          {running ? `listening · ${status?.bind}` : "off"}
        </Chip>
        {status?.identityStore && (
          <Chip tone={status.identityStore === "keyring" ? "neutral" : "warn"} mono>
            identity in {status.identityStore}
          </Chip>
        )}
        {status?.identityError && (
          <Chip tone="danger" mono title={status.identityError}>
            keystore unavailable
          </Chip>
        )}
      </div>
      <p className="wb-hint">
        A paired device sees the decisions waiting here, with their evidence, and answers. No chat,
        no diffs. Dangerous actions are decided at this keyboard whatever the device's scope.
        {running && status?.addr && (
          <>
            {" "}
            · dial <span className="font-code">{status.addr}</span>
          </>
        )}
        {error && <> · {error}</>}
        {status?.identityError && <> · {status.identityError}</>}
      </p>
      <div className="flex flex-wrap gap-2">
        {!running && (
          <>
            <button
              className="wb-cta"
              disabled={busy}
              onClick={() => void start("loopback")}
              data-testid="remote-start-loopback"
            >
              listen on this machine
            </button>
            <button className="wb-cta" disabled={busy} onClick={() => void start("lan")}>
              listen on the LAN…
            </button>
          </>
        )}
        {running && (
          <>
            <button
              className="wb-cta"
              disabled={busy || offer !== null}
              onClick={() => void run("Couldn't start pairing", startPairing)}
              data-testid="remote-pair"
              title={
                status?.bind === "loopback"
                  ? "The listener is on this machine only: a phone needs the LAN listener or a tunnel to this port"
                  : undefined
              }
            >
              {status?.bind === "loopback" ? "pair a device on this machine" : "pair a device"}
            </button>
            <button
              className="wb-cta"
              disabled={busy}
              onClick={() => void run("Couldn't stop the listener", stopServer)}
            >
              stop listening
            </button>
          </>
        )}
        <button className="wb-cta" disabled={busy} onClick={() => void load()}>
          refresh
        </button>
      </div>

      {offer && (
        <div className="mt-2 flex flex-wrap items-start gap-3" data-testid="remote-offer">
          <div
            className="h-[220px] w-[220px] shrink-0 rounded-1 bg-white p-1"
            // The SVG comes from our own Rust QR renderer, not from user input.
            dangerouslySetInnerHTML={{ __html: offer.qrSvg }}
            aria-label="Pairing QR code"
            role="img"
          />
          <div className="flex min-w-0 flex-1 flex-col gap-1 text-small text-muted-foreground">
            <p className="m-0">
              Scan from the device, or paste the link into it. The offer is one-time and expires in{" "}
              <span className="font-code tabular-nums">{secondsLeft}s</span>; the device then shows
              up below for your approval.
              {status?.bind === "loopback" && (
                <>
                  {" "}
                  The link dials <span className="font-code">{status.addr}</span>: only a client on
                  this machine (or a tunnel to this port) can reach it — a phone on the Wi-Fi needs
                  "listen on the LAN".
                </>
              )}
            </p>
            <textarea
              readOnly
              rows={3}
              value={offer.uri}
              className="w-full rounded-1 border border-border bg-background p-1 font-code text-caption text-foreground"
              aria-label="Pairing link"
              onFocus={(e) => e.currentTarget.select()}
            />
            <button className="wb-cta self-start" onClick={clearOffer}>
              dismiss
            </button>
          </div>
        </div>
      )}

      {status && status.pending.length > 0 && (
        <ul className="m-0 mt-2 list-none p-0" data-testid="remote-pending">
          {status.pending.map((p) => (
            <PendingRow
              key={p.id}
              label={p.label}
              publicKey={p.publicKey}
              proposed={p.proposedScope}
              busy={busy}
              onApprove={(scope) => void run("Couldn't approve", () => approve(p.id, scope))}
              onReject={() => void run("Couldn't reject", () => reject(p.id))}
            />
          ))}
        </ul>
      )}

      {status && status.devices.length > 0 && (
        <ul className="m-0 mt-2 list-none p-0" data-testid="remote-list">
          {status.devices.map((d) => (
            <li
              key={d.id}
              className="flex flex-wrap items-center gap-2 border-t border-border py-1 text-small"
              data-testid={`remote-device-${d.id}`}
            >
              <span className="min-w-0 flex-1 truncate text-foreground">{d.label}</span>
              <span className="font-code text-caption text-faint-foreground">
                {d.publicKey.slice(0, 10)}…
              </span>
              <select
                aria-label={`Scope of ${d.label}`}
                className="rounded-1 border border-border bg-background px-1 text-caption text-foreground"
                value={d.scope}
                disabled={busy}
                onChange={(e) =>
                  void run("Couldn't change the scope", () =>
                    setScope(d.id, e.target.value as DeviceScope),
                  )
                }
              >
                {SCOPES.map((s) => (
                  <option key={s.value} value={s.value} title={s.hint}>
                    {s.label}
                  </option>
                ))}
              </select>
              <span className="text-caption text-muted-foreground">
                {d.lastSeenMs ? `seen ${new Date(d.lastSeenMs).toLocaleString()}` : "never seen"}
              </span>
              <button
                className="wb-cta"
                disabled={busy}
                onClick={() =>
                  void run("Couldn't revoke", async () => {
                    if (
                      await confirmDialog({
                        title: "Revoke device",
                        message: `Forget "${d.label}"? Its next connection is refused before any handshake.`,
                        confirmLabel: "Revoke",
                        danger: true,
                      })
                    )
                      await revoke(d.id);
                  })
                }
              >
                revoke
              </button>
            </li>
          ))}
        </ul>
      )}
      {status && status.devices.length === 0 && status.pending.length === 0 && !offer && (
        <p className="wb-hint">No device paired yet.</p>
      )}
    </section>
  );
}

function PendingRow({
  label,
  publicKey,
  proposed,
  busy,
  onApprove,
  onReject,
}: {
  label: string;
  publicKey: string;
  proposed: DeviceScope;
  busy: boolean;
  onApprove: (scope: DeviceScope) => void;
  onReject: () => void;
}) {
  const [scope, setScopeLocal] = useState<DeviceScope>(proposed);
  return (
    <li
      className="flex flex-wrap items-center gap-2 border-t border-border py-1 text-small"
      data-testid="remote-pending-row"
    >
      <Chip tone="warn" mono>
        wants to pair
      </Chip>
      <span className="min-w-0 flex-1 truncate text-foreground">{label}</span>
      <span className="font-code text-caption text-faint-foreground">
        {publicKey.slice(0, 10)}…
      </span>
      <select
        aria-label={`Scope for ${label}`}
        className="rounded-1 border border-border bg-background px-1 text-caption text-foreground"
        value={scope}
        onChange={(e) => setScopeLocal(e.target.value as DeviceScope)}
      >
        {SCOPES.map((s) => (
          <option key={s.value} value={s.value} title={s.hint}>
            {s.label}
          </option>
        ))}
      </select>
      <button className="wb-cta" disabled={busy} onClick={() => onApprove(scope)}>
        approve as {scopeLabel(scope)}
      </button>
      <button className="wb-cta" disabled={busy} onClick={onReject}>
        reject
      </button>
    </li>
  );
}
