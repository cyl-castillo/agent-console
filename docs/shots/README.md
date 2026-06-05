# docs/shots/

Product screenshots for the landing page (`docs/index.html`, `#screens` band).

**The page ships shot-less safely:** if a file is missing, its `<img>` removes its
own `<figure>`; if *none* of the shots exist, the whole `#screens` band and its nav
link hide on load. So the landing can merge today and the screenshot section simply
appears — figure by figure — as files land here. Zero code change on capture.

Shipped shots (captured by Opus driving the live app — see below):

- `room-multi-agent.png` — lead/hero: two Claude agents (Opus + Sonnet) in one shared room thread
- `worktree-review.png` — split git diff of the agents' work, file tree + inspector
- `room-setup.png` — room setup: agents, engine (Claude/Codex), models, roles, budgets, edit toggle
- `og-cover.png` — 1200×630 social cover (wired into `og:image` / `twitter:image`)

The page still degrades gracefully per-figure: a missing PNG drops its `<figure>`,
and an empty band hides itself. So you can swap, add, or remove shots freely.

Capture spec (environment, sizing, dark theme, path-scrubbing): see
`../LANDING_SHOTLIST.md`.

**How these were captured (June 2026).** GNOME/Wayland blocks programmatic
screenshots, so the run forced the app onto XWayland (`GDK_BACKEND=x11`), captured
each window with `xwd -id <wid> | ffmpeg` (rootless-XWayland safe — `x11grab` of the
root sees only black), and drove the UI with `xdotool` (mouse via `--window`,
keyboard via XTEST). The shots use a neutral `/tmp/taskflow` demo repo. Codex was
not authenticated at capture time, so the room ran two Claude models; re-shoot with
a Claude+Codex room once Codex is logged in if you want both engines in the feed.
