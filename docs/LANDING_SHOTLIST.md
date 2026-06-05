# Landing screenshots — capture spec (Track 3)

Goal: replace the zero-screenshot credibility gap on `docs/index.html`. Everything
below is turnkey so capture is the only manual step (gated on: who captures —
headed `tauri dev` locally, see [[tauri-dev-needs-interactive-shell]], or Carlos).

## Environment
- Window: 1440×900, dark theme (matches the landing palette `--bg #0d0f12`).
- Hide personal paths / repo names — use a neutral demo repo.
- Export 2× (retina) PNG, then downscale; target < 250 KB each (pngquant/oxipng).
- Output to `docs/shots/` with the exact filenames below.

## Shots (priority order)
1. **`room-multi-agent.png`** — a room with 2 agents + human mid-conversation,
   one agent's diff visible. This is THE differentiator; lead with it.
2. **`roundtable.png`** — twin-worktree debate view, moderator picking a winner.
3. **`worktree-review.png`** — per-agent worktree diff / review-before-merge UI.
4. **`file-preview.png`** — File preview tab on tree click.
5. **`mcp-engines.png`** — MCP servers + engine picker (Claude/Codex) visible.

## og:image cover — `og-cover.png` (1200×630)
- Hero shot #1 (room) cropped to 1200×630, with wordmark + one-line tagline baked in.
- On commit, flip the two URLs already wired in `docs/index.html` head
  (`og:image` + `twitter:image`) from `logo.png` → `og-cover.png`. Alt text is
  already in place.

## Wiring into the page
- Add a screenshots band between the hero and the feature list. Each `<img>`:
  `loading="lazy"`, explicit `width`/`height` (avoid CLS), descriptive `alt`.
- Keep it a plain responsive grid; no lightbox/JS needed for v1.
