# CLI Integration Watch — Claude Code & Codex

Documento vivo: qué superficies de los CLIs usa Agent Console, qué cambió upstream y
qué mejoras quedan por aplicar. Lo mantiene la tarea recurrente diaria `cli-integration-watch`
(scheduled task de Claude Code): cada corrida chequea versiones nuevas, agrega hallazgos al
backlog y aplica UNA mejora (PR, nunca merge/release sin OK de Carlos).

## Estado del watch

- **Última revisión completa**: 2026-08-17 (Claude `2.1.234` y Codex `0.147.0` analizados)
- **Baseline analizado**: Claude Code hasta `2.1.234` · Codex hasta `0.147.0`
- **Instalado local**: claude `2.1.218` · codex `0.144.3`

## Superficies que Agent Console depende hoy

| Superficie | Dónde |
|---|---|
| PTY interactivo: `claude --resume <id> --model <m>` / `codex resume <id>`; login repair `claude` / `codex login` | `src/agents/profiles.ts` |
| Headless: `claude -p --output-format stream-json` / `codex exec --json` (+ `codex exec resume <id>`), parsing de usage | `src-tauri/src/services/engine_runner.rs` |
| Headless permission flags: `--permission-mode plan\|acceptEdits`, `--dangerously-skip-permissions` | engine_runner, scheduler_service, advisor_service, learning_service |
| Hooks: 4 bridges (UserPromptSubmit/PreToolUse/PostToolUse/Stop) como `node "<path>"` en `~/.claude/settings.json` + espejo idéntico en `~/.codex/hooks.json` | `src-tauri/src/services/hooks_service.rs`, `src-tauri/resources/*.cjs` |
| Detección de errores de auth: `claude auth status --json` (2.1.41+, tolerante a ausencia) + heurística de texto como respaldo | `src-tauri/src/services/claude_cli.rs::auth_status`/`exit_error`, `engine_runner::finish` |
| Transcripts Claude `~/.claude/projects/<slug>/*.jsonl` (lectura tolerante) | context/semantic services |

**NO dependemos de**: archivos de sesión de Codex (`~/.codex/sessions`) — el cambio a `.jsonl.zst` (0.142) no nos afecta. Tampoco del permission mode `default` (renombrado a `manual` en 2.1.200).

## Backlog priorizado

### Quick wins (candidatos para la mejora diaria)

