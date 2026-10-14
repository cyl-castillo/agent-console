export async function openUrl(url: string): Promise<void> {
  console.warn("[shim] openUrl", url);
}
export async function openPath(path: string): Promise<void> {
  console.warn("[shim] openPath", path);
}
export async function revealItemInDir(path: string): Promise<void> {
  console.warn("[shim] revealItemInDir", path);
}
