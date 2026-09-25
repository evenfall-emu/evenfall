import { createHash } from "node:crypto";
import { readdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import type { Plugin } from "vite";

// Cache a complete, content-versioned player/SDK together. Never cache API
// responses or user media. Updates wait for old tabs to close, avoiding a
// running old Worker loading a new, incompatible WASM binary.
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
const PAGE = /^(library(\\/[^/]+)?|player)?$/;
self.addEventListener('install', event => {
  event.waitUntil(caches.open(CACHE).then(cache => cache.addAll([...urls])));
});
self.addEventListener('activate', event => {
  event.waitUntil((async () => {
    for (const name of await caches.keys()) {
      if (name.startsWith('zigba-player-') && name !== CACHE) await caches.delete(name);
    }
    await self.clients.claim();
  })());
});
self.addEventListener('fetch', event => {
  if (event.request.method !== 'GET') return;
  const url = new URL(event.request.url);
  if (url.origin !== self.location.origin || url.pathname.startsWith('/api/')) return;
  url.search = '';
  const page = url.href.startsWith(self.registration.scope)
    && PAGE.test(url.href.slice(self.registration.scope.length));
  if (page) url.href = new URL('index.html', self.registration.scope).href;
  if (!urls.has(url.href)) return;
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
