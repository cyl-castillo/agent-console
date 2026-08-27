# docs/shots/

Product screenshots for the landing page (`docs/index.html`, `#screens` band)
and the repository `README.md` (hero + gallery).

**The landing ships shot-less safely:** if a file is missing, its `<img>` removes
its own `<figure>`; if *none* of the shots exist, the whole `#screens` band and its
nav link hide on load. So you can swap, add, or remove shots freely — zero code
change on capture.

Shipped shots (August 2026 set, captured by Claude driving the live app — see below):

- `hero-session.png` — lead/hero: a full agent turn (failing test fixed, second
  bug found in the test runner) with the Proof panel's verified chain (12
  approvals · 38 events). Also the README's top image and the og-cover source.
- `approval-modal.png` — per-tool approval: an Edit stopped with its inline diff,
  Deny / Always / Approve once, keyboard hints, live countdown. Cropped to the
  center pane (the workbench panel showed user-specific skills).
- `changes-diff.png` — Changes tab: split diff, file list, commit box, inspector.
- `proof-verify.png` — the hosted Testigo verifier in Chrome validating the
  public demo packet: Ed25519/DSSE signature ✓, hash chain ✓, redaction notes.
- `room-multi-agent.png` — a Claude (Opus) + Codex room converging on a real
  geometry question, turn 6/6 with live token/cost counters.
- `og-cover.png` — 1200×630 social cover (wired into `og:image` /
  `twitter:image`), generated from the hero with the wordmark baked in — see
  the PIL snippet in the PR that shipped this set.

Capture spec (environment, sizing, dark theme, path-scrubbing): see
`../LANDING_SHOTLIST.md`.

**How these were captured (August 2026).** Same technique as the June set, plus
isolation so a second app instance can run while the daily instance is open:
launch with a scratch profile and a private bus (`XDG_{CONFIG,DATA,CACHE}_HOME`
+ `dbus-run-session` to dodge the single-instance plugin), `GDK_BACKEND=x11` to
land on XWayland, `GTK_THEME=Adwaita:dark` for the dark theme, and `env -u
CLAUDE*` so the spawned Claude CLI doesn't inherit a child-session marker. Drive
with `xdotool` (clicks/keys via XTEST — synthetic `--window` key events never
reach WebKit), capture with `xwd -id <wid> | ffmpeg`. Demo repo: a neutral
`orbit-api` with a deliberately failing test; everything on screen is real agent
work, not staged. Identity scrubbing (username/hostname/home paths in the shell
prompt and inspector) is done post-capture with PIL by painting the background
color over the affected line. Caveats: proof export can't run in the isolated
instance (no keyring on the private bus) — the verifier shot uses the public
demo packet from the `testigo` repo via Chrome + CDP `DOM.setFileInputFiles`.
