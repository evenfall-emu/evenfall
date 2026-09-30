// Public, read-only patch CDN. Deploy separately from the catalog/account
// Worker so the patches hostname cannot serve player assets or private saves.
type Env = { PATCHES?: R2Bucket };

const origins = new Set([
  "https://play.evenfall-emu.org",
  "http://127.0.0.1:5173",
]);
const patchKey = /^\/([0-9a-f]{40}\.(?:bps|ups|ips))$/;

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const origin = request.headers.get("Origin");
    const headers = new Headers({ Vary: "Origin" });
    if (origin && origins.has(origin))
      headers.set("Access-Control-Allow-Origin", origin);

    if (request.method !== "GET" && request.method !== "HEAD") {
      headers.set("Allow", "GET, HEAD");
      return new Response(null, { status: 405, headers });
    }
    const key = new URL(request.url).pathname.match(patchKey)?.[1];
    if (!key) return new Response(null, { status: 404, headers });
    if (!env.PATCHES) return new Response(null, { status: 503, headers });

    if (request.method === "HEAD") {
      const object = await env.PATCHES.head(key);
      if (!object) return new Response(null, { status: 404, headers });
      headers.set("Content-Type", "application/octet-stream");
      headers.set("Content-Length", String(object.size));
      headers.set("Cache-Control", "public, max-age=31536000, immutable");
      return new Response(null, { headers });
    }
    const object = await env.PATCHES.get(key);
    if (!object) return new Response(null, { status: 404, headers });
    headers.set("Content-Type", "application/octet-stream");
    headers.set("Content-Length", String(object.size));
    headers.set("Cache-Control", "public, max-age=31536000, immutable");
    return new Response(object.body, { headers });
  },
};
