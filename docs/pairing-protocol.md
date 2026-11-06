# Remote Decisions — Wire Protocol (v2)

The contract between **agent-console** (desktop, the responder) and a **device
app** (a separate repo — native or PWA; the initiator). Implement this exactly
to interoperate. The desktop reference implementation lives in
`src-tauri/src/services/transport.rs` (`client` module = the device side) and
the runnable reference client is the `phone-sim` binary
(`src-tauri/src/bin/phone_sim.rs`).

See also: `docs/threat-model-voice-companion.md` (security rationale).

Implementations: the desktop (this repo), the reference device `phone-sim`,
the reference desktop `desktop-sim` (a standalone responder for testing a
device app), and the companion PWA served at `agent-console.dev/app`
(source: the `agent-console-companion` repo; its built files live in
`docs/app/` here).

**What the device does, and only that:** show the decisions waiting on the
desktop — with their evidence — and return an answer. No chat, no diffs, no
prompts. v1 of this document carried a voice protocol; it was replaced, not
extended (COMPETENCIA-2026-10 F8). Driving the agent from a device stays
gated on an on-device second factor (threat model T5).

## Crypto

- **Noise Protocol Framework** (no hand-rolled crypto). Suites:
  - First-contact pairing: `Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s`
  - Reconnect (already paired): `Noise_KK_25519_ChaChaPoly_BLAKE2s`
- Identity = a long-term X25519 static keypair on each side. Private keys never
  leave the device. The desktop keeps its key in the OS keystore (Keychain /
  Credential Manager / Secret Service) and falls back to a 0600 file when no
  keystore answers; the UI says which (`identity in keyring|file`).
- The desktop's static public key is delivered to the device **out-of-band via
  the QR** — that's what makes the (untrusted) network un-MITM-able.

## Transports

The desktop listens on **one** TCP port and serves two transports on it,
telling them apart by the first bytes of the connection:

- **Raw TCP** — the framed byte stream below starts immediately (its first
  bytes are a big-endian length, never ASCII). Native apps and `phone-sim`.
- **WebSocket** (RFC 6455) — the connection starts with an HTTP `GET`; the
  desktop upgrades it and then runs the *identical* protocol over it. This is
  what a browser (the companion PWA) uses: `new WebSocket("ws://<host:port>/")`
  with `binaryType = "arraybuffer"`. Rules: binary messages only (a text frame
  ends the connection), no subprotocol, no extensions, any path. The
  WebSocket carries the protocol's **byte stream**: message boundaries mean
  nothing, the length prefix is the framing — concatenate what arrives and
  parse frames from it, exactly as on TCP. `phone-sim --ws` is the reference
  client for this transport and the desktop's own tests drive it.

Encryption is Noise above either transport, so `ws://` is fine on the LAN. A
page served over **https** (an installed PWA) can only open `wss://`: put a
TLS edge in front of the port — `tailscale serve` on the desktop gives a
`wss://<machine>.<tailnet>.ts.net` with a valid certificate and reaches the
phone anywhere; any reverse proxy with a certificate the phone trusts does
the same. The desktop never terminates TLS itself and never listens on
`0.0.0.0`.

## Framing

Every message on the wire is length-prefixed:

```
[u32 big-endian length N][N bytes payload]
```

Max frame = 65536 bytes. Handshake messages and encrypted app messages are all
framed this way. A clean EOF between frames = orderly close.

## Pairing offer (QR payload)

The desktop shows a QR encoding `agentconsole://pair?d=<base64url(JSON)>&a=<host:port>`,
where the JSON is:

```json
{
  "rendezvousId": "<base64url, 16 random bytes>",
  "responderStatic": "<base64url of desktop X25519 public key>",
  "psk": "<base64url, 32 random bytes — one-time>",
  "createdMs": 1700000000000,
  "ttlSecs": 120
}
```

`a` is the address the device dials — the listener's bound address:
`127.0.0.1:<port>` by default (only a client on the same machine, or a
tunnel to that port, can reach it) or the machine's LAN address when the
human chose to listen on the LAN (the console binds that one address, never
every interface). The offer is **one-time** and
**short-lived** (TTL). The device pins `responderStatic` and uses `psk` as the
Noise PSK.

