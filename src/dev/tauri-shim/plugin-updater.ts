export interface Update {
  version: string;
  body?: string;
  downloadAndInstall: (cb?: (e: unknown) => void) => Promise<void>;
}
export async function check(): Promise<Update | null> {
  return null;
}
