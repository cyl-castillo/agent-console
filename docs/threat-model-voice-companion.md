# Threat Model — Remote Decisions Client (was: Mobile Voice Companion)

**Status (2026-10-23, COMPETENCIA-2026-10 F8):** the desktop side is implemented and in `main` — Noise pairing (`secure_channel.rs`), pinned devices with scopes (`paired_devices_service.rs`), the listener (`transport.rs`, loopback by default, LAN by the human's typed choice) and the **decisions-only** application protocol (`remote_decisions.rs`, `docs/pairing-protocol.md` v2). The device app is a separate repo. **Voice is deferred**: nothing a device sends is a prompt until an on-device second factor exists (T5).
**Scope:** a paired device shows the decisions waiting on the user's own machine, with their evidence, and answers them.
**One-line framing:** this is a *remote control for an arbitrary-code executor*. Any weakness is RCE-grade — design as if the channel is already compromised, and keep dangerous actions behind the keyboard regardless.

## What is enforced today (desktop side)

| Control | Where |
|---|---|
| Noise XXpsk3 pairing with the desktop key pinned from the QR; KK reconnect; one-time PSK with TTL | `secure_channel.rs`, `transport.rs` |
| A handshake only creates a **pending** pairing; trust needs a click on the desktop, with a scope; revoke = forget the key (next connection refused before any handshake) | `paired_devices_service.rs`, Trust → Remote devices |
| **Dangerous actions are never approved from a device**: the desktop computes the tier (`tier_for`, same Bash rules as the Inbox card; every other tool medium) and `gate` refuses `allow` on a high tier whatever the scope; `deny` is always possible; `readOnly` devices may only look | `remote_decisions.rs` (tests: gate table + E2E over real TCP) |
| The tier a device displays is the desktop's; `decide` re-reads the request as it stands now | `transport.rs::serve_reconnect` |
| Listener bound to **127.0.0.1** by default; the LAN is a typed confirmation in the UI and a `remote_listener {bind: lan}` ledger line; the QR carries the dial address | `commands/remote.rs` |
| Every act goes to the ledger: `remote_device {paired|scope|revoked}`, `remote_listener {start|stop}`, and each remote answer is an `approval_decision` with `via.remote = {deviceId, label}`; export IETF maps them as decisions | `testigo_service.rs`, `aat_export.rs` |
| Desktop identity in the **OS keystore** (`keyring`), 0600 file only as fallback — and the UI says which | `secure_channel.rs::load_or_create_identity_keyring` |
| Frames capped at 64 KiB; the Inbox card leaves when a device answers (`approval://resolved`) so two humans never answer the same request | `transport.rs`, `approvalStore.ts` |

Not done (tracked): relay / outbound-only transport (today LAN or a tunnel the user brings), per-message replay nonces beyond Noise's own counters, on-device biometric gate (the device app's job), prompts over the channel (needs T5's second factor).

## System & data flow

```
🎤 Phone (push-to-talk) → on-device STT → [E2E-encrypted text] → Relay (blind cartero)
   → outbound conn → Desktop agent-console → inject prompt → claude/codex (tools: shell, git, fs, deploy)
   → spoken-summary pass → [E2E text] → Relay → Phone → TTS 🔊
```

- **Model B (relay):** desktop makes only **outbound** connections; nothing is exposed/bound to a public interface.
- **Audio + transcription stay on the phone**; only encrypted *text* crosses the wire.

## Assets (what an attacker wants)
Source code & repos · credentials (Claude auth, **vault secrets**) · the ability to **run shell / `git push` / deploy Fixy to prod** · the dev machine itself · (multi-user) the relay reaching *many* machines · the **release-signing keys** (auto-updater).

## Trust boundaries
`phone app` ⟂ `relay (untrusted infra)` ⟂ `desktop endpoint` ⟂ `agent + its tools` ⟂ `prod/secrets`.
The most important shift: voice **moves the human-approval boundary off the keyboard onto a noisy, identity-less channel.**

## Actors
Legit: device owner. Adversaries: relay operator / compromised relay · network MITM · thief of phone/token · **anyone within earshot** · malicious ambient/replayed audio · prompt-injection content · attacker of the release pipeline.

## Threats → controls

| # | Threat | Control |
|---|--------|---------|
| T1 | Relay reads/forges traffic | **E2E encryption**; relay sees only ciphertext. Messages **signed** so relay can't fabricate. TLS underneath. |
| T2 | Replay (resend a captured "approve") | Per-message **nonce + sequence + expiry**. |
| T3 | Attacker pairs to your agent | QR pairing with **ephemeral secret**, per-device keys, explicit confirm on desktop. |
| T4 | Stolen phone/token = full machine control | Biometric to open app · token **rotation** · **remote device revocation** · per-device scopes. |
| T5 | Dangerous action triggered remotely (deploy, `rm`, secrets) | **Never voice-only**: prod/secrets/destructive ops require **second factor** (on-screen tap + biometric). Aligns with the project's "confirm-before prod/secrets/external" policy. |
| T6 | Anyone near the phone speaks commands | **Push-to-talk only**, never always-on/wake-word by default. |
| T7 | Ambient / ultrasonic / replayed audio injection | Push-to-talk + on-screen confirm for anything irreversible. |
| T8 | STT mishears → wrong/destructive command | Confirm irreversible actions; echo the parsed intent before acting. |
| T9 | Desktop endpoint exposed to the internet | **Outbound-only**; never bind a public interface; validate & authenticate **every** message (don't trust the relay). |
| T10 | Prompt injection escalates via agent tools | Least-privilege agent scopes; never treat transcribed text as shell; keep tool sandboxing. |
| T11 | Secrets/code leak via cloud STT or spoken summaries | **On-device STT**; redact secrets from narrated output. |
| T12 | (Multi-user) relay is a high-value mass target | Stateless, E2E (no plaintext), rate-limit/abuse controls, minimize metadata retention. |
| T13 | (Multi-user) compromised release pipeline = mass RCE via auto-updater | **Sign releases**; protect signing keys; reproducible/verified builds. |

## Pairing — secure device binding (the trust anchor)

**Core insight:** trust must be bootstrapped **out-of-band**, because the relay is untrusted. The **QR shown on the desktop screen is that OOB channel** — authenticated by physical proximity (the attacker isn't looking at your monitor). If the QR carries the desktop's public key, the phone knows *who it's talking to before touching the relay* → **MITM becomes impossible**. **Do not invent crypto** — use the **Noise Protocol Framework** (à la WireGuard) or a **PAKE (SPAKE2, à la magic-wormhole)**. Primitives: X25519 (ECDH), Ed25519 (identity), HKDF, XChaCha20-Poly1305 (AEAD).

**Decisions (locked):** confirmation = **QR + explicit desktop approval** (SAS optional, off by default); a freshly paired phone's default scope = **read + small/reversible approvals** (prod/secrets/destructive always require the second factor of T5).

**Protocol**
1. Desktop (all local): long-term identity `ID_desktop` (created once, private key never leaves the machine) · an **ephemeral** pairing keypair `E` · a random rendezvous id `R` and high-entropy one-time secret `S`, **short TTL (~60s)**.
2. Desktop shows a QR — revealed only by an action on the **unlocked** desktop — containing `relay_url`, `R`, **pubkey/fingerprint of `E`**, `S`.
3. Phone scans → now authentically knows `E`'s pubkey + `S` (OOB satisfied).
4. Authenticated handshake (Noise_XX or SPAKE2 over the relay at `R`): phone **pins the QR key** so a relay-injected key won't match; `S` additionally **binds** the handshake; derive session key `K`; exchange a **MAC over the full transcript** — if it verifies, no tampering.
5. **Pin identities (TOFU done):** phone stores `ID_desktop`, desktop stores `ID_phone`; future sessions use these pinned keys (Noise_KK). `E` and `S` are discarded.
6. **Human approval on the desktop:** even after a clean handshake, *"Pair phone 'X'?"* requires a local click — the final safety net.

**Why attackers fail:** relay/MITM can't substitute keys (pinned from QR) or forge the transcript MAC · eavesdropper sees only ciphertext · replay blocked by one-time `R` + ephemeral `E` + TTL + MAC · `S` can't be brute-forced offline (PAKE → one online guess) + relay rate-limit · stolen phone mitigated by biometric keystore + remote revoke + scopes.

**Residual risk — the QR itself:** someone photographs the QR and races you. Layered mitigations: short TTL + one-time `R`/`S` (die the moment your phone finishes) · QR revealed only from the unlocked desktop · **the step-6 desktop approval** (a pairing you didn't approve never proceeds) · optional SAS (4-word code matching on both).

**Hardening:** desktop **device list + revocation** (each phone has its own pinned key → revoke = forget that key) · **per-device scopes** set at pairing.

## Defense-in-depth principles
1. E2E always — relay is a blind cartero. 2. Push-to-talk, never always-on. 3. Dangerous = strong second factor, never voice alone. 4. STT on-device; only text crosses the wire. 5. Per-device scopes + remote revocation. 6. Desktop outbound-only; never expose a port. 7. Sign releases / harden the updater.

## Assumptions & out of scope
**Assumes:** the user's desktop is running and trusted; the phone OS keypair/biometric store is sound; on-device STT is used. **Out of scope (for now):** cloud-hosted agent execution (Option A — running untrusted users' code is a separate, larger threat model); endpoint malware on an already-compromised desktop; supply chain of third-party deps.

## Open questions to resolve before build
- Exact second-factor UX for dangerous approvals (tap+biometric vs typed confirm)?
- Default device scope (read/notify only, vs. allow small approvals, vs. full)?
- Relay: rendezvous-only (P2P hole-punch, get out of the way) vs. always-forward?
- Self-hosted relay for power users, or only the hosted one?
