# Testigo — From Intent to Proof (draft v0.3, 2026-07-15)

> Nombre elegido: **Testigo** (colisión verificada: limpio). El working name anterior era "ProofFlow", que colisiona ×4.

Capa de confianza que conecta la intención humana original con la ejecución por agentes y la
evidencia verificable del resultado. No es task manager, ni CI/CD, ni agente, ni pagos.

---

## 1. Veredicto de la investigación (2026-07)

**Espacio: parcialmente ocupado, calentándose rápido, sin estándar adoptado. Ventana: ~6-12 meses.**

| Quién | Qué cubre de la cadena | Qué NO cubre | Tipo / madurez |
|---|---|---|---|
| in-toto / SLSA / sigstore | Provenance de build, atestaciones firmadas (DSSE) | Intención humana, aprobación, acciones de agentes | Estándar abierto maduro (CNCF) |
| W3C PROV / PROV-O | Modelo genérico quién-hizo-qué post-hoc | Nada nativo de agentes IA (PROV-AGENT es research 2025) | Estándar W3C, adopción académica |
| OTel GenAI / LangSmith / Langfuse | Telemetría de ejecución del agente | Evidencia firmable, aprobación, intención upstream | Maduro en observabilidad |
| Identidad/delegación 2025-26 (AIP, Agent Passport, HDP, KYA, APS) | Delegación humano→agente verificable, receipts | Cadena completa intención→evidencia→resultado | Papers arXiv, NADA adoptado |
| AER (Agent Execution Record) | Razonamiento del agente durante ejecución | Intención del cliente, reglas de negocio, aprobación | Paper 2026 + SDK preliminar |
| EU AI Act art. 12/14, ISO 42001 | **Demanda** esta trazabilidad (reconstrucción por decisión + registro de supervisión humana), high-risk desde ago 2026 | Es ley, no implementación | Viento de cola regulatorio |
| Claude Enterprise (Audit Logs + Compliance API) | 35 tipos de evento, retención 6 años, OTel | ⚠️ **NO captura sesiones locales** (gap documentado); solo su engine | Producto, enterprise-only |
| GitHub agentic audit log / Copilot | Eventos de agentes **hosteados en GitHub** | Agentes locales, otros engines, intención/aprobación | Producto |
| ProvenanceOne, aybruhm/provenance, Cursor Origin | Middleware de governance / git forge para agentes | Capa local del developer; POCs tempranos | Productos/POCs 2026 |
| Vanta/Drata, DOORS/ALM | Controls de compliance; requisito→test clásico | Acciones de agentes, evidencia por acción | Productos maduros, otro nivel |
| **Hyperion-GPU/ProofFlow** ⚠️ | Work contract → proof packet, MCP para Claude Code/Codex | Es tool standalone (hay que trabajar DENTRO de él), no protocolo; 48★ v0.1.8 | Competidor directo (y dueño del ex working name) |

**El hueco estructural** (la tesis de éxito): las plataformas auditan *su* lado — Anthropic audita su
API/enterprise pero no la sesión local; GitHub audita sus agentes hosteados; los observability venden
telemetría sin firma ni aprobación. **Nadie está parado en la máquina del humano, en el momento de la
aprobación, con visión multi-engine.** Ahí está agent-console, y ya captura ~80% de los insumos.

## 2. Estrategia de éxito

### Posicionamiento
- **Categoría** (lo que evangelizamos): *intent-to-proof* — "toda acción importante de un agente
  puede demostrar por qué existió, quién la aprobó y qué evidencia produjo".
- **Producto** (la cuña): el **proof packet** — un artefacto portable, firmado y verificable
  *sin agent-console*, que se adjunta a un PR, un release, una factura o un deploy.
- **Protocolo** (el foso): el formato del packet + la coreografía de captura, publicado como
  predicado in-toto (interoperar > inventar).

### Loop de adopción (el packet ES el marketing)
1. El packet se comparte hacia afuera (PR, cliente, auditor) con un verificador de un click.
2. Quien lo recibe ve la cadena intención→aprobación→evidencia y pregunta "¿cómo genero yo esto?".
3. Respuesta: agent-console. El verificador standalone es la puerta de entrada gratuita.

### Dogfooding ×2 (credibilidad antes que spec)
- **agent-console self-hosted**: cada PR del propio repo lleva su proof packet + commit trailer.
  El badge en el README y los PRs verificables SON la insignia.
- **Fixy como caso de estudio**: cada deploy de Fixy lleva packet (encaja con FIXY_AGENTIC_COMPANY:
  una empresa operada por agentes que puede *demostrar* lo que sus agentes hicieron). Es la historia
  de venta: "así opera una empresa agéntica auditable".

