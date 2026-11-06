# Threat Model — Remote Decisions (desktop + companion)

**Status (2026-11-06, SEGURIDAD-MOVIL-2026-11):** rewritten to describe the
system that exists, not the relay-and-voice design this file began as. The
desktop side is in `main` (`secure_channel.rs`, `pairing_service.rs`,
`paired_devices_service.rs`, `transport.rs`, `ws.rs`, `remote_decisions.rs`,
`push_service.rs`); the device is the companion PWA at
`agent-console.dev/app` (repo `agent-console-companion`). Voice and a relay
are **not** part of it; both stay deferred and are listed at the end so
nobody mistakes them for controls.

**One-line framing:** this is a *remote control for an arbitrary-code
executor*. Any weakness is RCE-grade. Design as if the channel is already
compromised: a device may allow only what cannot change the machine, and
every act leaves a ledger line.

## The system

```
📱 companion PWA (initiator)               🖥️ agent-console (responder)
   scans the QR (desktop key + one-time PSK)   Trust → Remote devices: listen, Pair
   Noise XXpsk3 over WebSocket  ─────────────▶  pending pairing + 4-word SAS
   human compares the 4 words on both screens   human approves with a scope
   Noise KK reconnect, one request each ─────▶  pending / decide / sessions / push_info / subscribe
   ◀── Web Push "N decisions wait" (count only) ─ knock after 15 s, ≤ 1/min
```

Transport: one TCP port on the desktop, bound to `127.0.0.1` by default or
to one private LAN/tailnet address by a typed choice (never `0.0.0.0`, never
a public or VPN address); the same port serves raw sockets and WebSocket
upgrades. From outside the LAN the user brings Tailscale; `tailscale serve`
gives the `wss://` an installed PWA needs. Nothing of ours is exposed to the
internet and no server of ours is in the path.

## Assets

