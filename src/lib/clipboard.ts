/// Copy text: the native plugin first (WebKitGTK reports success on
/// `navigator.clipboard` without writing), the browser API in Storybook.
export async function copyText(text: string): Promise<void> {
  try {
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
    await writeText(text);
    return;
  } catch {
    // Not inside Tauri (stories, tests): fall through.
  }
  if (!navigator.clipboard) throw new Error("clipboard unavailable");
  await navigator.clipboard.writeText(text);
}