### Interoperar, no inventar
- Packet = in-toto Statement + DSSE envelope, predicado propio `…/attestation/v0.1` → verificable
  con tooling sigstore existente, historia EU AI Act art. 12 gratis.
- Commits de agentes con trailers `Proof-Case:` / `Agent-Session:` — alineado con la práctica
  emergente de git-auditing (trailers sobreviven rebases).
- Vocabulario del spec mapeado a art. 12/14 (reconstrucción por decisión, registro de supervisión
  humana) → cada feature es un checkbox regulatorio.

### Métricas de éxito (en orden)
1. Primer packet real: un deploy de Fixy o un PR de agent-console con packet verificable.
2. Un tercero verifica un packet **sin instalar agent-console** (verificador standalone funciona).
3. Primer usuario externo genera packets con su propio trabajo.
4. El spec público recibe su primer issue/PR de alguien que no somos nosotros.

### Riesgos y mitigaciones
| Riesgo | Mitigación |
|---|---|
| Plataformas subsumen (Anthropic/GitHub shippean session records firmados) | Vivir en el gap documentado (local + multi-engine + aprobación humana); velocidad: packet demoable en semanas, no spec en meses |
| Evidencia floja (intención sin resultado = ledger débil) | PostToolUse hook + diff pre/post turno son F1-F2, no opcionales |
| Sobre-prometer integridad | Honestidad en el spec: ledger local es **tamper-evident** (hash-chain), no tamper-proof; anclaje del head hash en git refs/notes como refuerzo barato |
| Privacidad (prompts contienen secretos/datos de cliente) | Pipeline de redacción + revisión humana ANTES de exportar (reusar la UX de approvals); niveles de detalle del packet |
| Spec-first trap (enamorarse del protocolo) | Regla: nada entra al spec que no esté implementado y usado por nosotros |

### Nombre: Testigo (decidido 2026-07-15)
El protocolo es el testigo presencial de la acción; "testigo firma, testigo declara" da el
vocabulario del producto. Check de colisión limpio (no existe software/protocolo "Testigo";
certigo/Testmo no confunden). Descartados por tomados: ProofFlow (×4: Hyperion-GPU + PyPI
`proofflow-mcp`, proofflow.online, proofflow.app, Huawei), ProofTrail (×2), IntentProof, Prova.
Pendiente para F5 (publicación): dominio (testigo.dev o similar) y handle de GitHub.
"Intent-to-proof" queda como nombre de la *categoría* en todo el copy.

## 3. Arquitectura v2 (endurecida)

### Unidad de correlación: el TURNO, agrupado en CASOS
- **Turn** = prompt → stop (los hooks ya marcan ambos bordes). Todo evento PreToolUse/PostToolUse
  se ata al turno abierto de su `termId`. Correlación honesta: por termId + ventana temporal —
  el spec lo declara (binding heurístico, no criptográfico, dentro de la sesión).
- **Case** (= `chain_id`) = hilo de intención: agrupa turnos de una o más sesiones. Nace de un
  ticket Jira (requisito), una nota, o el primer prompt; el usuario puede unir/partir casos.

### ProofEvent (ledger)
```
ProofEvent {
  seq, ts, case_id, turn_id, term_id, session_id,
  kind: prompt | approval_request | approval_decision | tool_use | tool_result
      | snapshot | turn_end | job_run | case_link | export,
  actor: { type: human|agent|scheduler, engine?, model? },
  payload,            // específico del kind
  prev_hash, hash     // hash-chain sobre JSON canónico (JCS)
}
```
- Persistencia: JSONL append-only por proyecto en `~/.local/share/agent-console/proof/`,
  calcado de `activity_service.rs` (atomic, crash-safe) pero **sin trim** (retención = valor).
- Anclaje barato: head hash publicado como `refs/agent-console/proof-head` y como trailer en
  commits del agente.

### Cambios concretos sobre lo existente
1. `hooks_service.rs::respond()` — dejar de borrar req/res de approvals; emitir
   `approval_request` + `approval_decision` (con reason) al ledger.
2. Nuevo `posttooluse-hook.cjs` (Claude Code y Codex soportan el mismo schema de hooks) —
   captura tool, exit/resultado truncado → `tool_result`.
3. `stop-hook.cjs` enriquecido — al cerrar turno: snapshot post-turno + lista de archivos
   cambiados vs snapshot pre-turno (git diff --name-status entre refs) → `turn_end`.
