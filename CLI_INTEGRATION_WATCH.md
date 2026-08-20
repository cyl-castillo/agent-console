# CLI Integration Watch — Claude Code & Codex

Documento vivo: qué superficies de los CLIs usa Agent Console, qué cambió upstream y
qué mejoras quedan por aplicar. Lo mantiene la tarea recurrente diaria `cli-integration-watch`
(scheduled task de Claude Code): cada corrida chequea versiones nuevas, agrega hallazgos al
backlog y aplica UNA mejora (PR, nunca merge/release sin OK de Carlos).

## Estado del watch

- **Última revisión completa**: 2026-08-19 (versiones nuevas de los dos: Claude `2.1.237`, Codex `0.148.0`; delta leído y volcado al backlog)
- **Baseline analizado**: Claude Code hasta `2.1.237` · Codex hasta `0.148.0`
- **Instalado local**: claude `2.1.218` · codex `0.144.3`

## Superficies que Agent Console depende hoy

| Superficie | Dónde |
|---|---|
| PTY interactivo: `claude --resume <id> --model <m>` / `codex resume <id>`; login repair `claude` / `codex login` | `src/agents/profiles.ts` |
| Headless: `claude -p --output-format stream-json` / `codex exec --json` (+ `codex exec resume <id>`), parsing de usage | `src-tauri/src/services/engine_runner.rs` |
| Headless permission flags: `--permission-mode plan\|acceptEdits`, `--dangerously-skip-permissions` | engine_runner, scheduler_service, advisor_service, learning_service |
| Hooks: 4 bridges (UserPromptSubmit/PreToolUse/PostToolUse/Stop) como `node "<path>"` en `~/.claude/settings.json` + espejo idéntico en `~/.codex/hooks.json` | `src-tauri/src/services/hooks_service.rs`, `src-tauri/resources/*.cjs` |
| Entrada del hook Stop: `last_assistant_message` (2.1.47) → `summary` del turn_end en el ledger | `src-tauri/resources/stop-hook.cjs`, `hooks_service::handle_event` |
| Detección de errores de auth: `claude auth status --json` (2.1.41+, tolerante a ausencia) + heurística de texto como respaldo | `src-tauri/src/services/claude_cli.rs::auth_status`/`exit_error`, `engine_runner::finish` |
| Transcripts Claude `~/.claude/projects/<slug>/*.jsonl` (lectura tolerante) | context/semantic services |

**NO dependemos de**: archivos de sesión de Codex (`~/.codex/sessions`) — el cambio a `.jsonl.zst` (0.142) no nos afecta. Tampoco del permission mode `default` (renombrado a `manual` en 2.1.200).

## Backlog priorizado

### Quick wins (candidatos para la mejora diaria)

