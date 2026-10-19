// Default fixture data: one realistic project ("demo-playground") with two
// sessions, a dirty working tree, a witnessed ledger of three turns, rules,
// skills, jobs and rooms — enough for every surface to render with content.
// Keep values deterministic (fixed timestamps) so screenshots are stable.
import type {
  ApprovalRequest,
  AvailableSnapshot,
  BranchInfo,
  ContextStatus,
  FeedbackContext,
  FileNode,
  FlywheelMetrics,
  GitStatus,
  HooksStatus,
  InstalledPlugin,
  Job,
  JobsBoard,
  McpServer,
  MemoryEntry,
  PermissionsSnapshot,
  PersistedSession,
  Preflight,
  Project,
  ProofEvent,
  RecentProject,
  RoomSummary,
  RunRecord,
  SessionUsage,
  Skill,
  StickyNote,
  TestigoSettings,
  TestigoVerifyReport,
  VaultEntryView,
  VoiceStatus,
  WorkspaceContext,
} from "../../types/domain";
import { emit } from "../tauri-shim/event";
import type { FixtureMap } from "./registry";

/// Fixed "now": 2026-10-13 14:00:00 UTC. Relative ages in the UI stay put.
export const NOW = 1792245600000;
const min = (n: number) => n * 60_000;
const h = (n: number) => n * 3_600_000;
const d = (n: number) => n * 86_400_000;

export const ROOT = "/home/demo/code/demo-playground";

export const project: Project = {
  root: ROOT,
  name: "demo-playground",
  language: "typescript",
  framework: "vite",
};

export const workspace: WorkspaceContext = {
  root: ROOT,
  language: "typescript",
  framework: "vite",
  fileCount: 184,
  packageScripts: ["dev", "build", "test", "lint"],
  entryPoints: ["src/main.ts"],
  readmePreview: "# demo-playground\n\nA small Vite app used to exercise Agent Console.",
};

export const tree: FileNode = {
  name: "demo-playground",
  path: ROOT,
  isDir: true,
  children: [
    {
      name: "src",
      path: `${ROOT}/src`,
      isDir: true,
      children: [
        { name: "api.ts", path: `${ROOT}/src/api.ts`, isDir: false },
        { name: "app.ts", path: `${ROOT}/src/app.ts`, isDir: false },
        { name: "main.ts", path: `${ROOT}/src/main.ts`, isDir: false },
        { name: "retry.ts", path: `${ROOT}/src/retry.ts`, isDir: false },
      ],
    },
    {
      name: "tests",
      path: `${ROOT}/tests`,
      isDir: true,
      children: [
        { name: "auth.test.ts", path: `${ROOT}/tests/auth.test.ts`, isDir: false },
        { name: "retry.test.ts", path: `${ROOT}/tests/retry.test.ts`, isDir: false },
      ],
    },
    { name: "CLAUDE.md", path: `${ROOT}/CLAUDE.md`, isDir: false },
    { name: "package.json", path: `${ROOT}/package.json`, isDir: false },
    { name: "README.md", path: `${ROOT}/README.md`, isDir: false },
  ],
};

export const sessions: PersistedSession[] = [
  {
    id: "sess-auth",
    agentId: "urn:agent-console:agent:00000000-0000-4000-8000-000000000001",
    runsIn: "local",
    allowedHosts: [".github.com", "registry.npmjs.org"],
    name: "fix auth test",
    cwd: ROOT,
    createdAtMs: NOW - h(3),
    lastActiveMs: NOW - min(4),
    scrollback: "",
    agent: "claude",
    agentSessionId: "0f2c1e4a-9b7d-4c1e-8a2f-5d6e7f8a9b0c",
    model: "claude-opus-5-5",
  },
  {
    id: "sess-retry",
    name: "retry path regression",
    cwd: ROOT,
    createdAtMs: NOW - h(26),
    lastActiveMs: NOW - h(25),
    scrollback: "",
    agent: "codex",
    agentSessionId: "thread_7Hq2n",
    worktree: {
      path: "/home/demo/.local/share/agent-console/worktrees/demo-playground/retry",
      branch: "ac/retry-path",
      baseBranch: "main",
    },
  },
];

export const gitStatus: GitStatus = {
  isRepo: true,
  branch: "main",
  changes: [
    { path: "src/api.ts", code: "M", staged: false, unstaged: true, untracked: false },
    { path: "tests/auth.test.ts", code: "M", staged: true, unstaged: false, untracked: false },
    { path: "src/retry.ts", code: "A", staged: false, unstaged: false, untracked: true },
  ],
};

export const branches: BranchInfo[] = [
  {
    name: "main",
    current: true,
    upstream: "origin/main",
    ahead: 1,
    behind: 0,
    lastCommitMs: NOW - h(2),
    lastSubject: "Fix expired token fixture in auth test",
  },
  {
    name: "ac/retry-path",
    current: false,
    upstream: null,
    ahead: 2,
    behind: 0,
    lastCommitMs: NOW - h(25),
    lastSubject: "Add retry regression test",
  },
];

