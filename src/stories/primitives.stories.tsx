// Every primitive in every state. These stories are the axe gate for src/ui:
// zero serious/critical violations, and Tab reaches everything.
import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Bot, FolderOpen, GitBranch, Play, Search, Settings } from "lucide-react";
import {
  Badge,
  Button,
  Checkbox,
  Chip,
  Command,
  ConfirmDialog,
  Dialog,
  EmptyState,
  Field,
  IconButton,
  Input,
  Kbd,
  Popover,
  Progress,
  Row,
  SectionHeader,
  Select,
  Sheet,
  showToast,
  Spinner,
  StatusGlyph,
  Switch,
  Tabs,
  Textarea,
  ToastHost,
  Tooltip,
  TooltipProvider,
  type AgentState,
  BudgetRing,
  DecisionCard,
} from "../ui";
import { useThemeStore } from "../stores/themeStore";

const meta: Meta = {
  title: "Primitives",
  parameters: { layout: "padded" },
  decorators: [
    (Story) => (
      <TooltipProvider>
        <div className="min-h-screen bg-background p-6 font-sans text-body text-foreground">
          <Story />
        </div>
      </TooltipProvider>
    ),
  ],
};
export default meta;

const ready = async () => {
  await new Promise((r) => setTimeout(r, 250));
  await document.fonts.ready;
  document.body.setAttribute("data-story-ready", "1");
};
const theme = (mode: "dark" | "light"): StoryObj["loaders"] => [
  async () => {
    useThemeStore.getState().setTheme(mode);
    return {};
  },
];

const H = ({ children }: { children: string }) => (
  <h2 className="mb-2 mt-6 text-small font-semibold uppercase tracking-wide text-faint-foreground first:mt-0">
    {children}
  </h2>
);

function Controls() {
  const [on, setOn] = useState(true);
  const [checked, setChecked] = useState(false);
  const [engine, setEngine] = useState<"claude" | "codex" | null>("claude");
  return (
    <div className="max-w-2xl">
      <H>Buttons</H>
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="primary">Approve once</Button>
        <Button>Always…</Button>
        <Button variant="ghost">Later</Button>
        <Button variant="danger">Deny</Button>
        <Button loading>Running</Button>
        <Button disabled>Disabled</Button>
        <Button size="sm" icon={<Play size={14} />}>
          Run
        </Button>
        <Button size="sm" trailing={<Kbd>⏎</Kbd>}>
          Send
        </Button>
        <Tooltip content="Open settings">
          <IconButton label="Settings" icon={<Settings size={16} />} />
        </Tooltip>
        <IconButton label="Branches" icon={<GitBranch size={16} />} active />
        <IconButton label="Search" icon={<Search size={16} />} size="sm" />
      </div>

      <H>Chips, badges, status</H>
      <div className="flex flex-wrap items-center gap-2">
        <Chip>default</Chip>
        <Chip tone="accent" icon={<Bot size={11} />}>
          claude
        </Chip>
        <Chip tone="ok">passed</Chip>
        <Chip tone="warn">needs you</Chip>
        <Chip tone="danger">failed</Chip>
        <Chip tone="purple" mono>
          a1b2c3d
        </Chip>
        <Badge count={3} />
        <Badge count={120} />
        <Kbd>Ctrl</Kbd>
        <Kbd>K</Kbd>
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-4">
        {(["needs-you", "working", "review", "done", "failed", "stopped"] as AgentState[]).map(
          (s) => (
            <StatusGlyph key={s} state={s} showLabel />
          ),
        )}
        <Spinner label="Loading sessions" />
      </div>

      <H>Progress</H>
      <div className="flex flex-col gap-2">
        <Progress value={62} label="Context used" />
        <Progress value={91} label="Budget" tone="warn" />
        <Progress value={null} label="Downloading Piper" />
      </div>

      <H>Form</H>
      <div className="grid max-w-md grid-cols-1 gap-3">
        <Field label="Session name" description="Shown in the sidebar and spoken by voice.">
          <Input defaultValue="fix auth test" />
        </Field>
        <Field label="Problem" error="Say what the room should decide.">
          <Textarea placeholder="e.g. Should backoff live in retry.ts or in the API client?" />
        </Field>
        <Field label="Engine">
          <Select
            label="Engine"
            value={engine}
            onValueChange={setEngine}
            options={[
              { value: "claude", label: "Claude", description: "claude-opus-5-5" },
              { value: "codex", label: "Codex", description: "gpt-5.3-codex" },
            ]}
          />
        </Field>
        <Switch checked={on} onCheckedChange={setOn} label="Confirm before landing" />
        <Checkbox checked={checked} onCheckedChange={setChecked} label="Attach proof to the PR" />
      </div>
    </div>
  );
}

