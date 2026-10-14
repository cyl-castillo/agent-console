import { describe, expect, it } from "vitest";

import {
  analyzeScreen,
  normalize,
  parseOption,
  parseYesNo,
  resolveVoice,
  stripLeading,
  type VoiceContext,
} from "./grammar";

const COMMANDS = [
  { id: "nav.terminal", label: "Open Terminal" },
  { id: "nav.changes", label: "Open Changes" },
  { id: "nav.proof", label: "Open Proof" },
  { id: "git.commit", label: "Open Git Commit" },
  { id: "git.open_pr", label: "Open Pull Request" },
  { id: "session.new", label: "New Session" },
  { id: "ui.toggle_theme", label: "Toggle Theme" },
  { id: "app.check_updates", label: "Check for Updates" },
];

const SESSIONS = [
  { id: "s1", name: "IA-21554", cwd: "/home/u/work/central" },
  { id: "s2", name: "shell 3", cwd: "/home/u/work/fixy-backend" },
  { id: "s3", name: "Revisar notas de IA-20600", cwd: "/home/u/work/central" },
];

function ctx(over: Partial<VoiceContext> = {}): VoiceContext {
  return {
    lang: "es",
    approvalPending: false,
    screen: { kind: "none" },
    sessions: SESSIONS,
    activeSessionId: "s1",
    commands: COMMANDS,
    ...over,
  };
}

describe("normalize / stripLeading", () => {
  it("drops diacritics, punctuation and leading verbs", () => {
    expect(normalize("Sí, mandálo.")).toBe("si mandalo");
    expect(stripLeading("abri la terminal")).toBe("terminal");
    expect(stripLeading("consola anda a la proof")).toBe("proof");
    expect(stripLeading("open the changes")).toBe("changes");
    expect(stripLeading("abri")).toBe("");
  });
});

describe("parseYesNo / parseOption", () => {
  it("a no anywhere beats a yes", () => {
    expect(parseYesNo("no, dale")).toBe("no");
    expect(parseYesNo("Sí")).toBe("yes");
    expect(parseYesNo("mmm")).toBe("unclear");
  });
  it("reads a bare option number, not a number inside a sentence", () => {
    expect(parseOption("opcion dos")).toBe(2);
    expect(parseOption("la tercera")).toBe(3);
    expect(parseOption("2")).toBe(2);
    expect(parseOption("los dos archivos")).toBeNull();
  });
});

describe("analyzeScreen", () => {
  it("recognizes Claude's permission menu", () => {
    const s = analyzeScreen([
      "Do you want to run this command?",
      "❯ 1. Yes",
      "  2. Yes, and don't ask again",
      "  3. No, and tell Claude what to do differently",
      "",
    ]);
    expect(s).toEqual({ kind: "claude-menu", options: 3 });
  });
  it("recognizes a y/N prompt and a generic numbered menu", () => {
    expect(analyzeScreen(["Apply changes? [y/N]"])).toEqual({ kind: "yn-prompt" });
    expect(analyzeScreen(["Pick one:", "1) fast", "2) safe", "3) both"])).toEqual({
      kind: "numbered-menu",
      options: 3,
    });
    expect(analyzeScreen(["$ ", "working…"])).toEqual({ kind: "none" });
  });
});

describe("resolveVoice: questions first", () => {
  it("yes/no answer a pending approval, dangerous or not (the executor gates it)", () => {
    expect(resolveVoice("sí", ctx({ approvalPending: true }))).toEqual({ kind: "approve" });
    expect(resolveVoice("no, pará", ctx({ approvalPending: true }))).toEqual({ kind: "deny" });
  });
  it("yes/no/option drive the terminal prompt on screen", () => {
    const menu = ctx({ screen: { kind: "claude-menu", options: 3 } });
    expect(resolveVoice("sí", menu)).toEqual({ kind: "keys", data: "\r", label: "yes" });
    expect(resolveVoice("no", menu)).toEqual({ kind: "keys", data: "\x1b", label: "no" });
    expect(resolveVoice("opción dos", menu)).toEqual({
      kind: "keys",
      data: "2",
      label: "option 2",
    });
    expect(resolveVoice("la cuarta", menu).kind).toBe("unknown");
    const yn = ctx({ screen: { kind: "yn-prompt" } });
    expect(resolveVoice("dale", yn)).toEqual({ kind: "keys", data: "y\r", label: "yes" });
    expect(resolveVoice("no", yn)).toEqual({ kind: "keys", data: "n\r", label: "no" });
  });
  it("yes with nothing asking is reported, never typed", () => {
    const a = resolveVoice("sí", ctx());
    expect(a).toEqual({ kind: "unknown", heard: "sí", reason: "nothing-to-confirm" });
    expect(resolveVoice("dos", ctx()).kind).toBe("unknown");
  });
});

