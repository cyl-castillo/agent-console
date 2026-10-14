/// Voice command grammar: a spoken transcript → one action, decided without a
/// model. Pure and synchronous so every rule is testable; the executor
/// (`execute.ts`) gathers the context from the stores and performs the action.
///
/// Three kinds of target, resolved in this order:
///   1. a pending question (approval modal, or a prompt on the terminal
///      screen) for "sí" / "no" / "opción 2";
///   2. a console command: keys into the agent's terminal (enter, esc,
///      arrows…), a session switch, or a palette action by name;
///   3. dictation: anything longer that matched nothing, typed into the
///      composer (review-first: no Enter unless the user said "mandá").
/// Short utterances that match nothing are reported, never typed — a stray
/// "eh" must not land in the prompt.

export type ScreenPrompt =
  /// Claude Code's permission / question menu (`❯ 1. Yes … 3. No`).
  | { kind: "claude-menu"; options: number }
  /// A `[y/N]`-style line prompt (Codex, shell scripts).
  | { kind: "yn-prompt" }
  /// Any other numbered list the agent is waiting on.
  | { kind: "numbered-menu"; options: number }
  | { kind: "none" };

export interface VoiceSession {
  id: string;
  name: string;
  cwd: string;
}

export interface VoiceCommandDef {
  id: string;
  label: string;
}

export interface VoiceContext {
  lang: string;
  /// An approval request the modal is showing (head of the queue).
  approvalPending: boolean;
  screen: ScreenPrompt;
  sessions: VoiceSession[];
  activeSessionId: string | null;
  /// Palette actions available right now.
  commands: VoiceCommandDef[];
}

export type VoiceAction =
  | { kind: "approve" }
  | { kind: "deny" }
  | { kind: "keys"; data: string; label: string }
  | { kind: "palette"; id: string; label: string }
  | { kind: "session"; id: string; name: string }
  | { kind: "dictate"; text: string; submit: boolean }
  | { kind: "help" }
  | { kind: "silence" }
  | { kind: "unknown"; heard: string; reason: "nothing-to-confirm" | "no-match" | "too-short" };

// --- Normalization -------------------------------------------------------------

/// Lowercase, no diacritics, no punctuation, single spaces. "Sí, mandálo." →
/// "si mandalo".
export function normalize(text: string): string {
  return text
    .toLowerCase()
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[^a-z0-9ñ\s]/g, " ")
    .split(/\s+/)
    .filter(Boolean)
    .join(" ");
}

const NO = new Set(["no", "nope", "cancela", "cancelar", "niega", "deny", "denegar", "para"]);
const YES = new Set([
  "si",
  "dale",
  "ok",
  "okay",
  "aprueba",
  "apruebo",
  "aprobar",
  "hazlo",
  "adelante",
  "claro",
  "confirmo",
  "yes",
  "approve",
  "sure",
]);

/// Tolerant yes/no parser. A negative anywhere wins over a positive
/// ("no, dale" must not approve).
export function parseYesNo(transcript: string): "yes" | "no" | "unclear" {
  const words = normalize(transcript).split(" ").filter(Boolean);
  if (words.some((w) => NO.has(w))) return "no";
  if (words.some((w) => YES.has(w))) return "yes";
  return "unclear";
}

const NUMBER_WORDS: Record<string, number> = {
  uno: 1,
  una: 1,
  primera: 1,
  primero: 1,
  first: 1,
  one: 1,
  dos: 2,
  segunda: 2,
  segundo: 2,
  second: 2,
  two: 2,
  tres: 3,
  tercera: 3,
  tercero: 3,
  third: 3,
  three: 3,
  cuatro: 4,
  cuarta: 4,
  cuarto: 4,
  fourth: 4,
  four: 4,
  cinco: 5,
  quinta: 5,
  quinto: 5,
  fifth: 5,
  five: 5,
  seis: 6,
  six: 6,
  siete: 7,
  seven: 7,
  ocho: 8,
  eight: 8,
  nueve: 9,
  nine: 9,
};

/// "opción dos" / "la segunda" / "2" → 2. Only when the whole utterance is
/// about picking a number, so a dictated "los dos archivos" is left alone.
export function parseOption(norm: string): number | null {
  const words = norm.split(" ").filter(Boolean);
  const filler = new Set([
    "opcion",
    "option",
    "la",
    "el",
    "elegi",
    "elige",
    "pick",
    "choose",
    "numero",
    "number",
  ]);
  const rest = words.filter((w) => !filler.has(w));
  if (rest.length !== 1) return null;
  const w = rest[0];
  if (/^\d$/.test(w)) return Number(w);
  return NUMBER_WORDS[w] ?? null;
}