## Connection flow

Each connection begins with a **cleartext** `Hello` frame (JSON) declaring intent.
It carries no secret (a public key and an intent are not sensitive); the handshake
authenticates.

```json
{ "type": "pair" }
{ "type": "reconnect", "key": "<base64url of device's static public key>" }
```

### A. Pair (first contact)

1. Device → `Hello{type:"pair"}`.
2. Noise **XXpsk3** handshake over framed messages (initiator = device, writes
   first). Device supplies its static key + the offer's PSK; desktop supplies
   its static key + PSK.
3. Device **verifies** the desktop's transmitted static key equals the
   QR-pinned key (anti-MITM). Abort if mismatch.
4. Transport mode. Device sends one E2E-encrypted frame = its **label** (UTF-8,
   e.g. "Carlos's iPhone").
5. Desktop registers a **pending** pairing and replies with an encrypted
   `"pending-approval"` frame, then closes.
6. The human approves on the desktop (Trust → Remote devices), choosing the
   scope. Only then is the device trusted; the ledger records
   `remote_device {action: paired, scope}`.

### B. Reconnect (paired device)

1. Device → `Hello{type:"reconnect", key:<its pubkey>}`.
2. Desktop looks the key up in its trust store. Unknown/revoked → it closes the
   connection (no handshake).
3. Noise **KK** handshake (both sides already know each other's static key —
   device pinned the desktop at pairing; desktop stored the device on
   approval). No PSK.