4. Jira: persistir `case_id ↔ ticket` al sembrar sesión desde un issue.
5. Scheduler: `RunRecord` a disco como `job_run` (hoy vive en memoria).

### Proof packet (export)
- Selección: un case (o rango de turnos) → **pipeline de redacción** (scan de secretos +
  revisión humana con la UX de approvals) → in-toto Statement con predicado propio →
  firma DSSE con clave ed25519 en keychain (misma infra que el token de Jira) → archivo
  `.proofpack` (zip: statement + evidencias referenciadas por hash).
- **Verificador standalone**: CLI mínimo + página HTML single-file que valida hash-chain +
  firma + integridad de evidencias. Sin agent-console. Es la pieza de confianza y de adquisición.

### Qué NO hace (guardrails de scope)
- No orquesta trabajo (no es workflow engine), no bloquea ejecución (captura, no gatekeeping —
  el gatekeeping ya existe: son los approvals), no sube nada a ningún servidor (local-first;
  compartir = exportar packet explícitamente).

## 4. Plan por fases (reordenado para momentum)

- **F1 — Ledger + turnos** ✅ (2026-07-15): `testigo_service.rs` hash-encadenado (torn-tail
  self-healing, verify, input acotado); approvals persistidas (request en watcher, decision en
  `approval_respond`); turnos prompt→stop por termId; case_id con rebind desde el ledger;
  vínculo Jira en `startSessionForIssue` (`testigo_link_case`). Comandos: `testigo_list`,
  `testigo_verify`, `testigo_link_case`. *Demo: timeline crudo de un case en JSON.*
- **F2 — Resultado** ✅ (2026-07-15): `posttooluse-hook.cjs` (observer auto-instalado, excerpt
  acotado 1KB) → eventos `tool_result` atados al turno; turn_end enriquecido con snapshot
  post-turno + `git diff --name-status` pre→post (cap 500 archivos, `filesTruncated` explícito);
  runs del scheduler al ledger como `job_run` bajo case `job:<id>`. stop-hook ahora pasa cwd
  (fallback para worktrees si se pierde el estado del turno).
  *Demo: un turno cuenta qué cambió.*
- **F3 — Packet + verificador** ✅ (2026-07-15, la insignia): `testigo_export.rs` — in-toto
  Statement v1 (predicado `https://github.com/cyl-castillo/testigo/attestation/v0.1`) firmado como DSSE ed25519
  (clave en keychain, keyid = sha256 del pubkey); case export con poda tipo Merkle (eventos
  fuera del case = stubs seq/hashes → linkage verifica entero); redacción conservadora de
  secretos (token-shaped patterns; evento redactado conserva hashes = linkage sí, contenido no,
  declarado); rechaza exportar cadena rota. `testigo-verifier.html` standalone (WebCrypto, cero
  deps, cero red) escrito junto a cada packet. Panel "Proof" mínimo (verify badge + cases +
  export). ⚠️ Commit trailers movidos a F4 (requieren noción de "case activo" por sesión que
  el tab de F4 introduce). Incluye hardening de approvals descubierto en la prueba GUI: la cola
  del modal ahora re-sincroniza desde disco (`approvals_pending`, attach + window focus) — un
  evento perdido ya no es un approval invisible + stall de 90s, los `.req.json` son la verdad.
- **F4 — UI "Proof" timeline + trailers** ✅ (2026-07-15): timeline por case en ProofPanel
  (click en case → turnos cronológicos: prompt, aprobaciones con razón, tool calls, archivos
  cambiados; export del case desde la vista); commit trailers `Testigo-Case:` en `git_commit` —
  atribución **por evidencia del ledger** (el turn_end más reciente ≤24h cuyo filesChanged
  intersecta los staged), nunca por sesión activa; best-effort, no bloquea el commit.
- **F5 — Spec público** ✅ (2026-07-15): **https://github.com/cyl-castillo/testigo** — SPEC.md
  v0.1 (ledger, hash chain, turnos/cases, packet DSSE, algoritmo de verificación, security
  considerations honestas, mapping EU AI Act art. 12/14), JSON Schema del predicado, verificador
  standalone, ejemplo sintético auto-verificado (con redacted + stub), MIT.
  Pendientes de F5: ⏳ **dominio testigo.dev** (el predicado ya usa esa URI — no necesita
  resolver para ser identificador válido, pero conviene poseerlo antes de difundir) y
  ⏳ **caso de estudio Fixy** (primer deploy de Fixy trabajado vía console → packet real como
  ejemplo canónico).

Cadencia estándar: plan → fase → commit → release por fase (`/phased-feature-build`).