- [ ] **W1 — Hook exec-form `args` (Claude 2.1.139)**: reemplaza nuestro workaround `node "<path>"` (PR #109) por `{command, args: [path]}` sin shell. CUIDADO compat: versiones <2.1.122 invalidan settings.json entero ante entradas que no entienden → gatear por versión detectada (`claude --version`) o esperar adopción. Codex: quoting Windows arreglado en 0.145 con el formato actual.
- [x] **W2 — Auth detección estructurada** (PR #139, 2026-08-06): `claude auth status --json` → `claude_cli::auth_status()` (probe acotado a 8s, `None` = "no sé", nunca "deslogueado"); `exit_error` y `engine_runner::finish` nombran la expiración aunque el CLI culpe a otra cosa; "Fix Claude login" usa `claude auth login` cuando el probe prueba que el subcomando existe, con fallback a `claude` pelado. Queda como opcional: mostrar cuenta/método en la GUI (el comando IPC `claude_auth_status` ya los devuelve).
- [ ] **W3 — `sessionTitle` writeback (Claude 2.1.94)**: nuestro auto-naming puede devolver `hookSpecificOutput.sessionTitle` desde el UserPromptSubmit hook → el nombre queda también en `/resume` y `claude agents` del propio CLI. Campo ignorado por versiones viejas (seguro).
- [x] **W4 — `last_assistant_message` en Stop (Claude 2.1.47)** (PR #150, 2026-08-19): el bridge Stop guarda las palabras de cierre del agente (`summary` + `summaryTruncated`, cap 1000 chars, trim) y el turn_end del ledger las lleva junto al diff — el turno cierra con lo que el agente DICE que hizo al lado de la evidencia de lo que cambió. Segundo guard en Rust (`truncate_chars`) por si el script en disco es de un install viejo. Ausente ⇒ el evento es idéntico al de antes (Codex hoy no lo manda; tolerado por campo faltante, no por versión). El resumen se ve en el timeline de proof y entra en la review pre-firma (redactable como cualquier otro campo). Siguen abiertos, sobre este mismo dato: resumen hablado fin-de-turno (hito 3 de voz) y digest sin parsear jsonl.
- [ ] **W5 — Trust de workspace**: hooks NO corren en directorios no confiados (Claude 2.1.3/2.1.51/2.1.218) → detectar y avisar en la GUI en vez de asumir que el hook disparó (hoy el fallo es silencioso; misma clase de bug que el de Windows/Melissa).

### Medianas

- [ ] **M1 — `claude agents --json` (2.1.145)**: fuente nativa de sesiones vivas (id, cwd, estado) para el sidebar — complementa/reemplaza la captura por hook.
- [ ] **M2 — `StopFailure` hook (Claude 2.1.78)**: señal estructurada de auth/rate-limit al terminar un turno → dispara el flujo "Fix login" proactivamente.
- [ ] **M3 — SessionEnd hook de Codex (0.145)**: señal de teardown que hoy no tenemos del lado Codex.
- [ ] **M4 — Approvals headless (Claude 2.1.85/2.1.89)**: PreToolUse `"defer"` + `AskUserQuestion` vía `updatedInput` → camino real a approvals con UI propia en sesiones headless (scheduler/advisor).
- [ ] **M6 — Hooks asíncronos de Codex (0.148)**: los command hooks pueden correr async (y llamar tools MCP); `hooks list` ahora expone el execution mode. Nuestros 4 bridges son spawns sincrónicos de node en el camino del prompt → async saca esa latencia del lado Codex. Se pisa con S2 (HTTP hooks): decidir un solo camino de transporte antes de tocar el instalador.
- [ ] **M5 — C4 parser Codex**: retarget a 0.145/0.146 — el riesgo real no fue 0.144 sino 0.145 (markdown incremental, output acotado, hyperlinks OSC 8 `\x1b]8;;url\x07`). Evaluar si el terminal xterm los degrada bien.

### Estratégicas

- [ ] **S1 — `codex app-server` como backbone**: protocolo completo (threads list/resume/fork, `thread/items/list`, hooks list + notificaciones, `review/start`, usage exacto, auth inyectable), daemon gestionado, bindings TS generables. Mata la fragilidad del parsing de TUI. Gran refactor — planificar aparte.
- [ ] **S2 — HTTP hooks (Claude 2.1.63)**: hooks que POSTean a localhost → el backend Tauri recibe eventos sin spawn de node por prompt ni scripts en disco. Evaluar junto con W1 (se pisan).
- [ ] **S3 — `codex review` scriptable**: `codex review --uncommitted|--base|--commit` corre por `codex exec` → integrable al flujo de review/roundtable. Ojo: churn de Guardian (0.144.2 revert, 0.146.1 defaults nuevos).
- [ ] **S4 — Background daemon Claude (`--bg`, 2.1.14x+)**: sesiones que sobreviven al terminal + hook `Notification` con `agent_needs_input`/`agent_completed` (2.1.198).

### Vigilar (sin acción aún)

- Codex SQLite thread store (experimental 0.145/0.146): dirección anunciada para reemplazar rollouts JSONL — si se vuelve default, cambia cómo se capturan/resumen sesiones.
- Claude npm deprecado como método de install (2.1.15): binario nativo será la norma; revisar docs/onboarding que mencionen `npm install`.
- `codex login --api-key` hard-deprecado (usamos `codex login` interactivo — OK).
- SessionStart `source:"fork"` (2.1.214): hoy no usamos SessionStart; relevante si lo adoptamos.
- Matchers de hooks: hyphens exact-match (2.1.195), comma matchers (2.1.191) — nuestros hooks no usan matchers con guiones; re-verificar si agregamos.
- Binarios de codex ahora se sirven desde `releases.openai.com`/R2 con fallback GitHub (0.146).
- `ANTHROPIC_DEFAULT_MODEL` (2.1.236): fija el modelo de sesiones nuevas sin pisar un `/model` del usuario, a diferencia de `ANTHROPIC_MODEL`. Hoy pasamos `--model <m>` explícito por sesión (sigue siendo lo correcto); relevante si alguna vez queremos un default de proyecto blando.
- SIGTERM en modo print/SDK ya no registra turno interrumpido ni denials sintéticos, y sigue saliendo 143 (2.1.237): nuestro cancel de headless (`engine_runner`) queda más limpio en el transcript; verificar que seguimos tratando 143 como cancelación y no como fallo.
- `codex exec fork` (0.148): forkear una sesión desde headless — candidato para roundtable/worktrees, que hoy arrancan sesiones vacías.
- Codex 0.148 restaura cwd y approval policy persistidos al resumir: nuestro `codex resume <id>` hereda el estado correcto sin que le pasemos nada.

## Historial de mejoras aplicadas

| Fecha | Item | PR |
|---|---|---|
| 2026-08-06 | W2 — auth estructurada (`claude auth status --json`) + `claude auth login` gateado por probe | #139 |
| 2026-08-19 | W4 — el cierre de turno lleva las palabras del agente (`last_assistant_message` → `summary` en el ledger) | #150 |

## Protocolo de la tarea diaria

1. `npm view @anthropic-ai/claude-code version` y `npm view @openai/codex version`; comparar contra baseline. Si hay versión nueva: leer el delta del changelog (Claude: `raw.githubusercontent.com/anthropics/claude-code/main/CHANGELOG.md`; Codex: releases de `openai/codex`) y agregar al backlog lo que toque nuestras superficies.
2. Elegir UNA mejora desbloqueada (prioridad: breaking > quick win > mediana). Implementarla en branch `feat/cli-watch-<slug>`, con tests, gates verdes (`cargo fmt --check`, `clippy -D warnings`, `cargo test`, `npm run build`, `npm run format:check`).
3. Abrir PR (nunca mergear, nunca releasear — eso lo decide Carlos tras probar en GUI).
4. Actualizar este archivo (baseline, checkbox, historial) dentro del mismo PR.
5. Si no hay nada accionable: actualizar solo "Última revisión" y terminar barato.
