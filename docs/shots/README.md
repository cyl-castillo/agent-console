# docs/shots/

Product screenshots for the landing page (`docs/index.html`, `#screens` band).

**The page ships shot-less safely:** if a file is missing, its `<img>` removes its
own `<figure>`; if *none* of the shots exist, the whole `#screens` band and its nav
link hide on load. So the landing can merge today and the screenshot section simply
appears — figure by figure — as files land here. Zero code change on capture.

Drop the PNGs here with these exact filenames:

- `room-multi-agent.png` — lead/hero shot (the differentiator)
- `roundtable.png`
- `worktree-review.png`
- `file-preview.png`
- `mcp-engines.png`
- `og-cover.png` — 1200×630 social cover (then flip the `og:image` / `twitter:image` URLs in the head)

Capture spec (environment, sizing, dark theme, path-scrubbing): see
`../LANDING_SHOTLIST.md`.

**Decision (b) — settled: Opus (Claude) captures.** Requires a headed session on
Carlos's machine (`tauri dev` must run interactively — see
[[tauri-dev-needs-interactive-shell]]); not capturable from the automated room
worktree. Until then the band stays hidden and the rest of the page ships as-is.