// --- Screen reading ------------------------------------------------------------

/// What the agent's terminal is waiting on, from its last visible lines.
export function analyzeScreen(lines: string[]): ScreenPrompt {
  const tail = lines.map((l) => l.replace(/\s+$/, "")).filter((l) => l.trim() !== "");
  const numbered = tail.filter((l) => /^\s*(❯|›|>)?\s*\d+[.)]\s+\S/.test(l));
  const claudeYes = tail.some((l) => /^\s*(❯|›|>)?\s*1[.)]\s+Yes\b/i.test(l));
  const claudeAsk = tail.some((l) => /Do you want to|¿Quer[eé]s|Allow .* to\b/i.test(l));
  if (claudeYes || (claudeAsk && numbered.length >= 2)) {
    return { kind: "claude-menu", options: Math.max(numbered.length, 2) };
  }
  if (tail.some((l) => /\[(y\/n|y\/N|Y\/n)\]|\(y\/n\)|\(y\/N\)|\by\/n\s*[:?]?\s*$/i.test(l))) {
    return { kind: "yn-prompt" };
  }
  if (numbered.length >= 2) {
    return { kind: "numbered-menu", options: numbered.length };
  }
  return { kind: "none" };
}

// --- Keys into the terminal ----------------------------------------------------

interface KeyDef {
  data: string;
  label: string;
  phrases: string[];
}

const KEYS: KeyDef[] = [
  {
    data: "\r",
    label: "enter",
    phrases: [
      "manda",
      "mandalo",
      "mandala",
      "envia",
      "envialo",
      "enviar",
      "enter",
      "dale enter",
      "send",
      "send it",
      "submit",
      "go",
    ],
  },
  {
    data: "\x1b",
    label: "escape",
    phrases: [
      "para",
      "parala",
      "paralo",
      "detene",
      "detenlo",
      "stop",
      "cancela",
      "cancelalo",
      "escape",
      "interrumpi",
      "interrumpilo",
      "cortala",
      "cortalo",
      "salir",
      "cerra el menu",
    ],
  },
  {
    data: "\x03",
    label: "ctrl+c",
    phrases: ["control c", "control ce", "ctrl c", "matalo", "kill it"],
  },
  { data: "\x1b[A", label: "up", phrases: ["arriba", "subi", "up", "flecha arriba"] },
  { data: "\x1b[B", label: "down", phrases: ["abajo", "baja", "down", "flecha abajo"] },
  { data: "\x1b[D", label: "left", phrases: ["izquierda", "left"] },
  { data: "\x1b[C", label: "right", phrases: ["derecha", "right"] },
  { data: "\t", label: "tab", phrases: ["tab", "tabulador", "autocompleta", "autocomplete"] },
  { data: "\x7f", label: "backspace", phrases: ["borra", "borrar", "borra uno", "backspace"] },
  {
    data: "\x15",
    label: "clear line",
    phrases: ["limpia la linea", "borra la linea", "borra todo", "clear line", "clear the line"],
  },
  { data: " ", label: "space", phrases: ["espacio", "space"] },
];

// --- Palette aliases ---------------------------------------------------------