export const diffApi = `diff --git a/src/api.ts b/src/api.ts
index 3f1c2a9..b7e4d10 100644
--- a/src/api.ts
+++ b/src/api.ts
@@ -12,9 +12,13 @@ export async function fetchUser(token: string) {
-  const res = await fetch(\`\${BASE}/me\`, { headers: { Authorization: token } });
-  if (!res.ok) throw new Error("auth failed");
-  return res.json();
+  const res = await fetch(\`\${BASE}/me\`, {
+    headers: { Authorization: \`Bearer \${token}\` },
+  });
+  if (res.status === 401) throw new AuthError("token expired");
+  if (!res.ok) throw new Error(\`auth failed: \${res.status}\`);
+  return (await res.json()) as User;
 }
`;

export const preflight: Preflight = {
  tools: [
    { name: "claude", found: true, version: "2.1.296", fixCommand: null, fixNote: null },
    { name: "codex", found: true, version: "0.162.1", fixCommand: null, fixNote: null },
    { name: "git", found: true, version: "2.51.0", fixCommand: null, fixNote: null },
    { name: "node", found: true, version: "22.23.3", fixCommand: null, fixNote: null },
  ],
  auth: [
    { engine: "claude", loggedIn: true, detail: "demo@example.com (Max)" },
    { engine: "codex", loggedIn: true, detail: "ChatGPT Pro" },
  ],
};

export const hooksStatus: HooksStatus = {
  sessionDir: "/home/demo/.claude",
  scriptPath: "/home/demo/.local/share/agent-console/hooks/user-prompt.cjs",
  pretooluseScriptPath: "/home/demo/.local/share/agent-console/hooks/pre-tool-use.cjs",
  installed: true,
  pretooluseInstalled: true,
  permissionrequestInstalled: true,
  permissionrequestSupported: true,
  notificationInstalled: true,
  statuslineInstalled: true,
  posttooluseInstalled: true,
  settingsPath: "/home/demo/.claude/settings.json",
  codexAvailable: true,
  codexInstalled: true,
  codexPretooluseInstalled: true,
  codexHooksPath: "/home/demo/.codex/hooks.json",
};

export const skills: Skill[] = [
  {
    name: "vitest-focus",
    kind: "skill",
    source: "project",
    path: `${ROOT}/.claude/skills/vitest-focus/SKILL.md`,
    description: "Run the one test file that covers the change before the whole suite",
    allowedTools: ["Bash(npm test:*)"],
    generatedAt: "2026-10-01T12:00:00Z",
    generatedBy: "learning",
  },
  {
    name: "api-error-contract",
    kind: "skill",
    source: "project",
    path: `${ROOT}/.claude/skills/api-error-contract/SKILL.md`,
    description: "Every API error carries a status and a typed class; never a bare string",
    allowedTools: [],
  },
  {
    name: "release-notes",
    kind: "command",
    source: "user",
    path: "/home/demo/.claude/commands/release-notes.md",
    description: "Draft release notes from the commits since the last tag",
    allowedTools: [],
  },
];

const CASE = "case-2026-10-13";
let seq = 0;
let prev = "0".repeat(64);
function ev(
  ts: number,
  kind: string,
  payload: Record<string, unknown>,
  turnId?: string,
  actor = "agent",
): ProofEvent {
  seq += 1;
  const hash = (seq * 2654435761).toString(16).padStart(8, "0").repeat(8).slice(0, 64);
  const e: ProofEvent = {
    seq,
    ts,
    caseId: CASE,
    turnId,
    kind,
    termId: "sess-auth",
    sessionId: sessions[0].agentSessionId,
    actor,
    payload,
    prevHash: prev,
    hash,
  };
  prev = hash;
  return e;
}