4. Transport mode → the decisions protocol below. Request/response, one reply
   per message; the device closes the stream when done (clean EOF ends the
   desktop's loop).

## Decisions protocol (after reconnect)

Each app message is an E2E-encrypted frame whose plaintext is JSON.

Device → desktop (`ClientMessage`):

```json
{ "type": "ping" }
{ "type": "pending" }
{ "type": "decide", "id": "<approval id>", "decision": "allow" | "deny", "reason": "optional" }
{ "type": "sessions" }
{ "type": "push_info" }
{ "type": "subscribe", "endpoint": "https://…", "p256dh": "<base64url>", "auth": "<base64url>" }
{ "type": "unsubscribe" }
```

Desktop → device (`ServerMessage`):

```json
{ "type": "pong" }
{ "type": "decisions", "items": [ <RemoteDecision>, … ], "truncated": false }
{ "type": "decided", "id": "<approval id>" }
{ "type": "sessions", "items": [ <RemoteSession>, … ] }
{ "type": "push_info", "vapidPublicKey": "<base64url, 65 bytes>" }
{ "type": "subscribed" }
{ "type": "unsubscribed" }
{ "type": "error", "message": "..." }
```

A `RemoteSession` (read-only; every scope may ask):

```json
{
  "termId": "sess-auth",
  "name": "auth refactor",
  "project": "/home/u/code/demo",
  "agent": "claude",
  "state": "needs_you" | "working" | "idle" | "stopped",
  "pending": 2,
  "lastActiveMs": 1700000000000
}
```

`state` mirrors the desktop Inbox's groups: `needs_you` = a decision waits
in that session (`pending` says how many), `working` = a turn is open in the
ledger, `idle` = a shell is live and nothing is open, `stopped` = no shell
behind it. Needs-you first, then working, idle, stopped; at most 100 items.

A `RemoteDecision`:

```json
{
  "id": "ap-…",
  "ts": 1700000000000,
  "tool": "Bash",
  "summary": "npm test",
  "termId": "sess-auth",
  "session": "auth refactor",
  "project": "/home/u/code/demo",
  "tier": "low" | "medium" | "high",
  "tierLabel": "Read-only",
  "tierReason": "inspects, builds or tests",
  "remoteAllow": true,
  "evidence": {
    "verification": "verified" | "unverified_claim" | "claim_contradicted" | …,
    "lastCheck": { "command": "npm test", "exitCode": 0, "status": "passed" }
  }
}
```

- `items` is newest first and holds at most 40 decisions; `truncated: true`
  says more wait on the desktop (ask again after answering some).
- `tier` is computed on the **desktop** from the request (`remote_decisions::tier_for`,
  the same rules as the Inbox card: for the shell a read-only allowlist of
  single commands — no chaining, redirection, substitution or `xargs` — and
  pattern lists for what is irreversible; for file tools a sensitive or
  out-of-project path is high; tool servers are high when their name says
  they destroy, send or publish). The device must render what it receives
  and may not raise or lower it.
- `remoteAllow` is `true` **only for a low (read-only) tier**. Medium and
  high mean the device may **deny** but never **allow**: anything that
  changes the machine or leaves it is decided at the keyboard. The desktop
  enforces this on `decide` regardless of what the device shows.
- `reason` on a `decide` goes to the **ledger only**, bounded to 300
  characters with control characters removed. It is never shown to the
  agent: the hook that answers the agent gets a fixed sentence. Text from a
  device is never a prompt.
- `evidence` is the ledger's: the console's verdict on the session's last
  closed turn (F1c) and its last witnessed check. Show both next to the request.
- Per-device scope: `readOnly` may send `pending` only (every `decide` is an
  error); `readApprovals` and `full` may decide within the tier rule above
  (allow only low; deny anything). `full` does not yet grant anything more
  over this protocol.
- A `decide` is recorded on the desktop as `approval_decision` (actor `human`,
  `via.remote = {deviceId, label}`), answered to the hook, and the Inbox card
  leaves with the device's name. The desktop reads the request as it stands
  **now** — `decide` on an id no longer pending is an error.

## Push: "N decisions wait" (Web Push)

A device may ask to be knocked when decisions wait, so the human opens the
app instead of discovering the queue hours later. Standard Web Push, so a
PWA gets it with the browser's `PushManager` and no app store:

1. `push_info` → the desktop's VAPID public key (RFC 8292). Pass it as
   `applicationServerKey` to `registration.pushManager.subscribe({
   userVisibleOnly: true, applicationServerKey })`.
2. Send the resulting subscription with `subscribe` (`endpoint` must be
   https; `p256dh` is the 65-byte uncompressed P-256 point; `auth` 16
   bytes). The desktop stores it on the device's record and writes
   `remote_device {action: push_subscribed, host}` to the ledger — the host
   of the push service, never the endpoint (a capability URL).
3. The desktop pushes an RFC 8291 (`aes128gcm`) message whose plaintext is
   exactly `{"type":"decisions","pending":N}`, with `TTL: 120` and
   `Urgency: high`. The push service (Google, Apple, Mozilla) relays a
   count. It never sees a tool, a command, a path or a session name.
4. The service worker shows "N decisions wait"; the tap opens the app,
   which reconnects and asks `pending` — the decision is taken on the
   evidence, never from the notification.

When it knocks: a request is pushed only if it is **still pending 15 s
after it arrived** (the human at the keyboard answers in seconds), and a
device is knocked **at most once a minute** — a burst is one push with the
count. A `404`/`410` from the push service drops the subscription.
`unsubscribe` clears it; revoking the device clears it too.

iOS note: Web Push works for a PWA added to the Home Screen (iOS ≥ 16.4);
in Safari's tab it does not. Android and desktop browsers have no such
restriction.

## Security obligations for the device implementation

- Generate the identity keypair with a **CSPRNG**; store the private key in the
  **OS keystore** (Keychain / Keystore), never in plaintext.
- Treat the pinned desktop key as the source of truth; refuse a reconnect whose
  handshake doesn't authenticate it.
- Render `tier`, `tierLabel`, `tierReason` and `evidence` exactly as received;
  never offer **allow** when `remoteAllow` is false.
- An answer is one tap on a visible card after the evidence; no auto-answer,
  no "allow all".
- A push notification is a knock, not a decision: never offer allow/deny
  actions on the notification itself.

## Versioning

This is **v2** (decisions). v1 (voice) is retired; no desktop speaks it.
Breaking changes bump a version carried in a future `Hello` field. Keep
desktop and device in lockstep on the suite strings and message shapes.