/// Spoken names per palette action, Spanish and English. The label itself
/// (minus a leading "Open") is always accepted too.
const ALIASES: Record<string, string[]> = {
  "nav.terminal": ["terminal", "la terminal"],
  "nav.changes": ["cambios", "changes"],
  "nav.preview": ["preview", "vista previa"],
  "nav.skills": ["skills", "habilidades"],
  "nav.permissions": ["permisos", "permissions"],
  "nav.advisor": ["advisor", "asesor"],
  "nav.vault": ["vault", "boveda"],
  "nav.context": ["contexto", "context", "memoria", "claude md"],
  "nav.plugins": ["plugins"],
  "nav.mcp": ["mcp", "servidores mcp", "mcp servers"],
  "nav.feedback": ["feedback"],
  "nav.tasks": ["tareas", "tasks"],
  "nav.teams": ["teams"],
  "nav.agenda": ["agenda"],
  "nav.notes": ["notas", "notes"],
  "nav.proof": ["proof", "evidencia", "testigo", "pruebas"],
  "nav.learning": ["learning", "aprendizaje"],
  "nav.roundtable": ["room", "sala", "roundtable", "mesa redonda"],
  "nav.jobs": ["jobs", "trabajos"],
  "nav.schedule": ["schedule", "agendador", "programador"],
  "nav.transfer": ["transfer", "transferencia"],
  "modules.configure": ["configurar modulos", "modulos", "configure modules", "modules"],
  "git.commit": ["commit", "hacer commit", "commitear", "git commit", "hace commit"],
  "git.push": ["push", "pushear", "push branch", "subi la rama", "subir la rama", "hace push"],
  "git.open_pr": ["pull request", "abri el pr", "abrir pr", "open pr", "pr", "abri un pr"],
  "git.refresh": [
    "refrescar git",
    "actualizar cambios",
    "refresh git",
    "refresh changes",
    "refresca los cambios",
  ],
  "session.new": ["nueva sesion", "sesion nueva", "new session", "crea una sesion"],
  "session.close": ["cerrar sesion", "cerra la sesion", "close session", "cerra esta sesion"],
  "ui.toggle_sidebar": ["sidebar", "barra lateral", "toggle sidebar"],
  "ui.toggle_right_panel": ["panel lateral", "panel derecho", "side panel", "right panel"],
  "ui.toggle_theme": [
    "tema",
    "cambiar tema",
    "cambia el tema",
    "theme",
    "toggle theme",
    "modo oscuro",
    "modo claro",
  ],
  "project.copy_path": ["copiar ruta", "copia la ruta", "copy path", "copy project path"],
  "auth.claude_login": [
    "login de claude",
    "arreglar login de claude",
    "arregla el login de claude",
    "fix claude login",
  ],
  "auth.codex_login": ["login de codex", "arregla el login de codex", "fix codex login"],
  "app.check_updates": [
    "buscar actualizaciones",
    "actualizaciones",
    "check updates",
    "check for updates",
  ],
  "app.report_problem": [
    "reportar problema",
    "reportar un problema",
    "report problem",
    "report a problem",
  ],
  "prompt.rerun_last": [
    "repetir el ultimo prompt",
    "repeti el prompt",
    "repeti el ultimo prompt",
    "rerun last prompt",
    "re run last prompt",
  ],
  "prompt.composer": ["composer", "compositor"],
  "setup.wizard": ["asistente de configuracion", "setup wizard", "wizard"],
  "snapshot.restore_latest": [
    "restaurar snapshot",
    "restaurar el ultimo snapshot",
    "restore snapshot",
    "restore latest snapshot",
  ],
  "snapshot.undo_restore": ["deshacer restauracion", "deshace la restauracion", "undo restore"],
  "help.getting_started": ["getting started", "primeros pasos"],
};

const LEADING_VERBS = [
  "consola",
  "abri",
  "abre",
  "abrir",
  "mostra",
  "mostrar",
  "muestra",
  "anda a",
  "anda al",
  "anda a la",
  "ir a",
  "ir al",
  "ve a",
  "vamos a",
  "vamos al",
  "pasa a",
  "pasa al",
  "cambia a",
  "cambia al",
  "cambia a la",
  "open",
  "show",
  "go to",
  "switch to",
  "the",
  "el",
  "la",
  "los",
  "las",
  "al",
  "un",
  "una",
];

/// Drop leading verbs/articles so "abrí la terminal" and "terminal" agree.
export function stripLeading(norm: string): string {
  let s = norm;
  let changed = true;
  while (changed) {
    changed = false;
    for (const v of LEADING_VERBS) {
      if (s === v) return "";
      if (s.startsWith(v + " ")) {
        s = s.slice(v.length + 1);
        changed = true;
      }
    }
  }
  return s;
}

function tokens(s: string): string[] {
  return s.split(" ").filter(Boolean);
}

function subset(small: string[], big: string[]): boolean {
  return small.every((t) => big.includes(t));
}

/// Match a stripped target against the palette: exact alias first, then an
/// alias whose words all appear in the target (longest alias wins).
function matchPalette(
  target: string,
  commands: VoiceCommandDef[],
): { id: string; label: string } | null {
  if (!target) return null;
  const byId = new Map(commands.map((c) => [c.id, c]));
  const candidates: { id: string; label: string; alias: string }[] = [];
  for (const c of commands) {
    const aliases = [...(ALIASES[c.id] ?? []), normalize(c.label.replace(/^Open\s+/i, ""))];
    for (const a of aliases) {
      if (a === target) return { id: c.id, label: c.label };
      candidates.push({ id: c.id, label: c.label, alias: a });
    }
  }
  const tt = tokens(target);
  const partial = candidates
    .filter((c) => subset(tokens(c.alias), tt))
    .sort((a, b) => b.alias.length - a.alias.length);
  const best = partial[0];
  if (!best || !byId.has(best.id)) return null;
  // An alias buried in a much longer sentence is dictation, not a command:
  // "hacé un commit con un mensaje claro" is a prompt, "abrí la proof por
  // favor" is a command. Two extra words of courtesy are allowed.
  if (tt.length > tokens(best.alias).length + 2) return null;
  return { id: best.id, label: best.label };
}

