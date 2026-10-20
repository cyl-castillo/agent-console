# docs/shots/

Product screenshots for the landing page (`docs/index.html`, `#screens` band)
and the repository `README.md` (hero + gallery).

**The landing ships shot-less safely:** if a file is missing, its `<img>` removes
its own `<figure>`; if *none* of the shots exist, the whole `#screens` band and its
nav link hide on load. So you can swap, add, or remove shots freely — zero code
change on capture.

Shipped shots (October 2026 set — the v2 shell, captured from the Storybook
stories, see below):

- `hero-session.png` — lead/hero: Sessions with a live turn — prompt, tool
  calls, the agent's reply streaming in as markdown (table included), the
  running tool in the status line, the terminal in its drawer underneath, the
  Inspector holding the turn's detail. Also the README's top image and the
  og-cover source. Story `app-v2--sessions` + a `transcript://delta`.
- `approval-modal.png` — the Inbox with four decision cards graded by risk
  (read-only, irreversible with the typed "approve", an Edit with its inline
  diff), live countdowns. Story `app-v2--inbox-decisions`. (The filename is
  historical: the modal became cards in v0.88.)
- `changes-diff.png` — Changes tab: split diff, file list, commit box, file
  inspector; terminal drawer hidden. Story `app-v2--sessions-changes`.
- `proof-verify.png` — the hosted Testigo verifier in Chrome validating the
  public demo packet: Ed25519/DSSE signature ✓, hash chain ✓, redaction notes
  (August 2026 capture, unchanged — it is not the app's UI).
- `ledger-map.png` — Proof › Ledger: the map (a lane per session, a bar per
  turn, a tick per event) with the selected turn's events in the Inspector,
  chain verified. Story `app-v2--proof-turn-selected`.
- `og-cover.png` — 1200×630 social cover (wired into `og:image` /
  `twitter:image`), generated from the hero with the wordmark baked in (PIL:
  resize to 1200 wide, crop to 630, dark gradient over the lower half,
  DejaVu Sans Mono Bold wordmark + DejaVu Sans tagline).

Capture spec: see `../LANDING_SHOTLIST.md`.

**How these were captured (October 2026).** From the built Storybook
(`npm run build-storybook`, served on 127.0.0.1:6006) with Playwright's
Chromium at the exact pixel size (1534×831, 1213×831; device scale 1), on the
stories above — the same fixtures the visual sweep and the a11y gate run on, so
everything on screen is the demo project (`demo-playground`, user `demo`) and
nothing needs scrubbing. Details: the page clock is pinned to the fixtures'
`NOW` + 25 s so countdowns and durations read sensibly; the hero's drawer is
dragged down by a quarter of the centre so the thread and the terminal share
the screen; the F5 transcript delta is emitted through `window.__fixtures`.
The earlier sets (June/August 2026) were captured from the live app driven
with xdotool; that recipe is in the git history of this file.