function Layout() {
  const [tab, setTab] = useState<"turns" | "ledger" | "files">("turns");
  return (
    <div className="grid max-w-3xl grid-cols-2 gap-6">
      <div className="rounded-3 border border-border bg-surface-1">
        <SectionHeader
          title="Sessions"
          count={3}
          actions={<IconButton label="New session" icon={<Play size={14} />} size="sm" />}
        />
        <Row
          leading={<StatusGlyph state="needs-you" />}
          title="fix auth test"
          subtitle="waiting for approval · npm test"
          trailing="4m"
          selected
        />
        <Row
          leading={<StatusGlyph state="working" />}
          title="retry path regression"
          subtitle="Edit src/retry.ts"
          trailing="12s"
        />
        <Row leading={<StatusGlyph state="done" />} title="document retry contract" trailing="2h" />
        <Row
          leading={<StatusGlyph state="failed" />}
          title="flaky auth timeout"
          subtitle="reviewer did not record a verdict"
          trailing="1h"
        />
      </div>
      <div className="rounded-3 border border-border bg-surface-1">
        <Tabs
          label="Session views"
          value={tab}
          onValueChange={setTab}
          items={[
            { value: "turns", label: "Turns", count: 3 },
            { value: "ledger", label: "Ledger" },
            { value: "files", label: "Files", count: 2 },
          ]}
        >
          {{
            turns: <div className="p-3 text-muted-foreground">Three turns, one running.</div>,
            ledger: <div className="p-3 text-muted-foreground">18 events, chain verified.</div>,
            files: <div className="p-3 text-muted-foreground">src/api.ts, tests/auth.test.ts</div>,
          }}
        </Tabs>
      </div>
      <div className="col-span-2 rounded-3 border border-border bg-surface-1">
        <EmptyState
          icon={<FolderOpen />}
          title="Nothing to decide"
          description="2 sessions working, 1 ready for review. You will be nudged when one needs you."
          action={<Button variant="primary">New session</Button>}
          checklist={[
            { label: "Open a project", done: true },
            { label: "Run your first turn", done: true },
            { label: "Export a proof packet", done: false },
          ]}
        />
      </div>
    </div>
  );
}

function Overlays() {
  const [dialog, setDialog] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [sheet, setSheet] = useState(false);
  return (
    <div className="flex flex-wrap gap-2">
      <ToastHost />
      <Button onClick={() => setDialog(true)}>Open dialog</Button>
      <Button variant="danger" onClick={() => setConfirm(true)}>
        Discard worktree…
      </Button>
      <Button onClick={() => setSheet(true)}>Open sheet</Button>
      <Popover trigger={<Button>Session details</Button>} title="fix auth test">
        <div className="flex flex-col gap-1 text-small text-muted-foreground">
          <span>claude-opus-5-5 · 61k of 200k context</span>
          <span>worktree ac/retry-path · 2 ahead</span>
        </div>
      </Popover>
      <Button onClick={() => showToast("Project path copied", "success")}>Toast: success</Button>
      <Button onClick={() => showToast("Couldn't open the browser", "error")}>Toast: error</Button>
      <Dialog
        open={dialog}
        onOpenChange={setDialog}
        title="Export proof packet"
        description="18 events, 2 redactions. Anyone can verify it offline."
        footer={
          <>
            <Button onClick={() => setDialog(false)}>Cancel</Button>
            <Button variant="primary" onClick={() => setDialog(false)}>
              Export
            </Button>
          </>
        }
      >
        <Field label="File name">
          <Input defaultValue="proof-2026-10-13.json" />
        </Field>
      </Dialog>
      <ConfirmDialog
        open={confirm}
        onResolve={() => setConfirm(false)}
        title="Discard this worktree?"
        message={
          "ac/retry-path has 2 commits that are not on main.\nThe branch is kept; only the checkout is removed."
        }
        confirmLabel="Discard"
        cancelLabel="Keep"
        danger
      />
      <Sheet open={sheet} onOpenChange={setSheet} title="Turn inspector">
        <div className="flex flex-col gap-2 text-muted-foreground">
          <span>prompt → Read → Edit → Bash(npm test) → turn_end</span>
          <span>hash 7e3a…c1 · prev 00…00</span>
        </div>
      </Sheet>
    </div>
  );
}

function Palette() {
  const items = [
    {
      id: "nav.terminal",
      label: "Open Terminal",
      hint: "Switch to the Terminal tab",
      group: "Navigate",
      shortcut: ["Ctrl", "1"],
      run: () => {},
    },
    {
      id: "nav.changes",
      label: "Open Changes",
      hint: "Switch to the Changes tab",
      group: "Navigate",
      shortcut: ["Ctrl", "2"],
      run: () => {},
    },
    {
      id: "session.new",
      label: "New session",
      group: "Sessions",
      icon: <Play size={14} />,
      run: () => {},
    },
    {
      id: "ui.theme",
      label: "Toggle theme",
      group: "Appearance",
      keywords: ["dark", "light"],
      run: () => {},
    },
  ];
  return (
    <div className="w-[560px] overflow-hidden rounded-3 border border-border bg-surface-2 shadow-e3">
      <Command items={items} />
    </div>
  );
}