function basename(p: string): string {
  const parts = p.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] ?? p;
}

/// "sesión fixy" → the session whose name or folder contains the words.
function matchSession(target: string, sessions: VoiceSession[]): VoiceSession | null {
  const tt = tokens(target);
  if (tt.length === 0) return null;
  const scored = sessions
    .map((s) => {
      const hay = tokens(normalize(`${s.name} ${basename(s.cwd)}`));
      const exact = normalize(s.name) === target || normalize(basename(s.cwd)) === target;
      const hits = tt.filter((t) =>
        hay.some((h) => h === t || h.startsWith(t) || t.startsWith(h)),
      ).length;
      return { s, score: exact ? 100 : hits === tt.length ? hits : 0 };
    })
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score);
  return scored[0]?.s ?? null;
}

const SESSION_WORDS = ["sesion", "session", "sesion de", "session de"];
const DICTATE_PREFIXES = [
  "escribi",
  "escribe",
  "escribir",
  "dictado",
  "dicta",
  "anota",
  "type",
  "write",
  "dictate",
];
const SUBMIT_SUFFIXES = [
  "y manda",
  "y mandalo",
  "y envia",
  "y envialo",
  "mandalo",
  "manda",
  "and send",
  "send it",
  "enviar",
];
const HELP_PHRASES = [
  "que puedo decir",
  "que puedo hacer",
  "ayuda de voz",
  "ayuda",
  "comandos de voz",
  "what can i say",
  "voice help",
  "help",
];
const SILENCE_PHRASES = ["callate", "silencio", "shh", "basta", "quiet", "shut up", "hush"];

/// Minimum words for an unmatched utterance to count as dictation.
const DICTATION_MIN_WORDS = 3;

// --- Resolver ------------------------------------------------------------------

export function resolveVoice(transcript: string, ctx: VoiceContext): VoiceAction {
  const raw = transcript.trim();
  const norm = normalize(raw);
  if (!norm) return { kind: "unknown", heard: raw, reason: "too-short" };

  // Forced dictation: "escribí: …" types the rest, verbatim from the raw text.
  for (const p of DICTATE_PREFIXES) {
    if (norm === p) return { kind: "unknown", heard: raw, reason: "too-short" };
    if (norm.startsWith(p + " ")) {
      const idx = raw.toLowerCase().normalize("NFD").replace(/[̀-ͯ]/g, "").indexOf(p);
      const rest = raw.slice(idx + p.length).replace(/^[\s:,.]+/, "");
      return dictation(rest);
    }
  }

  if (SILENCE_PHRASES.includes(norm)) return { kind: "silence" };
  if (HELP_PHRASES.includes(norm) || HELP_PHRASES.includes(stripLeading(norm)))
    return { kind: "help" };

  // A pending question gets first claim on yes / no / option N.
  const yn = parseYesNo(norm);
  const option = parseOption(norm);
  if (ctx.approvalPending) {
    if (yn === "yes") return { kind: "approve" };
    if (yn === "no") return { kind: "deny" };
  }
  if (ctx.screen.kind !== "none") {
    const s = ctx.screen;
    if (option !== null && s.kind !== "yn-prompt") {
      if (option >= 1 && option <= s.options) {
        return { kind: "keys", data: String(option), label: `option ${option}` };
      }
      return { kind: "unknown", heard: raw, reason: "nothing-to-confirm" };
    }
    if (yn === "yes") {
      return s.kind === "yn-prompt"
        ? { kind: "keys", data: "y\r", label: "yes" }
        : { kind: "keys", data: "\r", label: "yes" };
    }
    if (yn === "no") {
      return s.kind === "yn-prompt"
        ? { kind: "keys", data: "n\r", label: "no" }
        : { kind: "keys", data: "\x1b", label: "no" };
    }
  } else if (tokens(norm).length <= 2 && (yn !== "unclear" || option !== null)) {
    // "sí" / "dos" with nothing asking: say so instead of guessing.
    // ("para" is also the escape key, handled below.)
    if (!KEYS.some((k) => k.phrases.includes(norm))) {
      return { kind: "unknown", heard: raw, reason: "nothing-to-confirm" };
    }
  }

  // Keys into the terminal.
  const stripped = stripLeading(norm);
  for (const k of KEYS) {
    if (k.phrases.includes(norm) || k.phrases.includes(stripped)) {
      return { kind: "keys", data: k.data, label: k.label };
    }
  }

  // Sessions: an explicit "sesión X", or "cambiá a X" matching a session.
  for (const w of SESSION_WORDS) {
    if (stripped.startsWith(w + " ")) {
      const target = stripLeading(stripped.slice(w.length + 1));
      const s = matchSession(target, ctx.sessions);
      if (s) return { kind: "session", id: s.id, name: s.name };
      return { kind: "unknown", heard: raw, reason: "no-match" };
    }
  }
  if (/^(cambia|pasa|anda|ve|switch|go) /.test(norm)) {
    const s = matchSession(stripped, ctx.sessions);
    if (s) return { kind: "session", id: s.id, name: s.name };
  }

  // Palette actions by name.
  const cmd = matchPalette(stripped, ctx.commands);
  if (cmd) return { kind: "palette", id: cmd.id, label: cmd.label };

  // Dictation for anything long enough to be a prompt.
  if (tokens(norm).length >= DICTATION_MIN_WORDS) return dictation(raw);
  return { kind: "unknown", heard: raw, reason: "no-match" };
}

