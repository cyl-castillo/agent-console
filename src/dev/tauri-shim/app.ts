declare const __APP_VERSION__: string;
export async function getVersion(): Promise<string> {
  return typeof __APP_VERSION__ === "string" ? __APP_VERSION__ : "0.0.0";
}
