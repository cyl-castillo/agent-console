# Landing screenshots — capture spec (Track 3)

Goal: replace the zero-screenshot credibility gap on `docs/index.html`. Everything
below is turnkey so capture is the only manual step (gated on: who captures —
headed `tauri dev` locally, see [[tauri-dev-needs-interactive-shell]], or Carlos).

## Environment (since October 2026: Storybook, not the live app)
- Source: the built Storybook's stories with their fixtures — the demo project
  (`demo-playground`), no personal paths, no scrubbing. Playwright Chromium at
  the exact pixel size, device scale 1, dark theme (`--bg #0d0f12`).
- Pin the page clock to the fixtures' `NOW` (+25 s) so countdowns/durations read.
- Target < 250 KB each. Output to `docs/shots/` with the exact filenames below.

## Shots (current set, see `shots/README.md` for the story behind each)
1. **`hero-session.png`** — Sessions, live turn with the reply streaming in,
   terminal drawer at ~35 %, Inspector = turn detail.
2. **`approval-modal.png`** — Inbox decision cards by risk tier.
3. **`changes-diff.png`** — Changes tab, split diff, terminal hidden.
4. **`proof-verify.png`** — the hosted Testigo verifier (browser; unchanged).
5. **`ledger-map.png`** — Proof › Ledger map with a turn selected.

Candidates for a later set: a room mid-conversation (needs a fixture with a
real transcript), a roundtable, a worktree review.

## og:image cover — `og-cover.png` (1200×630)
- Hero shot #1 cropped to 1200×630, with wordmark + one-line tagline baked in.
- On commit, flip the two URLs already wired in `docs/index.html` head
  (`og:image` + `twitter:image`) from `logo.png` → `og-cover.png`. Alt text is
  already in place.

## Wiring into the page
- Add a screenshots band between the hero and the feature list. Each `<img>`:
  `loading="lazy"`, explicit `width`/`height` (avoid CLS), descriptive `alt`.
- Keep it a plain responsive grid; no lightbox/JS needed for v1.