const T1 = "turn-001";
const T2 = "turn-002";
const T3 = "turn-003";
export const proofEvents: ProofEvent[] = [
  ev(NOW - h(2) - min(40), "session_start", { model: "claude-opus-5-5" }, T1),
  ev(
    NOW - h(2) - min(39),
    "prompt",
    { prompt: "Fix the failing user-auth test", cwd: ROOT },
    T1,
    "human",
  ),
  ev(NOW - h(2) - min(39), "context_injected", { docs: ["memory/auth-token-expiry.md"] }, T1),
  ev(NOW - h(2) - min(38), "tool_result", { tool: "Read", command: "tests/auth.test.ts" }, T1),
  ev(NOW - h(2) - min(37), "tool_result", { tool: "Edit", command: "src/api.ts" }, T1),
  ev(
    NOW - h(2) - min(36),
    "approval_decision",
    { tool: "Bash", decision: "allow", reason: "npm test" },
    T1,
    "human",
  ),
  ev(
    NOW - h(2) - min(35),
    "tool_result",
    { tool: "Bash", command: "npm test", excerpt: "✓ 14 passed", exitCode: 0 },
    T1,
  ),
  ev(
    NOW - h(2) - min(35),
    "check_run",
    { command: "npm test", status: "passed", exitCode: 0, durationMs: 4120 },
    T1,
  ),
  ev(
    NOW - h(2) - min(34),
    "turn_end",
    {
      filesChanged: [
        { status: "M", path: "src/api.ts" },
        { status: "M", path: "tests/auth.test.ts" },
      ],
      summary:
        "Fixed: the fixture token was expired. Also found a second bug in the test runner's retry path and left a note.",
    },
    T1,
  ),
  ev(
    NOW - h(1) - min(50),
    "prompt",
    { prompt: "Add a regression test for the retry path", cwd: ROOT },
    T2,
    "human",
  ),
  ev(NOW - h(1) - min(49), "tool_result", { tool: "Write", command: "tests/retry.test.ts" }, T2),
  ev(
    NOW - h(1) - min(48),
    "tool_result",
    { tool: "Bash", command: "npm test -- retry", excerpt: "✓ 3 passed", exitCode: 0 },
    T2,
  ),
  ev(
    NOW - h(1) - min(48),
    "check_run",
    { command: "npm test -- retry", status: "passed", exitCode: 0, durationMs: 1880 },
    T2,
  ),
  ev(
    NOW - h(1) - min(47),
    "commit",
    { sha: "a1b2c3d4e5f6", subject: "Add retry regression test", files: ["tests/retry.test.ts"] },
    T2,
  ),
  ev(
    NOW - h(1) - min(46),
    "turn_end",
    {
      filesChanged: [{ status: "A", path: "tests/retry.test.ts" }],
      summary: "Added tests/retry.test.ts covering the backoff ceiling and the give-up path.",
    },
    T2,
  ),
  ev(
    NOW - min(6),
    "prompt",
    { prompt: "Refactor retry.ts so backoff is configurable", cwd: ROOT },
    T3,
    "human",
  ),
  ev(NOW - min(5), "tool_result", { tool: "Read", command: "src/retry.ts" }, T3),
  ev(NOW - min(4), "tool_result", { tool: "Edit", command: "src/retry.ts" }, T3),
];

export const verifyReport: TestigoVerifyReport = {
  ok: true,
  total: proofEvents.length,
  tornTail: false,
};
export const testigoSettings: TestigoSettings = { witness: true, repoMarks: true };

export const pendingApproval: ApprovalRequest = {
  id: "apr-0042",
  ts: NOW - 12_000,
  sessionDir: "/home/demo/.claude",
  cwd: ROOT,
  tool: "Bash",
  input: { command: "npm test -- --run tests/retry.test.ts", description: "Run the retry tests" },
  termId: "sess-auth",
  timeoutMs: 55_000,
  source: "permission_request",
  permissionMode: "default",
  sessionId: sessions[0].agentSessionId,
};

/// A high-tier request (irreversible) and a medium one with a diff preview.
export const approvalHigh: ApprovalRequest = {
  ...pendingApproval,
  id: "apr-0043",
  ts: NOW - 4_000,
  input: {
    command: "rm -rf dist && git push --force origin main",
    description: "Clean and republish",
  },
  termId: "sess-retry",
};
/// D14: a fetch to a host outside the session's declared perimeter.
export const approvalFetch: ApprovalRequest = {
  ...pendingApproval,
  id: "apr-0045",
  ts: NOW - 2_000,
  tool: "WebFetch",
  input: { url: "https://pastebin.com/raw/k3yz", prompt: "Read the config snippet" },
  termId: "sess-auth",
};
export const approvalEdit: ApprovalRequest = {
  ...pendingApproval,
  id: "apr-0044",
  ts: NOW - 20_000,
  tool: "Edit",
  input: {
    file_path: `${ROOT}/src/retry.ts`,
    old_string: "const MAX_BACKOFF_MS = 8000;\nexport function retry(fn) {",
    new_string:
      "export interface RetryOptions {\n  maxBackoffMs?: number;\n}\nconst MAX_BACKOFF_MS = 8000;\nexport function retry(fn, opts: RetryOptions = {}) {",
  },
};

export const permissions: PermissionsSnapshot = {
  rules: [
    {
      scope: "project",
      effect: "allow",
      raw: "Bash(npm test:*)",
      source: "agent-console",
      createdAtMs: NOW - d(3),
      settingsPath: `${ROOT}/.claude/settings.json`,
      engine: "claude",
    },
    {
      scope: "project",
      effect: "allow",
      raw: "Read(**)",
      source: "agent-console",
      createdAtMs: NOW - d(3),
      settingsPath: `${ROOT}/.claude/settings.json`,
      engine: "claude",
    },
    {
      scope: "global",
      effect: "deny",
      raw: "Bash(rm -rf:*)",
      source: "external",
      createdAtMs: null,
      settingsPath: "/home/demo/.claude/settings.json",
      engine: "claude",
    },
    {
      scope: "project",
      effect: "allow",
      raw: "npm test",
      source: "agent-console",
      createdAtMs: NOW - d(1),
      settingsPath: `${ROOT}/.codex/rules/agent-console.rules`,
      engine: "codex",
    },
  ],
  projectSettingsPath: `${ROOT}/.claude/settings.json`,
  globalSettingsPath: "/home/demo/.claude/settings.json",
};