The ability to **run commands on the developer's machine** through the
agent; the repos and credentials on it; the desktop identity key (pins every
paired device); the device identity key (one device's right to answer);
the ledger (what reviewers are shown); the published app (what every phone
runs).

## Trust boundaries

`phone browser` ⟂ `network (LAN / tailnet)` ⟂ `desktop listener` ⟂
`console (gate, ledger)` ⟂ `agent + its tools` ⟂ `machine`. The agent's
tools run **as the user**: inside that boundary nothing of the console's is
secret from the agent. Only the engine's sandbox draws a line there.

## Adversaries

Someone on the same network · someone who photographed the QR · a thief of
the phone (unlocked, or with the browser's storage) · a malicious or buggy
device app · a man-in-the-middle on the path · the agent itself under prompt
injection · whoever can change the published app · the push relay (Google,
Apple, Mozilla) · a compromised dependency.

## Threats → controls (what the code does)

| # | Threat | Control | Where |
|---|---|---|---|
| T1 | MITM on the network | Noise XXpsk3 with the desktop key pinned from the QR; the device verifies the key the moment it arrives (message 2), before its own key travels; KK on reconnect authenticates both sides with pinned keys | `secure_channel.rs`, `noise.ts` |
| T2 | Replay | Fresh ephemerals per connection, Noise nonces, one request per connection | `transport.rs` |
| T3 | Someone pairs to the desktop | One-time PSK with a 2-min TTL, spent only by a completed handshake; pairing creates a *pending* entry; trust needs a click with a scope; the **4-word SAS** derived from the handshake hash is shown on both screens and the human approves only if they match; pendings expire in 10 min, 8 at most, labels bounded | `pairing_service.rs`, `paired_devices_service.rs`, `sas.rs` |
| T4 | Stolen phone / stolen device key | Scope per device; **only read-only requests can be allowed from a device**; deny is always possible; revoke forgets the key and the next connection is refused before any crypto; the device key lives in IndexedDB (no OS keystore in a web app — stated, see F7) | `remote_decisions::gate`, Trust → Remote devices |
| T5 | A device drives the agent | No prompts over the channel; the device's free text (`reason`) never reaches the hook or the model — a fixed sentence does; the text goes to the ledger bounded and control-free | `remote_decisions::decide` |
| T6 | A dangerous action approved remotely | Tier computed on the desktop (shell allowlist of single commands, no chaining/redirection/substitution/xargs; irreversible patterns; sensitive paths; destructive tool servers), recomputed at `decide`; `remoteAllow` only for low | `remote_decisions::tier_for`, `lib/decisions.ts` |
| T7 | The agent approves itself (same user) | The console remembers every answer it wrote; the hook reports what it picked up and a forged answer becomes a deny; a stray answer file is removed, recorded (`approval_forged`), the session interrupted; answers are exclusive, UUID-named, 0600 in a 0700 dir. **Prevention is the engine's sandbox** (`sandbox.enabled`, one click in Trust) | `hooks_service.rs`, `inject_service.rs`, `hook-bridge`, `sandbox_service.rs` |
| T8 | Resource exhaustion from the network | 10 s to Hello + handshake, 5 min idle, 32 concurrent, accept back-off, 64 KiB frames, WebSocket origin allowlist, pong cap; UI nudged only by real pairings | `transport.rs`, `ws.rs` |
| T9 | Secrets leave the machine | Summaries and ledger inputs redacted (headers, key=value, URL userinfo, PEM, token shapes); file bodies kept as digests; ledger 0700/0600; the push carries a count and nothing else | `redact.rs`, `testigo_service.rs`, `push_service.rs` |
| T10 | SSRF through the push endpoint | https only, no IP literals, no localhost, known push services or hosts resolving only to public addresses, no redirects; `subscribe` needs `readApprovals` | `push_service.rs` |
| T11 | A poisoned published app | https + HSTS at the edge (Cloudflare — operator setting), CSP (`default-src 'self'`, no inline script, `frame-ancestors 'none'`), worker caches only good shell responses; the page is static with no backend | `docs/app`, companion `index.html`, `sw.js` |
| T12 | Push relay learns things | Payload is ciphertext of `{"type":"decisions","pending":N}`; the relay sees timing and that a knock happened | `push_service.rs` |
| T13 | Ledger incomplete | Every remote act is a ledger line (`remote_listener`, `remote_device`, `approval_decision via.remote`, `approval_forged`), with a machine-level ledger when no project is open; the decision's line is written before the hook sees the answer | `commands/remote.rs`, `remote_decisions.rs` |

## What is NOT covered, said plainly

- **Same-user isolation without the sandbox.** With the engine's sandbox
  off, an agent that forges an answer *and* rewrites `inject-port.json` to
  its own server can tell its own hook the answer is genuine. The console
  detects the common case and says so in red; the sandbox is the wall.
- **The device key is software-protected.** A web app has no keystore for
  raw X25519 secrets. Compromise of the browser profile or an unlocked
  phone = that device's rights (read-only approvals at most) until revoked.
- **The pin check trusts the QR.** A sticker over the real QR pairs the
  phone to the attacker's desktop; the companion shows host and key
  fingerprint before dialling and the SAS afterwards, but a human who does
  not look is not protected.
- **HTTPS at the edge is an operator setting**, not code: if Cloudflare's
  "Always use HTTPS" and HSTS are off, a hotspot can serve a modified app.
- **No external audit** of the protocol composition has been done.
- **Hello is cleartext**: a LAN observer learns device public keys and
  whether a desktop trusts them.

## Deferred (not controls, not promises)

Voice over the channel (needs an on-device second factor); a relay for
"from anywhere without a VPN" (outbound-only, blind — designed, not built);
a native build with the key in Keychain/Keystore (F7); an external audit.

## How this document is kept honest

Every control above names the code that enforces it. A change to that code
updates this table in the same PR, and the adversarial review of
`SEGURIDAD-MOVIL-2026-11.md` is repeated on each minor release of the
channel by someone who did not write the change.