- [ ] **W1 — Hook exec-form `args` (Claude 2.1.139)**: reemplaza nuestro workaround `node "<path>"` (PR #109) por `{command, args: [path]}` sin shell. CUIDADO compat: versiones <2.1.122 invalidan settings.json entero ante entradas que no entienden → gatear por versión detectada (`claude --version`) o esperar adopción. Codex: quoting Windows arreglado en 0.145 con el formato actual.
- [x] **W2 — Auth detección estructurada** (PR #139, 2026-08-06): `claude auth status --json` → `claude_cli::auth_status()` (probe acotado a 8s, `None` = "no sé", nunca "deslogueado"); `exit_error` y `engine_runner::finish` nombran la expiración aunque el CLI culpe a otra cosa; "Fix Claude login" usa `claude auth login` cuando el probe prueba que el subcomando existe, con fallback a `claude` pelado. Queda como opcional: mostrar cuenta/método en la GUI (el comando IPC `claude_auth_status` ya los devuelve).
- [ ] **W3 — `sessionTitle` writeback (Claude 2.1.94)**: nuestro auto-naming puede devolver `hookSpecificOutput.sessionTitle` desde el UserPromptSubmit hook → el nombre queda también en `/resume` y `claude agents` del propio CLI. Campo ignorado por versiones viejas (seguro).
- [ ] **W4 — `last_assistant_message` en Stop (Claude 2.1.47)**: el Stop hook ya recibe el texto final del turno → habilita resumen hablado fin-de-turno (hito 3 de voz) y digest sin parsear jsonl. Codex: verificar si su Stop lo incluye (su set de eventos creció: sessionEnd, permissionRequest, subagentStart/Stop, postCompact).
- [x] **W5 — Trust de workspace** (PR pendiente, 2026-08-17): `workspace_trust.rs` lee los stores propios de cada CLI (`~/.claude.json` → `projects[dir].hasTrustDialogAccepted`; `~/.codex/config.toml` → `[projects."dir"] trust_level`), sube por ancestros hasta la raíz del repo y **para ahí** (2.1.232: un repo anidado ya no hereda el trust del padre). Store ausente/ilegible = `unknown`, nunca "desconfiado" (falsa alarma si el CLI jamás corrió acá). GUI: la pill de `integration` pasa a `untrusted` (visible con la sección colapsada) + hint explicando que los hooks no van a disparar. Comando `hooks_trust_status`.
- [ ] **W6 — Slug de transcripts para paths largos (Claude 2.1.224)**: upstream arregló que paths de proyecto >200 chars cayeran en el directorio de *otro* proyecto bajo un prefijo saneado compartido. Nosotros seguimos calculando el slug a mano (`abs.replace(['/','\\'], "-")` en `usage_service::transcripts_dir` y `context_service::memory_dir_for`) → para paths largos ahora apuntamos a un dir que el CLI ya no usa (usage vacío, memoria en el lugar equivocado). BLOQUEADO: no conocemos la codificación nueva (truncado/hash) y el claude local (2.1.218) es anterior al fix, así que no se puede verificar empíricamente sin actualizar el CLI. Mitigación candidata sin adivinar: resolver el dir por *descubrimiento* (escanear `~/.claude/projects` y quedarse con el que contenga sesiones cuyo `cwd` sea el proyecto) y usar el slug calculado solo como fallback/creación.

### Medianas

- [ ] **M1 — `claude agents --json` (2.1.145)**: fuente nativa de sesiones vivas (id, cwd, estado) para el sidebar — complementa/reemplaza la captura por hook.
- [ ] **M2 — `StopFailure` hook (Claude 2.1.78)**: señal estructurada de auth/rate-limit al terminar un turno → dispara el flujo "Fix login" proactivamente.
- [ ] **M3 — SessionEnd hook de Codex (0.145)**: señal de teardown que hoy no tenemos del lado Codex.
- [ ] **M4 — Approvals headless (Claude 2.1.85/2.1.89)**: PreToolUse `"defer"` + `AskUserQuestion` vía `updatedInput` → camino real a approvals con UI propia en sesiones headless (scheduler/advisor).
- [ ] **M5 — C4 parser Codex**: retarget a 0.145/0.146 — el riesgo real no fue 0.144 sino 0.145 (markdown incremental, output acotado, hyperlinks OSC 8 `\x1b]8;;url\x07`). Evaluar si el terminal xterm los degrada bien.

### Estratégicas

- [ ] **S1 — `codex app-server` como backbone**: protocolo completo (threads list/resume/fork, `thread/items/list`, hooks list + notificaciones, `review/start`, usage exacto, auth inyectable), daemon gestionado, bindings TS generables. Mata la fragilidad del parsing de TUI. Gran refactor — planificar aparte.
- [ ] **S2 — HTTP hooks (Claude 2.1.63)**: hooks que POSTean a localhost → el backend Tauri recibe eventos sin spawn de node por prompt ni scripts en disco. Evaluar junto con W1 (se pisan).
- [ ] **S3 — `codex review` scriptable**: `codex review --uncommitted|--base|--commit` corre por `codex exec` → integrable al flujo de review/roundtable. Ojo: churn de Guardian (0.144.2 revert, 0.146.1 defaults nuevos).
- [ ] **S4 — Background daemon Claude (`--bg`, 2.1.14x+)**: sesiones que sobreviven al terminal + hook `Notification` con `agent_needs_input`/`agent_completed` (2.1.198).

### Vigilar (sin acción aún)

- `CLAUDE_CODE_PROJECT_DIR_NAME` (2.1.234): un host puede elegir el nombre del dir de transcripts por proyecto. Nosotros no lo seteamos (y por eso leemos el slug por defecto), pero si algún día lo usamos, tenemos control explícito del path en vez de replicar la codificación de upstream — ver W6.
- Codex 0.147: `codex exec --full-auto` eliminado (no lo usamos: pasamos `--sandbox`/flags propios), `--approve-for-me` nuevo (approvals auto-revisadas; se pisa con M4), plugins portables + secciones de conversación persistentes, MCP 2026-07-28 opt-in, y **trust explícito para proyectos locales desconocidos** (#36960) — esto último es justo lo que W5 ahora detecta del lado Codex.
- Claude 2.1.232: cada repo git necesita su propia confirmación de trust (los anidados ya no heredan) → más directorios arrancan sin trust; W5 lo contempla parando el walk en la raíz del repo.
- Claude 2.1.233: `Notification` hooks no disparaban bajo Claude Desktop/VS Code (arreglado) — relevante para S4 si adoptamos `Notification`.

- Codex SQLite thread store (experimental 0.145/0.146): dirección anunciada para reemplazar rollouts JSONL — si se vuelve default, cambia cómo se capturan/resumen sesiones.
- Claude npm deprecado como método de install (2.1.15): binario nativo será la norma; revisar docs/onboarding que mencionen `npm install`.
- `codex login --api-key` hard-deprecado (usamos `codex login` interactivo — OK).
- SessionStart `source:"fork"` (2.1.214): hoy no usamos SessionStart; relevante si lo adoptamos.
- Matchers de hooks: hyphens exact-match (2.1.195), comma matchers (2.1.191) — nuestros hooks no usan matchers con guiones; re-verificar si agregamos.
- Binarios de codex ahora se sirven desde `releases.openai.com`/R2 con fallback GitHub (0.146).

## Historial de mejoras aplicadas

| Fecha | Item | PR |
|---|---|---|
| 2026-08-06 | W2 — auth estructurada (`claude auth status --json`) + `claude auth login` gateado por probe | #139 |
| 2026-08-17 | W5 — detección de trust de workspace (Claude + Codex) y aviso en la GUI cuando los hooks están instalados pero inertes | #148 |

## Protocolo de la tarea diaria

1. `npm view @anthropic-ai/claude-code version` y `npm view @openai/codex version`; comparar contra baseline. Si hay versión nueva: leer el delta del changelog (Claude: `raw.githubusercontent.com/anthropics/claude-code/main/CHANGELOG.md`; Codex: releases de `openai/codex`) y agregar al backlog lo que toque nuestras superficies.
2. Elegir UNA mejora desbloqueada (prioridad: breaking > quick win > mediana). Implementarla en branch `feat/cli-watch-<slug>`, con tests, gates verdes (`cargo fmt --check`, `clippy -D warnings`, `cargo test`, `npm run build`, `npm run format:check`).
3. Abrir PR (nunca mergear, nunca releasear — eso lo decide Carlos tras probar en GUI).
4. Actualizar este archivo (baseline, checkbox, historial) dentro del mismo PR.
5. Si no hay nada accionable: actualizar solo "Última revisión" y terminar barato.