describe("resolveVoice: keys, sessions, palette", () => {
  it("maps spoken keys to terminal bytes", () => {
    expect(resolveVoice("mandá", ctx())).toEqual({ kind: "keys", data: "\r", label: "enter" });
    expect(resolveVoice("Pará.", ctx())).toEqual({ kind: "keys", data: "\x1b", label: "escape" });
    expect(resolveVoice("control c", ctx())).toEqual({
      kind: "keys",
      data: "\x03",
      label: "ctrl+c",
    });
    expect(resolveVoice("abajo", ctx())).toEqual({ kind: "keys", data: "\x1b[B", label: "down" });
    expect(resolveVoice("send it", ctx({ lang: "en" }))).toEqual({
      kind: "keys",
      data: "\r",
      label: "enter",
    });
  });
  it("finds sessions by name or folder, partially", () => {
    expect(resolveVoice("sesión fixy", ctx())).toEqual({
      kind: "session",
      id: "s2",
      name: "shell 3",
    });
    expect(resolveVoice("cambiá a la sesión 21554", ctx())).toEqual({
      kind: "session",
      id: "s1",
      name: "IA-21554",
    });
    expect(resolveVoice("andá a backend", ctx())).toEqual({
      kind: "session",
      id: "s2",
      name: "shell 3",
    });
    expect(resolveVoice("sesión marte", ctx())).toEqual({
      kind: "unknown",
      heard: "sesión marte",
      reason: "no-match",
    });
  });
  it("names palette actions in Spanish, English, or by label", () => {
    expect(resolveVoice("abrí changes", ctx())).toEqual({
      kind: "palette",
      id: "nav.changes",
      label: "Open Changes",
    });
    expect(resolveVoice("mostrá la proof", ctx())).toEqual({
      kind: "palette",
      id: "nav.proof",
      label: "Open Proof",
    });
    expect(resolveVoice("nueva sesión", ctx())).toEqual({
      kind: "palette",
      id: "session.new",
      label: "New Session",
    });
    expect(resolveVoice("open pull request", ctx({ lang: "en" }))).toEqual({
      kind: "palette",
      id: "git.open_pr",
      label: "Open Pull Request",
    });
    expect(resolveVoice("cambiá el tema", ctx())).toEqual({
      kind: "palette",
      id: "ui.toggle_theme",
      label: "Toggle Theme",
    });
    expect(resolveVoice("buscar actualizaciones", ctx())).toEqual({
      kind: "palette",
      id: "app.check_updates",
      label: "Check for Updates",
    });
  });
  it("only offers actions that are available right now", () => {
    expect(resolveVoice("commit", ctx({ commands: [] })).kind).toBe("unknown");
  });
});

describe("resolveVoice: dictation", () => {
  it("types anything long enough that matched nothing, review-first", () => {
    expect(resolveVoice("corré los tests del backend", ctx())).toEqual({
      kind: "dictate",
      text: "corré los tests del backend",
      submit: false,
    });
  });
  it("a trailing 'y mandá' sends it", () => {
    expect(resolveVoice("arreglá el bug del login y mandá", ctx())).toEqual({
      kind: "dictate",
      text: "arreglá el bug del login",
      submit: true,
    });
    expect(resolveVoice("fix the login bug and send", ctx({ lang: "en" }))).toEqual({
      kind: "dictate",
      text: "fix the login bug",
      submit: true,
    });
  });
  it("'escribí:' forces dictation even for short or command-like text", () => {
    expect(resolveVoice("escribí: mandá", ctx())).toEqual({
      kind: "dictate",
      text: "mandá",
      submit: false,
    });
    expect(resolveVoice("Escribí, commit", ctx())).toEqual({
      kind: "dictate",
      text: "commit",
      submit: false,
    });
    expect(resolveVoice("escribí", ctx()).kind).toBe("unknown");
  });
  it("a one-word alias inside a long sentence is dictation, not a command", () => {
    expect(resolveVoice("hacé un commit con un mensaje claro", ctx()).kind).toBe("dictate");
  });
  it("short unmatched utterances are reported, not typed", () => {
    expect(resolveVoice("eh", ctx())).toEqual({ kind: "unknown", heard: "eh", reason: "no-match" });
    expect(resolveVoice("", ctx()).kind).toBe("unknown");
  });
});

describe("resolveVoice: help and silence", () => {
  it("answers 'what can I say' and 'quiet'", () => {
    expect(resolveVoice("¿qué puedo decir?", ctx())).toEqual({ kind: "help" });
    expect(resolveVoice("what can I say", ctx({ lang: "en" }))).toEqual({ kind: "help" });
    expect(resolveVoice("callate", ctx())).toEqual({ kind: "silence" });
  });
});
