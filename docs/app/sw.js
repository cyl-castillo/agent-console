// Service worker: shows the desktop's push ("N decisions wait") and opens
// the app on tap. Nothing is decided here — a notification is a knock.
// A minimal app-shell cache keeps the page installable and opening offline
// to its "cannot reach the desktop" state.

const SHELL = "ac-companion-shell-v1";
// The app may be served from a sub-path (agent-console.dev/app/): everything
// is relative to where this worker was registered.
const BASE = new URL(self.registration.scope).pathname;

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches.open(SHELL).then((c) => c.addAll([BASE, `${BASE}manifest.webmanifest`, `${BASE}icon.svg`]).catch(() => undefined)),
  );
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== SHELL).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET" || new URL(req.url).origin !== self.location.origin) return;
  event.respondWith(
    fetch(req)
      .then((res) => {
        const copy = res.clone();
        caches.open(SHELL).then((c) => c.put(req, copy)).catch(() => undefined);
        return res;
      })
      .catch(() => caches.match(req).then((hit) => hit || caches.match(BASE))),
  );
});

self.addEventListener("push", (event) => {
  let pending = 0;
  try {
    const data = event.data ? event.data.json() : null;
    if (data && data.type === "decisions") pending = Number(data.pending) || 0;
  } catch {
    // Not ours or not JSON: still a knock.
  }
  const title = pending === 1 ? "1 decision waits" : `${pending || "Some"} decisions wait`;
  event.waitUntil(
    self.registration.showNotification(title, {
      body: "Open to see the evidence and answer.",
      tag: "agent-console-decisions",
      renotify: true,
      icon: `${BASE}icon.svg`,
      badge: `${BASE}icon.svg`,
      data: { url: BASE },
    }),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((list) => {
      for (const c of list) {
        if ("focus" in c) return c.focus();
      }
      return self.clients.openWindow(BASE);
    }),
  );
});
