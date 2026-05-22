import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdateInfo = {
  version: string;
  currentVersion: string;
  notes?: string;
  update: Update;
};

export async function checkForUpdate(): Promise<UpdateInfo | null> {
  const update = await check();
  if (!update) return null;
  return {
    version: update.version,
    currentVersion: update.currentVersion,
    notes: update.body,
    update,
  };
}

export async function installAndRelaunch(info: UpdateInfo): Promise<void> {
  await info.update.downloadAndInstall();
  await relaunch();
}
