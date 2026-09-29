import { createHash } from "node:crypto";
import { readdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import type { Plugin } from "vite-plus";

// Cache a complete, content-versioned player/SDK together. Never cache API
// responses or user media. A new version installs next to the running one and
// activates when a page asks (see src/app-update.ts) or every tab has closed.
// Older versions' caches stay while other tabs are open, so a tab still
// running an older version keeps loading its own hashed Worker, WASM and
// chunks, never a new, incompatible binary.
export function offlinePlayer(): Plugin {
  return {
    name: "offline-player",
    apply: "build",
    async writeBundle(options) {
      const directory = resolve(options.dir!);
      const files = (await readdir(directory, { recursive: true }))
        .filter(
          (file) =>
            /\.(html|js|css|wasm|woff2?|ttf|otf|png|webp|svg|jpg)$/.test(
              file,
            ) && file !== "service-worker.js",
        )
        .sort();
      const hash = createHash("sha256");
      for (const file of files)
        hash.update(file).update(await readFile(resolve(directory, file)));
      const cache = `zigba-player-${hash.digest("hex").slice(0, 20)}`;
      await writeFile(
        resolve(directory, "service-worker.js"),
        `
const CACHE = ${JSON.stringify(cache)};
const FILES = ${JSON.stringify(files)};
const urls = new Set(FILES.map(file => new URL(file, self.registration.scope).href));
// The player's pages, relative to the scope; each one is index.html.
const PAGE = /^((library|manage)(\\/[^/]+)?|player|share\\/state\\/[^/]+)?$/;
// Vite's hashed assets: a name always identifies the same bytes.
const ASSETS = new URL('assets/', self.registration.scope).href;
self.addEventListener('install', event => {
  event.waitUntil(caches.open(CACHE).then(cache => cache.addAll([...urls])));
});
self.addEventListener('activate', event => {
  event.waitUntil(self.clients.claim().then(() => prune()));
});
self.addEventListener('message', event => {
  if (event.data?.type === 'skip-waiting') event.waitUntil(self.skipWaiting());
  // Sent by a page this version served, as it starts.
  else if (event.data?.type === 'prune') event.waitUntil(prune(event.source?.id));
});
// Deletes older versions' caches once no tab other than the given page (which
// runs this version) is open. caches.keys() lists caches in creation order, so a
// newer version installing alongside keeps its cache.
async function prune(page) {
  const tabs = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
  if (tabs.some(tab => tab.id !== page)) return;
  const names = await caches.keys();
  for (const name of names.slice(0, Math.max(names.indexOf(CACHE), 0))) {
    if (name.startsWith('zigba-player-')) await caches.delete(name);
  }
}
self.addEventListener('fetch', event => {
  if (event.request.method !== 'GET') return;
  const url = new URL(event.request.url);
  if (url.origin !== self.location.origin || url.pathname.startsWith('/api/')) return;
  url.search = '';
  const page = url.href.startsWith(self.registration.scope)
    && PAGE.test(url.href.slice(self.registration.scope.length));
  if (page) url.href = new URL('index.html', self.registration.scope).href;
  if (!urls.has(url.href)) {
    // A tab that loaded an older version asks for that version's assets.
    if (url.href.startsWith(ASSETS)) {
      event.respondWith(caches.match(url.href).then(response => response || fetch(event.request)));
    }
    return;
  }
  event.respondWith(caches.open(CACHE).then(async cache => {
    const response = await cache.match(url.href) || await fetch(event.request);
    // Static Assets redirects index.html to /. Navigation requests use manual
    // redirects and reject a cached response that was itself redirected.
    return response.redirected
      ? new Response(response.body, { status: response.status, statusText: response.statusText, headers: response.headers })
      : response;
  }));
});
`,
      );
    },
  };
}