export const usage: SessionUsage = {
  contextTokens: 61_400,
  inputTotal: 412_000,
  outputTotal: 38_200,
  cacheReadTotal: 1_210_000,
  cacheCreationTotal: 96_000,
  contextWindow: 200_000,
};

export const contextStatus: ContextStatus = {
  projectClaudeMd: {
    path: `${ROOT}/CLAUDE.md`,
    exists: true,
    sizeBytes: 2140,
    modifiedMs: NOW - d(2),
  },
  globalClaudeMd: {
    path: "/home/demo/.claude/CLAUDE.md",
    exists: true,
    sizeBytes: 5320,
    modifiedMs: NOW - d(9),
  },
  memoryDir: {
    path: "/home/demo/.claude/projects/demo-playground/memory",
    exists: true,
    entryCount: 4,
  },
};

export const memories: MemoryEntry[] = [
  {
    name: "MEMORY.md",
    kind: null,
    description: "Index",
    sizeBytes: 640,
    modifiedMs: NOW - d(1),
    isIndex: true,
  },
  {
    name: "auth-token-expiry.md",
    kind: "project",
    description: "Fixture tokens expire after 30 days; regenerate with npm run fixtures",
    sizeBytes: 410,
    modifiedMs: NOW - d(1),
    isIndex: false,
    generatedBy: "learning",
  },
  {
    name: "retry-backoff.md",
    kind: "project",
    description: "Backoff ceiling is 8s; the give-up path throws RetryExhausted",
    sizeBytes: 388,
    modifiedMs: NOW - h(2),
    isIndex: false,
  },
  {
    name: "no-git-add-all.md",
    kind: "feedback",
    description: "Stage by explicit list; the root has untracked notes",
    sizeBytes: 220,
    modifiedMs: NOW - d(20),
    isIndex: false,
  },
];

export const vault: VaultEntryView[] = [
  {
    key: "OPENAI_API_KEY",
    scope: "project",
    description: "Codex API key (tests)",
    secret: true,
    hasValue: true,
    createdAtMs: NOW - d(12),
    updatedAtMs: NOW - d(12),
  },
  {
    key: "JIRA_TOKEN",
    scope: "global",
    description: "Jira Cloud API token",
    secret: true,
    hasValue: true,
    createdAtMs: NOW - d(40),
    updatedAtMs: NOW - d(5),
  },
];

export const installedPlugins: InstalledPlugin[] = [
  {
    id: "frontend-design@anthropic",
    name: "frontend-design",
    marketplace: "anthropic",
    version: "1.4.0",
    scope: "user",
    enabled: true,
    path: "/home/demo/.claude/plugins/frontend-design",
  },
  {
    id: "testigo@cyl-castillo",
    name: "testigo",
    marketplace: "cyl-castillo",
    version: "0.3.1",
    scope: "project",
    enabled: true,
    path: `${ROOT}/.claude/plugins/testigo`,
  },
];

export const availablePlugins: AvailableSnapshot = {
  marketplaces: ["anthropic", "cyl-castillo"],
  plugins: [
    {
      installId: "code-review@anthropic",
      name: "code-review",
      marketplace: "anthropic",
      description: "Multi-agent review of the current diff",
      author: "Anthropic",
      category: "quality",
      homepage: null,
    },
    {
      installId: "dataviz@anthropic",
      name: "dataviz",
      marketplace: "anthropic",
      description: "Charts that read as one system",
      author: "Anthropic",
      category: "design",
      homepage: null,
    },
  ],
};

export const notes: StickyNote[] = [
  {
    id: "n1",
    text: "Ask the agent to make backoff configurable before the release",
    color: "yellow",
    createdAtMs: NOW - h(5),
    updatedAtMs: NOW - h(5),
  },
  {
    id: "n2",
    text: "retry.test.ts: cover the 429 path too",
    color: "blue",
    createdAtMs: NOW - h(1),
    updatedAtMs: NOW - min(30),
  },
];

export const mcpServers: McpServer[] = [
  {
    name: "agent_console",
    scope: "project",
    transport: "stdio",
    command: "hook-bridge",
    args: "mcp --project demo-playground",
    url: null,
    env: [],
    status: "connected",
    connected: true,
  },
  {
    name: "github",
    scope: "user",
    transport: "http",
    command: null,
    args: null,
    url: "https://api.githubcopilot.com/mcp/",
    env: [],
    status: "connected",
    connected: true,
  },
];

