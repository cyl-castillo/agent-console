// The OpenCode witness plugin ships as plain JS (it runs inside OpenCode,
// under Bun, not in the webview); this is its shape for the replay test.
declare module "*/opencode-plugin.js" {
  export type WitnessHooks = {
    "chat.message"?: (input: unknown, output: unknown) => Promise<void>;
    "tool.execute.after"?: (input: unknown, output: unknown) => Promise<void>;
    event?: (input: { event: unknown }) => Promise<void>;
  };
  export function AgentConsoleWitness(input: { directory: string }): Promise<WitnessHooks>;
}