/// Trailing "…y mandá" sends the dictated text; otherwise it waits for Enter.
function dictation(raw: string): VoiceAction {
  let text = raw.trim();
  let submit = false;
  const norm = normalize(text);
  for (const suf of SUBMIT_SUFFIXES) {
    if (norm.endsWith(" " + suf)) {
      // Cut the suffix off the raw text by word count (the raw and normalized
      // forms have the same word boundaries).
      const words = text.split(/\s+/);
      text = words.slice(0, words.length - suf.split(" ").length).join(" ");
      text = text.replace(/[\s,;:]+$/, "");
      submit = true;
      break;
    }
  }
  if (!text) return { kind: "unknown", heard: raw, reason: "too-short" };
  return { kind: "dictate", text, submit };
}

// --- Help ----------------------------------------------------------------------

export interface HelpGroup {
  title: string;
  phrases: string[];
}

export function helpGroups(lang: string): HelpGroup[] {
  const es = lang.startsWith("es");
  return es
    ? [
        {
          title: "Terminal",
          phrases: [
            "mandá",
            "pará",
            "control c",
            "arriba / abajo",
            "tab",
            "borrá",
            "limpiá la línea",
          ],
        },
        { title: "Preguntas en pantalla", phrases: ["sí", "no", "opción dos", "la tercera"] },
        {
          title: "Sesiones",
          phrases: ["sesión fixy", "cambiá a backend", "nueva sesión", "cerrá la sesión"],
        },
        {
          title: "Paneles",
          phrases: ["abrí changes", "proof", "permisos", "jobs", "notas", "tema"],
        },
        { title: "Git", phrases: ["commit", "push", "pull request", "refrescá los cambios"] },
        {
          title: "Dictar",
          phrases: ["escribí: …", "… y mandá", "(tres palabras o más se dictan solas)"],
        },
        { title: "Voz", phrases: ["callate", "qué puedo decir"] },
      ]
    : [
        {
          title: "Terminal",
          phrases: ["send", "stop", "ctrl c", "up / down", "tab", "backspace", "clear line"],
        },
        { title: "On-screen questions", phrases: ["yes", "no", "option two", "the third"] },
        {
          title: "Sessions",
          phrases: ["session fixy", "switch to backend", "new session", "close session"],
        },
        {
          title: "Panels",
          phrases: ["open changes", "proof", "permissions", "jobs", "notes", "theme"],
        },
        { title: "Git", phrases: ["commit", "push", "pull request", "refresh changes"] },
        {
          title: "Dictate",
          phrases: ["type: …", "… and send", "(three words or more are dictated)"],
        },
        { title: "Voice", phrases: ["quiet", "what can I say"] },
      ];
}

/// One short spoken line for "what can I say".
export function helpSpeech(lang: string): string {
  return lang.startsWith("es")
    ? "Podés decir: mandá, pará, sí, no, abrí changes, sesión y un nombre, nueva sesión, o escribí seguido de lo que quieras dictar."
    : "You can say: send, stop, yes, no, open changes, session and a name, new session, or type followed by what to dictate.";
}