export const feedbackContext: FeedbackContext = {
  appVersion: "0.87.0",
  os: "linux",
  projectName: "demo-playground",
  branch: "main",
};

export const jobs: Job[] = [
  {
    id: "job-nightly",
    name: "Nightly held-out tests",
    enabled: true,
    trigger: { type: "daily", hour: 3, minute: 0 },
    action: { type: "skill", name: "vitest-focus", args: "--held-out" },
    onMissed: "catchup",
    cooldownMs: h(6),
    createdAtMs: NOW - d(14),
    lastRunMs: NOW - h(11),
    nextDueMs: NOW + h(13),
    consecutiveFailures: 0,
  },
  {
    id: "job-digest",
    name: "Weekly curator pass",
    enabled: false,
    trigger: { type: "weekly", weekday: 1, hour: 9, minute: 0 },
    action: { type: "prompt", text: "Review the memory corpus for duplicates" },
    onMissed: "skip",
    cooldownMs: d(1),
    createdAtMs: NOW - d(30),
    consecutiveFailures: 2,
    backoffUntilMs: NOW + h(20),
  },
];

export const runHistory: RunRecord[] = [
  {
    jobId: "job-nightly",
    jobName: "Nightly held-out tests",
    startedMs: NOW - h(11),
    finishedMs: NOW - h(11) + min(4),
    status: "ok",
    summary: "17 held-out cases passed",
    outputExcerpt: "✓ 17 passed",
    tokens: 12_400,
    costUsd: 0.09,
  },
  {
    jobId: "job-digest",
    jobName: "Weekly curator pass",
    startedMs: NOW - d(7),
    finishedMs: NOW - d(7) + min(2),
    status: "failed",
    summary: "Plan mode refused a write",
    outputExcerpt: "blocked: Write(memory/*)",
    tokens: 3_100,
    costUsd: 0.02,
  },
];

export const rooms: RoomSummary[] = [
  {
    id: "room-backoff",
    problem: "Should backoff live in retry.ts or in the API client?",
    participantNames: ["Claude", "Codex"],
    messageCount: 9,
    lastTurn: 4,
    totalTokens: 48_900,
    updatedAtMs: NOW - h(3),
    jobMode: false,
    jobDone: false,
  },
  {
    id: "room-job-429",
    problem: "Handle 429 in the retry path with tests",
    participantNames: ["organizer", "implementer", "reviewer"],
    messageCount: 14,
    lastTurn: 7,
    totalTokens: 112_300,
    updatedAtMs: NOW - h(20),
    jobMode: true,
    jobDone: true,
  },
];

export const jobsBoard: JobsBoard = {
  project: ROOT,
  settings: { parallelJobs: 2 },
  cards: [
    {
      id: "room-job-429",
      problem: "Handle 429 in the retry path with tests",
      participantNames: ["organizer", "implementer", "reviewer"],
      status: "completed",
      column: "completed",
      phase: "completed",
      rank: 0,
      allowEdits: true,
      lastTurn: 7,
      updatedAtMs: NOW - h(20),
      live: false,
    },
    {
      id: "room-job-cfg",
      problem: "Make backoff configurable via RetryOptions",
      participantNames: ["organizer", "implementer", "reviewer"],
      status: "running",
      column: "running",
      phase: "implementing",
      rank: 1,
      allowEdits: true,
      lastTurn: 3,
      updatedAtMs: NOW - min(2),
      live: true,
    },
    {
      id: "room-job-docs",
      problem: "Document the retry contract in README",
      participantNames: ["organizer", "implementer"],
      status: "queued",
      column: "queued",
      phase: "kick-off",
      rank: 2,
      allowEdits: true,
      lastTurn: 0,
      updatedAtMs: NOW - min(30),
      live: false,
    },
    {
      id: "room-job-flaky",
      problem: "Stabilise the flaky auth timeout test",
      participantNames: ["organizer", "implementer", "reviewer"],
      status: "needs_attention",
      column: "needs_attention",
      reason: "reviewer did not record a verdict",
      phase: "reviewing",
      rank: 3,
      allowEdits: true,
      lastTurn: 5,
      updatedAtMs: NOW - h(1),
      live: false,
    },
  ],
  pending: [
    {
      pending: {
        id: "pend-1",
        sourceJobId: "room-job-cfg",
        creator: "organizer",
        instructions: "Add a changelog entry for RetryOptions",
        requestKey: "ck-1",
        status: "pending_approval",
        createdMs: NOW - min(9),
      },
      sourceProblem: "Make backoff configurable via RetryOptions",
    },
  ],
  busy: 1,
};

export const voice: VoiceStatus = {
  enabled: false,
  capturing: false,
  modelPresent: true,
  modelFile: "ggml-small-q5_1.bin",
  language: "es",
  ttsEngine: "piper",
  ttsVoice: "es_ES-davefx-medium",
};

