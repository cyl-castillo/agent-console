export async function isPermissionGranted(): Promise<boolean> {
  return false;
}
export async function requestPermission(): Promise<"granted" | "denied"> {
  return "denied";
}
export function sendNotification(opts: unknown): void {
  console.warn("[shim] sendNotification", opts);
}
