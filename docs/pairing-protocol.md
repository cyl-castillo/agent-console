# Mobile Voice Companion — Wire Protocol

The contract between **agent-console** (desktop, the responder) and a **phone app**
(a separate repo — native or PWA; the initiator). Implement this exactly to
interoperate. The desktop reference implementation lives in
`src-tauri/src/services/transport.rs` (`client` module = the phone side).

See also: `docs/threat-model-voice-companion.md` (security rationale).

## Crypto

- **Noise Protocol Framework** (no hand-rolled crypto). Suites:
  - First-contact pairing: `Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s`
  - Reconnect (already paired): `Noise_KK_25519_ChaChaPoly_BLAKE2s`
- Identity = a long-term X25519 static keypair on each side. Private keys never
  leave the device.
- The desktop's static public key is delivered to the phone **out-of-band via the
  QR** — that's what makes the (untrusted) network un-MITM-able.

## Framing

Every message on the wire is length-prefixed:

```
[u32 big-endian length N][N bytes payload]
```

Max frame = 65536 bytes. Handshake messages and encrypted app messages are all
framed this way. A clean EOF between frames = orderly close.

## Pairing offer (QR payload)

The desktop shows a QR encoding `agentconsole://pair?d=<base64url(JSON)>`, where
the JSON is:

```json
{
  "rendezvousId": "<base64url, 16 random bytes>",
  "responderStatic": "<base64url of desktop X25519 public key>",
  "psk": "<base64url, 32 random bytes — one-time>",
  "createdMs": 1700000000000,
  "ttlSecs": 120
}
```

The offer is **one-time** and **short-lived** (TTL). The phone pins
`responderStatic` and uses `psk` as the Noise PSK.

## Connection flow

Each connection begins with a **cleartext** `Hello` frame (JSON) declaring intent.
It carries no secret (a public key and an intent are not sensitive); the handshake
authenticates.

```json
{ "type": "pair" }
{ "type": "reconnect", "key": "<base64url of phone's static public key>" }
```

### A. Pair (first contact)

1. Phone → `Hello{type:"pair"}`.
2. Noise **XXpsk3** handshake over framed messages (initiator = phone, writes
   first). Phone supplies its static key + the offer's PSK; desktop supplies its
   static key + PSK.
3. Phone **verifies** the desktop's transmitted static key equals the QR-pinned
   key (anti-MITM). Abort if mismatch.
4. Transport mode. Phone sends one E2E-encrypted frame = its **label** (UTF-8,
   e.g. "Carlos's iPhone").
5. Desktop registers a **pending** pairing and replies with an encrypted
   `"pending-approval"` frame, then closes.
6. The human approves on the desktop. Only then is the phone trusted.

### B. Reconnect (paired device)

1. Phone → `Hello{type:"reconnect", key:<its pubkey>}`.
2. Desktop looks the key up in its trust store. Unknown/revoked → it closes the
   connection (no handshake). 
3. Noise **KK** handshake (both sides already know each other's static key — phone
   pinned the desktop at pairing; desktop stored the phone on approval). No PSK.
4. Transport mode → the voice protocol below.

## Voice protocol (after reconnect)

Each app message is an E2E-encrypted frame whose plaintext is JSON.

Phone → desktop (`ClientMessage`):

```json
{ "type": "utterance", "text": "what's the build status?" }
{ "type": "ping" }
```

Desktop → phone (`ServerMessage`):

```json
{ "type": "say", "text": "The build is green; tests pass." }
{ "type": "pong" }
{ "type": "error", "message": "..." }
```

- **STT and TTS happen on the phone** — only text crosses the wire.
- The desktop runs the agent and returns a **short, speakable** reply.
- Request/response, one reply per message. The phone closes the stream when done
  (clean EOF ends the desktop's loop).

## Security obligations for the phone implementation

- Generate the identity keypair with a **CSPRNG**; store the private key in the
  **OS keystore** (Keychain / Keystore), never in plaintext.
- **Push-to-talk**, never always-on by default.
- Treat the pinned desktop key as the source of truth; refuse a reconnect whose
  handshake doesn't authenticate it.
- Dangerous/destructive actions must require an **on-device second factor**
  (not voice alone) — see the threat model. (The desktop currently only answers;
  agent-driving over voice is gated on this.)

## Versioning

This is **v1**. Breaking changes bump a version carried in a future `Hello`
field. Keep desktop and phone in lockstep on the suite strings and message shapes.