export const flywheel: FlywheelMetrics = {
  corpusMemories: 4,
  corpusSkills: 3,
  excludedDocs: 0,
  injectionsTotal: 23,
  days: Array.from({ length: 14 }, (_, i) => ({
    startMs: NOW - d(13 - i),
    count: [0, 1, 2, 1, 3, 0, 0, 2, 4, 1, 2, 3, 2, 2][i],
  })),
  helpfulTotal: 9,
  unhelpfulTotal: 2,
  usefulnessPct: 82,
  verdictCoveragePct: 48,
};

export const recentProjects: RecentProject[] = [
  { path: ROOT, name: "demo-playground", lastOpenedMs: NOW - min(10) },
  { path: "/home/demo/code/fixy-app", name: "fixy-app", lastOpenedMs: NOW - d(1) },
  { path: "/home/demo/code/testigo", name: "testigo", lastOpenedMs: NOW - d(4) },
];

/// A short, colourful transcript so a mounted terminal is not an empty box.
export const demoTranscript = [
  "\x1b[90m$\x1b[0m claude --resume 0f2c1e4a\r\n",
  "\r\n",
  "\x1b[1m\x1b[38;5;215m✻ Welcome back\x1b[0m  claude-opus-5-5 · demo-playground\r\n",
  "\r\n",
  "\x1b[38;5;75m>\x1b[0m Refactor retry.ts so backoff is configurable\r\n",
  "\r\n",
  "\x1b[32m✓\x1b[0m Read \x1b[36msrc/retry.ts\x1b[0m\r\n",
  "\x1b[32m✓\x1b[0m Edit \x1b[36msrc/retry.ts\x1b[0m  \x1b[90m+18 −6\x1b[0m\r\n",
  "\x1b[33m▸\x1b[0m Bash \x1b[90mnpm test -- --run tests/retry.test.ts\x1b[0m\r\n",
  "\r\n",
  "\x1b[90m  waiting for approval…\x1b[0m\r\n",
].join("");

