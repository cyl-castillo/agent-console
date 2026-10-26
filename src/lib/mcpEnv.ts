/// The env textarea of the MCP add form is `KEY=VALUE` lines. Set one key,
/// replacing an existing line for it (a placeholder someone typed) rather
/// than appending a duplicate.
export function setEnvLine(envText: string, key: string, value: string): string {
  const lines = envText.split("\n");
  const prefix = `${key}=`;
  let done = false;
  const next = lines.map((l) => {
    if (l.trim().startsWith(prefix)) {
      done = true;
      return `${key}=${value}`;
    }
    return l;
  });
  if (!done) {
    if (next.length === 1 && next[0].trim() === "") next[0] = `${key}=${value}`;
    else next.push(`${key}=${value}`);
  }
  return next.join("\n");
}

/// A stdio command that is a GitHub MCP server: the one env it needs is the
/// token the gh CLI already holds.
export function looksLikeGithubMcp(command: string): boolean {
  return /github/i.test(command);
}