export const ControlsDark: StoryObj = {
  render: () => <Controls />,
  loaders: theme("dark"),
  play: ready,
};
export const ControlsLight: StoryObj = {
  render: () => <Controls />,
  loaders: theme("light"),
  play: ready,
};
export const LayoutDark: StoryObj = {
  render: () => <Layout />,
  loaders: theme("dark"),
  play: ready,
};
export const LayoutLight: StoryObj = {
  render: () => <Layout />,
  loaders: theme("light"),
  play: ready,
};
export const OverlaysDark: StoryObj = {
  render: () => <Overlays />,
  loaders: theme("dark"),
  play: ready,
};
export const DialogOpen: StoryObj = {
  render: () => <Overlays />,
  loaders: theme("dark"),
  play: async ({ canvasElement }) => {
    (canvasElement.querySelector("button") as HTMLButtonElement).click();
    await ready();
  },
};
export const ConfirmOpen: StoryObj = {
  render: () => <Overlays />,
  loaders: theme("dark"),
  play: async ({ canvasElement }) => {
    (canvasElement.querySelectorAll("button")[1] as HTMLButtonElement).click();
    await ready();
  },
};
export const SheetOpen: StoryObj = {
  render: () => <Overlays />,
  loaders: theme("dark"),
  play: async ({ canvasElement }) => {
    (canvasElement.querySelectorAll("button")[2] as HTMLButtonElement).click();
    await ready();
  },
};
export const ToastsShown: StoryObj = {
  render: () => <Overlays />,
  loaders: theme("dark"),
  play: async () => {
    showToast("Project path copied", "success");
    showToast("Cleaned 1 orphaned worktree (branches kept)", "info");
    showToast("Couldn't open https://example.com in your browser", "error");
    await ready();
  },
};
export const CommandPalette: StoryObj = {
  render: () => <Palette />,
  loaders: theme("dark"),
  play: ready,
};

function Decisions() {
  const started = Date.now() - 20_000;
  const noop = () => {};
  return (
    <div className="flex max-w-2xl flex-col gap-3">
      <DecisionCard
        tier="low"
        tierLabel="Read-only"
        tierReason="inspects, builds or tests"
        intent="Bash · npm test -- --run tests/retry.test.ts"
        context="fix auth test · ~/code/demo-playground"
        preview={
          <pre className="m-0 p-2 font-code text-small">npm test -- --run tests/retry.test.ts</pre>
        }
        deadline={started + 90_000}
        startedAt={started}
        queuePosition={{ index: 0, total: 3 }}
        actions={[
          { label: "Deny", variant: "danger", onClick: noop },
          { label: "Always…", variant: "secondary", onClick: noop },
          { label: "Approve once", variant: "primary", onClick: noop },
        ]}
      />
      <DecisionCard
        tier="medium"
        tierLabel="Edits a file"
        tierReason="src/retry.ts"
        intent="Edit · src/retry.ts"
        preview={
          <pre className="m-0 p-2 font-code text-small leading-snug">
            <div className="text-danger">-const MAX_BACKOFF_MS = 8000;</div>
            <div className="text-ok">
              +export interface RetryOptions {"{"} maxBackoffMs?: number {"}"}
            </div>
            <div className="text-ok">+const MAX_BACKOFF_MS = 8000;</div>
          </pre>
        }
        actions={[
          { label: "Deny", variant: "danger", onClick: noop },
          { label: "Approve once", variant: "primary", onClick: noop },
        ]}
      />
      <DecisionCard
        tier="high"
        tierLabel="Irreversible or external"
        tierReason="force-push rewrites remote history"
        intent="Bash · git push --force origin main"
        preview={
          <pre className="m-0 p-2 font-code text-small">
            rm -rf dist && git push --force origin main
          </pre>
        }
        deadline={started + 90_000}
        startedAt={started}
        typedConfirm
        actions={[
          { label: "Deny", variant: "danger", onClick: noop },
          { label: "Approve once", variant: "primary", onClick: noop },
        ]}
      />
      <div className="flex flex-wrap gap-4">
        <BudgetRing spent={1.2} cap={5} />
        <BudgetRing spent={4.3} cap={5} />
        <BudgetRing spent={5.12} cap={5} />
        <BudgetRing spent={0.4} cap={0} />
      </div>
    </div>
  );
}
export const DecisionCards: StoryObj = {
  render: () => <Decisions />,
  loaders: theme("dark"),
  play: ready,
};
export const DecisionCardsLight: StoryObj = {
  render: () => <Decisions />,
  loaders: theme("light"),
  play: ready,
};