export const defaultFixtures: FixtureMap = {
  // project
  current_project: project,
  app_build_info: { commit: "7028aa9", buildTimeSecs: 1792238400, debug: false, snap: false },
  open_project: project,
  read_tree: tree,
  read_file_text: {
    content: diffApi,
    isBinary: false,
    sizeBytes: diffApi.length,
    truncated: false,
  },
  workspace_context: workspace,
  projects_recent: recentProjects,
  projects_last: recentProjects[0],
  projects_forget: undefined,
  projects_remember: undefined,
  set_active_repo: undefined,
  ledger_root_for: ROOT,
  linked_folders_list: [],
  worktree_setup_get: { copy: [".env"], setup: "npm install" },
  worktree_prune_orphans: [],
  worktree_status: { dirtyFiles: 0, ahead: 2, behind: 0 },
  // sessions & terminal
  sessions_list: sessions,
  sessions_save: undefined,
  session_usage: usage,
  term_agent_sessions: [],
  term_spawn: (args: Record<string, unknown>) => {
    const id = `term-${String(args.termKey ?? "x")}`;
    window.setTimeout(() => void emit("term://output", { id, data: demoTranscript }), 60);
    return id;
  },
  term_write: undefined,
  term_resize: undefined,
  term_kill: undefined,
  claude_session_exists: true,
  // git
  git_status: gitStatus,
  git_branches: branches,
  git_diff_file: diffApi,
  git_recent_messages: ["Fix expired token fixture in auth test", "Add retry regression test"],
  git_head_message: "Fix expired token fixture in auth test",
  git_file_log: [],
  git_pr_url: "https://github.com/demo/demo-playground/pull/41",
  // doctor, hooks, auth
  preflight_check: preflight,
  claude_auth_status: { loggedIn: true, method: "oauth", account: "demo@example.com" },
  hooks_status: hooksStatus,
  // trust
  approvals_pending: [],
  permissions_snapshot: permissions,
  vault_list: vault,
  // proof
  testigo_list: proofEvents,
  testigo_verify: verifyReport,
  testigo_get_settings: testigoSettings,
  testigo_public_key: { keyId: "ac-demo-2026", publicKey: "MCowBQYDK2VwAyEA…" },
  testigo_export_preview: {
    caseId: "case-2026-10-13",
    entries: proofEvents.map((e) => ({
      seq: e.seq,
      ts: e.ts,
      kind: e.kind,
      actor: e.actor,
      stub: false,
      autoRedacted: false,
    })),
  },
  testigo_export: {
    path: "/home/demo/.local/share/agent-console/proofpacks/case-2026-10-13.json",
    verifierPath: "/home/demo/.local/share/agent-console/proofpacks/verify.html",
    eventCount: proofEvents.length,
    stubCount: 0,
    redactionCount: 0,
    keyId: "ac-demo-2026",
    subjectDigest: "sha256:7e3a1c…",
    chainOk: true,
  },
  harness_lab_get: {
    config: {
      metric: "heldout_pass_rate",
      editable: ["CLAUDE.md", "AGENTS.md", ".claude/skills/", ".claude/agents/", ".claude/hooks/"],
      budget: { idleMinutes: 10, maxTokens: 400000 },
      focus: "",
    },
    heldout: {
      version: 2,
      tasks: [
        {
          id: "t1",
          prompt: "Add a retry test for the backoff ceiling",
          check: "npm test -- --run",
        },
        { id: "t2", prompt: "Fix the lint warnings in src/api.ts", check: "npx eslint src/api.ts" },
        { id: "t3", prompt: "Document the vault in README", check: "grep -q Vault README.md" },
      ],
    },
    experiments: [
      {
        id: "hx-1792000000-001",
        parentId: null,
        createdMs: NOW - 2 * 86_400_000,
        finishedMs: NOW - 2 * 86_400_000 + 600_000,
        metric: "heldout_pass_rate",
        hypothesis: "Name the test command in CLAUDE.md so the agent runs it before claiming done",
        branch: "harness/hx-1792000000-001",
        status: "kept",
        stopReason: null,
        heldoutVersion: 2,
        selectedTasks: ["t1", "t2", "t3"],
        baseline: { passed: 1, total: 3, tasks: [] },
        score: { passed: 2, total: 3, tasks: [] },
        filesChanged: ["CLAUDE.md"],
        outsideFiles: [],
        tokens: 41_200,
        costUsd: 0.31,
        prUrl: "https://github.com/demo/demo-playground/pull/41",
        commit: "a1b2c3d4",
      },
      {
        id: "hx-1792086400-002",
        parentId: "hx-1792000000-001",
        createdMs: NOW - 86_400_000,
        finishedMs: NOW - 86_400_000 + 420_000,
        metric: "heldout_pass_rate",
        hypothesis: "Add a lint skill",
        branch: "harness/hx-1792086400-002",
        status: "reverted",
        stopReason: "no improvement: 2/3 vs baseline 2/3",
        heldoutVersion: 2,
        selectedTasks: ["t2", "t3", "t1"],
        baseline: { passed: 2, total: 3, tasks: [] },
        score: { passed: 2, total: 3, tasks: [] },
        filesChanged: [".claude/skills/lint/SKILL.md"],
        outsideFiles: [],
        tokens: 38_000,
        costUsd: 0.28,
        prUrl: null,
        commit: "b2c3d4e5",
      },
    ],
    clean_runs: 2,
    kept: 1,
  },
  harness_lab_set_config: {
    metric: "heldout_pass_rate",
    editable: ["CLAUDE.md"],
    budget: { idleMinutes: 10, maxTokens: 400000 },
    focus: "",
  },
  harness_lab_set_heldout: { version: 3, tasks: [] },
  harness_lab_run: {
    id: "hx-1792172800-003",
    parentId: "hx-1792000000-001",
    createdMs: NOW,
    finishedMs: NOW + 300_000,
    metric: "heldout_pass_rate",
    hypothesis: "Ask for prettier before every commit",
    branch: "harness/hx-1792172800-003",
    status: "kept",
    stopReason: null,
    heldoutVersion: 2,
    selectedTasks: ["t3", "t1", "t2"],
    baseline: { passed: 2, total: 3, tasks: [] },
    score: { passed: 3, total: 3, tasks: [] },
    filesChanged: ["CLAUDE.md"],
    outsideFiles: [],
    tokens: 35_000,
    costUsd: 0.26,
    prUrl: "https://github.com/demo/demo-playground/pull/42",
    commit: "c3d4e5f6",
  },
  transcript_watch: null,
  transcript_unwatch: null,
  sponsor_get: {
    objective: "",
    owner: "",
    weeklyBudgetUsd: 0,
    riskThreshold: "low",
    autoApprove: false,
  },
  sponsor_set: {
    objective: "Ship billing v2 without regressions",
    owner: "Carlos",
    weeklyBudgetUsd: 40,
    riskThreshold: "low",
    autoApprove: true,
  },
  identity_card: {
    card: {
      type: "https://agent-console.dev/agent-card/v0.1",
      agentId: "urn:agent-console:agent:00000000-0000-4000-8000-000000000001",
      name: "fix auth test",
      kind: "session",
      engine: "claude",
      model: "claude-opus-5-5",
      principal: { principalType: "human", principalId: "urn:agent-console:user:local" },
      runsIn: "local",
      cwd: ROOT,
      allowedHosts: [".github.com"],
      createdAtMs: NOW,
      issuer: "agent-console demo",
    },
    signature: "c2ln",
    keyId: "ac-demo-2026",
    publicKey: "MCowBQYDK2VwAyEA…",
  },
  identity_verify: true,
  review_turn: {
    verdict: "block",
    findings: [
      {
        severity: "block",
        rule: "test_edited",
        title: "1 test file changed together with 1 source file",
        detail: "The prompt did not ask for test changes.",
        file: "tests/auth.test.ts",
        deterministic: true,
      },
    ],
    reviewer: "reviewer:codex",
    reviewerEngine: "codex",
    reviewerModel: null,
    producerModel: "claude-opus-5-5",
    termId: "sess-auth",
    turnId: "turn-001",
    caseId: "case-2026-10-13",
    files: 2,
    tokens: 812,
    costUsd: 0.01,
    rationale: "The assertion in tests/auth.test.ts was loosened from toEqual to toBeTruthy.",
    seq: 777,
  },
  review_decide: {
    seq: 778,
    ts: NOW,
    caseId: "case-2026-10-13",
    kind: "review_decision",
    actor: "human",
    payload: { decision: "accept" },
    prevHash: "",
    hash: "",
  },
  testigo_budget_event: {
    seq: 999,
    ts: NOW,
    caseId: "case-2026-10-13",
    kind: "budget_reached",
    actor: "console",
    payload: { spentUsd: 10, capUsd: 10, action: "interrupted" },
    prevHash: "",
    hash: "",
  },
  testigo_export_aat: {
    dir: "/home/demo/.local/share/agent-console/proofpacks/aat/case-2026-10-13",
    sessions: 1,
    records: proofEvents.length + 2,
    files: ["sess-1.jsonl", "index.json", "README.md", "aat-verify.mjs"],
    draft: "draft-sharif-agent-audit-trail-08",
  },
  testigo_verify_aat: [
    [
      "/home/demo/.local/share/agent-console/proofpacks/aat/case-2026-10-13/sess-1.jsonl",
      { ok: true, records: proofEvents.length + 2, brokenAt: null, tailClosed: true, problems: [] },
    ],
  ],
  // coach
  skill_list: skills,
  skill_read: "# vitest-focus\n\nRun the one test file that covers the change first.",
  context_status: contextStatus,
  context_read_md: "# demo-playground\n\n- Run `npm test` before every commit.\n",
  memory_list: memories,
  memory_read: "Fixture tokens expire after 30 days.",
  memory_injection_enabled: true,
  memory_injection_recent: [
    {
      tsMs: NOW - min(39),
      projectRoot: ROOT,
      promptHead: "Fix the failing user-auth test",
      termId: "sess-auth",
      hits: [{ id: "auth-token-expiry", title: "auth-token-expiry", kind: "memory", score: 0.81 }],
    },
  ],
  memory_feedback_stats: [
    {
      docId: "auth-token-expiry",
      injectedCount: 6,
      helpful: 4,
      unhelpful: 0,
      lastInjectedMs: NOW - min(39),
      excluded: false,
      pinned: true,
    },
  ],
  flywheel_metrics: flywheel,
  work_profile_get: "",
  activity_list: [],
  semantic_search: [],
  palette_index_files: [
    "src/api.ts",
    "src/app.ts",
    "src/main.ts",
    "src/retry.ts",
    "tests/auth.test.ts",
    "tests/retry.test.ts",
    "CLAUDE.md",
    "README.md",
    "package.json",
  ],
  // add-ons
  plugins_list_installed: installedPlugins,
  plugins_list_available: availablePlugins,
  mcp_list: mcpServers,
  console_reply: undefined,
  learning_save_memory: "/home/demo/.claude/projects/demo-playground/memory/handoff.md",
  console_mcp_config: (args: Record<string, unknown>) => {
    const caller = typeof args.caller === "string" && args.caller ? args.caller : "agent";
    const bridge = "/home/demo/.local/share/agent-console/bin/hook-bridge";
    const a = ["console-mcp", "--project", ROOT, "--caller", caller];
    return {
      command: bridge,
      args: a,
      claudeAdd: `claude mcp add agent-console -- ${bridge} ${a.join(" ")}`,
      codexToml: `[mcp_servers.agent_console]\ncommand = "${bridge}"\nargs = [${a.map((x) => `"${x}"`).join(", ")}]`,
      tools: [
        "console_state",
        "list_actions",
        "invoke_action",
        "list_sessions",
        "new_session",
        "send_prompt",
        "session_events",
        "memory_search",
        "ledger_verify",
      ],
    };
  },
  // work
  jira_status: { configured: false, siteUrl: "", email: "" },
  jira_daily_digest: [],
  teams_status: { configured: false, clientId: "", tenant: "", account: "" },
  notes_list: notes,
  notes_save: undefined,
  scheduler_list: jobs,
  scheduler_history: runHistory,
  scheduler_is_paused: false,
  // rooms & jobs
  roundtable_list_rooms: rooms,
  roundtable_get_room: null,
  roundtable_connector_state: {
    team: null,
    tasks: [],
    questions: [],
    reviews: [],
    pendingJobs: [],
  },
  jobs_board: jobsBoard,
  // voice, feedback, misc
  voice_status: voice,
  feedback_dev_enabled: false,
  feedback_context: feedbackContext,
};
