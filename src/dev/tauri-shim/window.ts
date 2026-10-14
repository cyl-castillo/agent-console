export function getCurrentWindow() {
  return {
    label: "main",
    setTitle: async (title: string) => {
      document.title = title;
    },
    onCloseRequested: async (_cb: (e: { preventDefault: () => void }) => void) => () => {},
    destroy: async () => {},
    close: async () => {},
  };
}
