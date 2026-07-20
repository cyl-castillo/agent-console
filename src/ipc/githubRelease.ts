import { getVersion } from "@tauri-apps/api/app";

const RELEASES_API = "https://api.github.com/repos/cyl-castillo/agent-console/releases/latest";
const RELEASES_PAGE = "https://github.com/cyl-castillo/agent-console/releases/latest";

export type ManualUpdateInfo = {
  version: string;
  currentVersion: string;
  notes?: string;
  url: string;
};

function cmpSemver(a: string, b: string): number {
  const pa = a
    .replace(/^v/, "")
    .split(/[.+-]/)
    .map((x) => parseInt(x, 10));
  const pb = b
    .replace(/^v/, "")
    .split(/[.+-]/)
    .map((x) => parseInt(x, 10));
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const ai = Number.isFinite(pa[i]) ? pa[i] : 0;
    const bi = Number.isFinite(pb[i]) ? pb[i] : 0;
    if (ai !== bi) return ai - bi;
  }
  return 0;
}

export async function checkGithubRelease(): Promise<ManualUpdateInfo | null> {
  const currentVersion = await getVersion();
  const res = await fetch(RELEASES_API, {
    headers: { Accept: "application/vnd.github+json" },
  });
  if (!res.ok) throw new Error(`GitHub API ${res.status}`);
  const data = (await res.json()) as {
    tag_name?: string;
    html_url?: string;
    body?: string;
  };
  const tag = data.tag_name?.replace(/^v/, "");
  if (!tag) return null;
  if (cmpSemver(tag, currentVersion) <= 0) return null;
  return {
    version: tag,
    currentVersion,
    notes: data.body,
    url: data.html_url ?? RELEASES_PAGE,
  };
}
