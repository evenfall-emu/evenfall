import { readFile, readdir } from "node:fs/promises";
import { createHash } from "node:crypto";
import { gzipSync } from "node:zlib";
import { build } from "esbuild";
import { convertV4MiniflareOptions, Miniflare } from "miniflare";
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import app from "../src/index";
import { sweepDeletedAccounts } from "../src/cloud-saves";

const origin = "https://play.evenfall-emu.org";
let miniflare: Miniflare;
let database: D1Database;
let saves: Awaited<ReturnType<Miniflare["getR2Bucket"]>>;
const messages: EmailMessageBuilder[] = [];

function sweepBindings(bucket: typeof saves = saves) {
  return { DB: database, SAVES: bucket as unknown as R2Bucket };
}

function request(
  path: string,
  init: RequestInit = {},
  overrides: Partial<Parameters<typeof app.request>[2]> = {},
) {
  return app.request(new URL(path, origin), init, {
    DB: database,
    BETTER_AUTH_URL: origin,
    BETTER_AUTH_SECRET: "test-only-auth-secret-not-for-production-123456789",
    DISCORD_CLIENT_ID: "test-discord-client",
    DISCORD_CLIENT_SECRET: "test-discord-secret",
    EMAIL_FROM: "noreply@evenfall-emu.org",
    EMAIL: {
      async send(message: EmailMessage | EmailMessageBuilder) {
        messages.push(message as EmailMessageBuilder);
        return { messageId: "test-message" };
      },
    },
    ...overrides,
  });
}

function post(body: unknown, ip = "192.0.2.1"): RequestInit {
  return {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Origin: origin,
      "CF-Connecting-IP": ip,
    },
    body: JSON.stringify(body),
  };
}

beforeAll(async () => {
  const bundled = await build({
    entryPoints: [new URL("../src/index.ts", import.meta.url).pathname],
    bundle: true,
    write: false,
    format: "esm",
    platform: "neutral",
    conditions: ["workerd", "worker", "browser"],
    external: ["node:*"],
  });
  miniflare = new Miniflare(
    convertV4MiniflareOptions({
      modules: true,
      script: bundled.outputFiles[0].text,
      compatibilityDate: "2026-09-20",
      compatibilityFlags: ["nodejs_compat"],
      bindings: {
        BETTER_AUTH_URL: origin,
        BETTER_AUTH_SECRET:
          "test-only-auth-secret-not-for-production-123456789",
        DISCORD_CLIENT_ID: "test-discord-client",
        DISCORD_CLIENT_SECRET: "test-discord-secret",
      },
      d1Databases: ["DB"],
      r2Buckets: ["SAVES"],
    }),
  );
  database = await miniflare.getD1Database("DB");
  saves = await miniflare.getR2Bucket("SAVES");
  const directory = new URL("../migrations/", import.meta.url);
  for (const file of (await readdir(directory))
    .filter((name) => name.endsWith(".sql"))
    .sort()) {
    const migration = await readFile(new URL(file, directory), "utf8");
    for (const statement of migration.split("--> statement-breakpoint")) {
      if (statement.trim()) await database.prepare(statement).run();
    }
  }
});

afterAll(async () => {
  await miniflare.dispose();
});

beforeEach(() => {
  messages.length = 0;
});

afterEach(() => vi.restoreAllMocks());

function messageLink() {
  return messages.at(-1)!.text!.match(/https:\/\/\S+/)![0];
}

function sessionCookie(response: Response) {
  return response.headers
    .getSetCookie()
    .find((value) => value.startsWith("__Secure-better-auth.session_token="))!
    .split(";")[0];
}

async function discordCallback(
  profile: { id: string; email: string | null; verified: boolean },
  tamperState = false,
) {
  const started = await request(
    "/api/auth/sign-in/social",
    post(
      {
        provider: "discord",
        callbackURL: "/?account=1",
        errorCallbackURL: "/?account=1",
      },
      `192.0.2.${60 + Number(profile.id[0])}`,
    ),
  );
  expect(started.status).toBe(200);
  const { url } = (await started.json()) as { url: string };
  const authorization = new URL(url);
  expect(authorization.origin).toBe("https://discord.com");
  expect(authorization.searchParams.get("redirect_uri")).toBe(
    `${origin}/api/auth/callback/discord`,
  );
  expect(authorization.searchParams.get("scope")!.split(" ").sort()).toEqual([
    "email",
    "identify",
  ]);
  const originalFetch = globalThis.fetch;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    const url = new URL(input instanceof Request ? input.url : String(input));
    if (url.hostname !== "discord.com") return originalFetch(input, init);
    if (url.pathname === "/api/oauth2/token")
      return Response.json({
        access_token: "test-access-token",
        refresh_token: "test-refresh-token",
        token_type: "Bearer",
        expires_in: 3600,
        scope: "identify email",
      });
    if (decodeURIComponent(url.pathname) === "/api/users/@me")
      return Response.json({
        ...profile,
        username: "test-player",
        avatar: null,
      });
    throw new Error(`Unexpected Discord API path ${url.pathname}`);
  });
  const query = new URLSearchParams({
    code: "test-code",
    state: tamperState
      ? "wrong-state"
      : authorization.searchParams.get("state")!,
  });
  return request(`/api/auth/callback/discord?${query}`, {
    headers: {
      Cookie: started.headers
        .getSetCookie()
        .map((cookie) => cookie.split(";")[0])
        .join("; "),
    },
  });
}

describe("account authentication", () => {
  it("keeps native Workers auth responses private, including redirects", async () => {
    const session = await miniflare.dispatchFetch(
      `${origin}/api/auth/get-session`,
    );
    expect(session.status).toBe(200);
    expect(await session.json()).toBeNull();
    const callback = await miniflare.dispatchFetch(
      `${origin}/api/auth/magic-link/verify?token=invalid&callbackURL=/`,
      { redirect: "manual" },
    );
    expect(callback.status).toBe(302);
    for (const response of [session, callback]) {
      expect(response.headers.get("cache-control")).toBe("no-store");
      expect(response.headers.get("referrer-policy")).toBe("no-referrer");
    }
  });

  it("fails closed when authentication secrets are not configured", async () => {
    const response = await app.request(
      `${origin}/api/auth/get-session`,
      {},
      { DB: database },
    );
    expect(response.status).toBe(503);
    expect(await response.json()).toEqual({
      error: { code: "auth_unavailable", message: "Sign-in is not configured" },
    });
    expect(response.headers.get("cache-control")).toBe("no-store");
  });

  it("accepts only an exact HTTPS origin or the fixed local origin", async () => {
    for (const configured of [
      undefined,
      "http://zigba.example",
      "http://localhost:5173",
      "https://zigba.example/",
      "https://zigba.example/app",
      "not a url",
    ]) {
      const response = await request(
        "/api/auth/get-session",
        {},
        { BETTER_AUTH_URL: configured },
      );
      expect(response.status, String(configured)).toBe(503);
    }
    for (const configured of [
      "https://zigba.example",
      "http://127.0.0.1:5173",
    ]) {
      const response = await request(
        "/api/auth/get-session",
        {},
        { BETTER_AUTH_URL: configured },
      );
      expect(response.status, configured).toBe(200);
    }
  });

  it("disables email sign-in without a configured sender", async () => {
    const response = await request(
      "/api/auth/sign-in/magic-link",
      post({ email: "player@example.com" }, "192.0.2.201"),
      { EMAIL_FROM: undefined },
    );
    expect(response.status).toBe(503);
    expect(await response.json()).toEqual({
      error: {
        code: "email_unavailable",
        message: "Email sign-in is not configured",
      },
    });
    expect(messages).toHaveLength(0);
  });

  it("emails a five-minute sign-in link and creates a private session", async () => {
    const response = await request(
      "/api/auth/sign-in/magic-link",
      post({
        email: "player@example.com",
        callbackURL: "/?account=1",
      }),
    );
    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ status: true });
    expect(messages).toHaveLength(1);
    expect(messages[0]).toMatchObject({
      from: { email: "noreply@evenfall-emu.org", name: "ZiGBA" },
      to: "player@example.com",
      subject: "Sign in to ZiGBA",
    });
    expect(messages[0].text).toContain("5 minutes");
    const link = messages[0].text!.match(/https:\/\/\S+/)![0];
    expect(new URL(link).origin).toBe(origin);
    const verified = await request(link);
    expect(verified.status).toBe(302);
    expect(verified.headers.get("location")).toBe(`${origin}/?account=1`);
    const cookie = verified.headers
      .getSetCookie()
      .find((value) =>
        value.startsWith("__Secure-better-auth.session_token="),
      )!;
    expect(cookie).toContain("HttpOnly");
    expect(cookie).toContain("Secure");
    expect(cookie).toContain("SameSite=Lax");
    expect(cookie).not.toContain("Domain=");
    const session = await request("/api/auth/get-session", {
      headers: { Cookie: cookie.split(";")[0] },
    });
    expect(session.status).toBe(200);
    expect(session.headers.get("cache-control")).toBe("no-store");
    expect(await session.json()).toMatchObject({
      user: { email: "player@example.com", emailVerified: true },
    });
  });

  it("consumes a magic link once even when two verification requests race", async () => {
    await request(
      "/api/auth/sign-in/magic-link",
      post({ email: "once@example.com", callbackURL: "/" }, "192.0.2.2"),
    );
    const link = messageLink();
    const responses = await Promise.all([request(link), request(link)]);
    expect(
      responses.filter((response) =>
        response.headers
          .getSetCookie()
          .some((cookie) =>
            cookie.startsWith("__Secure-better-auth.session_token="),
          ),
      ),
    ).toHaveLength(1);
    expect(
      responses.filter((response) =>
        response.headers.get("location")?.includes("error=INVALID_TOKEN"),
      ),
    ).toHaveLength(1);
  });

  it("rejects expired links and stops accepting a session after sign-out", async () => {
    await request(
      "/api/auth/sign-in/magic-link",
      post({ email: "expiry@example.com", callbackURL: "/" }, "192.0.2.3"),
    );
    const expiredLink = messageLink();
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      vi.setSystemTime(Date.now() + 301_000);
      const expired = await request(expiredLink);
      expect(expired.headers.get("location")).toContain("error=INVALID_TOKEN");
      expect(expired.headers.getSetCookie().join(";")).not.toContain(
        "session_token=",
      );
    } finally {
      vi.useRealTimers();
    }
    await request(
      "/api/auth/sign-in/magic-link",
      post({ email: "signout@example.com", callbackURL: "/" }, "192.0.2.4"),
    );
    const verified = await request(messageLink());
    const cookie = sessionCookie(verified);
    const admin = await request(
      "/api/admin/seed",
      {
        ...post({}),
        headers: { ...post({}).headers, Cookie: cookie },
      },
      { ADMIN_SECRET: "test-admin-secret" },
    );
    expect(admin.status).toBe(401);
    const signedOut = await request("/api/auth/sign-out", {
      ...post({}),
      headers: { ...post({}).headers, Cookie: cookie },
    });
    expect(signedOut.status).toBe(200);
    expect(
      await (
        await request("/api/auth/get-session", { headers: { Cookie: cookie } })
      ).json(),
    ).toBeNull();
  });

  it("rejects cross-origin writes and external callback URLs before sending", async () => {
    const crossOrigin = await request("/api/auth/sign-in/magic-link", {
      ...post({ email: "origin@example.com" }),
      headers: { ...post({}).headers, Origin: "https://attacker.example" },
    });
    expect(crossOrigin.status).toBe(403);
    for (const callbackURL of [
      "https://attacker.example",
      "//attacker.example",
      "https://play.evenfall-emu.org.attacker.example",
    ]) {
      const response = await request(
        "/api/auth/sign-in/magic-link",
        post({ email: "redirect@example.com", callbackURL }, "192.0.2.5"),
      );
      expect(response.status).toBe(403);
    }
    expect(messages).toHaveLength(0);
  });

  it("limits a normalized recipient across IPs, including concurrent requests", async () => {
    const responses = await Promise.all(
      Array.from({ length: 6 }, (_, index) =>
        request(
          "/api/auth/sign-in/magic-link",
          post(
            {
              email: index % 2 ? "limited@example.com" : "LIMITED@example.com",
              callbackURL: "/",
            },
            `192.0.2.${20 + index}`,
          ),
        ),
      ),
    );
    for (const response of responses) {
      expect(response.status).toBe(200);
      expect(await response.json()).toEqual({ status: true });
    }
    expect(messages).toHaveLength(3);
    expect(
      messages.every((message) => message.to === "limited@example.com"),
    ).toBe(true);
  });

  it("limits concurrent requests from one IP across different recipients", async () => {
    const responses = await Promise.all(
      Array.from({ length: 6 }, (_, index) =>
        request(
          "/api/auth/sign-in/magic-link",
          post({ email: `ip-limit-${index}@example.com` }, "192.0.2.80"),
        ),
      ),
    );
    expect(responses.map((response) => response.status).sort()).toEqual([
      200, 200, 200, 429, 429, 429,
    ]);
    expect(messages).toHaveLength(3);
  });

  it("resets recipient windows at the boundary without bypassing the daily limit", async () => {
    const start = Date.now();
    let ip = 100;
    async function attempt(offset: number, delivered: number) {
      vi.setSystemTime(start + offset);
      const response = await request(
        "/api/auth/sign-in/magic-link",
        post({ email: "windows@example.com" }, `192.0.2.${ip++}`),
      );
      expect(await response.json()).toEqual({ status: true });
      expect(messages).toHaveLength(delivered);
    }
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      await attempt(0, 1);
      await attempt(0, 2);
      await attempt(0, 3);
      await attempt(899_999, 3);
      await attempt(900_000, 4);
      await attempt(900_000, 5);
      await attempt(900_000, 6);
      await attempt(1_800_000, 7);
      await attempt(1_800_000, 8);
      await attempt(1_800_000, 9);
      // Suppressed requests count against the ten-per-day request budget too.
      await attempt(86_399_999, 9);
      await attempt(86_400_000, 10);
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not expose email-provider failures or log recipients and tokens", async () => {
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    const response = await request(
      "/api/auth/sign-in/magic-link",
      post({ email: "failed-delivery@example.com" }, "192.0.2.130"),
      {
        EMAIL: {
          async send() {
            throw new Error("Sensitive provider error");
          },
        },
      },
    );
    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ status: true });
    expect(logged.mock.calls).toEqual([["Magic-link delivery failed"]]);
  });

  it("links a verified Discord email to the same magic-link account", async () => {
    await request(
      "/api/auth/sign-in/magic-link",
      post({ email: "linked@example.com", callbackURL: "/" }, "192.0.2.50"),
    );
    const magic = await request(messageLink());
    const magicSession = (await (
      await request("/api/auth/get-session", {
        headers: { Cookie: sessionCookie(magic) },
      })
    ).json()) as { user: { id: string } };
    const discord = await discordCallback({
      id: "123456789",
      email: "linked@example.com",
      verified: true,
    });
    expect(new URL(discord.headers.get("location")!, origin).href).toBe(
      `${origin}/?account=1`,
    );
    const discordSession = await (
      await request("/api/auth/get-session", {
        headers: { Cookie: sessionCookie(discord) },
      })
    ).json();
    expect(discordSession).toMatchObject({
      user: {
        id: magicSession.user.id,
        email: "linked@example.com",
        emailVerified: true,
      },
    });
  });

  it.each([
    { id: "223456789", email: null, verified: true },
    { id: "323456789", email: "unverified@example.com", verified: false },
  ])(
    "rejects Discord without a verified email: $email / $verified",
    async (profile) => {
      const response = await discordCallback(profile);
      expect(response.headers.get("location")).toContain("error=");
      expect(response.headers.getSetCookie().join(";")).not.toContain(
        "session_token=",
      );
    },
  );

  it("rejects an OAuth callback with the wrong state", async () => {
    const response = await discordCallback(
      { id: "423456789", email: "state@example.com", verified: true },
      true,
    );
    expect(response.headers.get("location")).toContain("error=");
    expect(response.headers.getSetCookie().join(";")).not.toContain(
      "session_token=",
    );
  });
});

describe("private cloud resume discovery", () => {
  const knownHash =
    "7c4f90c8b68b64d4639a37306d5302a11cd4d7c44005114ec2faa2b28b210c3d";
  const otherHash = "f".repeat(64);
  let owner: { Cookie: string; "X-Zigba-Account": string };
  let other: typeof owner;

  beforeAll(async () => {
    async function login(email: string, ip: string) {
      const started = await request(
        "/api/auth/sign-in/magic-link",
        post({ email }, ip),
      );
      expect(started.status).toBe(200);
      expect(messages.at(-1)?.to).toBe(email);
      const Cookie = sessionCookie(await request(messageLink()));
      const session = (await (
        await request("/api/auth/get-session", { headers: { Cookie } })
      ).json()) as { user: { id: string } };
      return { Cookie, "X-Zigba-Account": session.user.id };
    }
    owner = await login("cloud-owner@example.com", "192.0.2.170");
    other = await login("cloud-other@example.com", "192.0.2.171");
    const seeded = await request(
      "/api/admin/seed",
      {
        method: "POST",
        headers: { Authorization: "Bearer test-seed-secret", Origin: origin },
      },
      { ADMIN_SECRET: "test-seed-secret" },
    );
    expect(seeded.status).toBe(200);
    const metadata = { customMetadata: { capturedAt: "12345" } };
    await Promise.all(
      Array.from({ length: 101 }, (_, index) =>
        saves.put(
          `${owner["X-Zigba-Account"]}/${index.toString(16).padStart(64, "0")}/resume`,
          "private checkpoint bytes",
          metadata,
        ),
      ),
    );
    await saves.put(
      `${owner["X-Zigba-Account"]}/${knownHash}/resume`,
      "known checkpoint",
      metadata,
    );
    await saves.put(
      `${owner["X-Zigba-Account"]}/${otherHash}/battery`,
      "private battery",
      metadata,
    );
    await saves.put(
      `${other["X-Zigba-Account"]}/${otherHash}/resume`,
      "another user's checkpoint",
      metadata,
    );
  });

  it("requires authentication and matching account even on the exact listing route", async () => {
    for (const [headers, status] of [
      [{}, 401],
      [{ Cookie: owner.Cookie }, 403],
      [{ ...owner, "X-Zigba-Account": other["X-Zigba-Account"] }, 403],
    ] as const) {
      const response = await miniflare.dispatchFetch(`${origin}/api/saves`, {
        headers,
      });
      expect(response.status).toBe(status);
      expect(response.headers.get("cache-control")).toBe("no-store");
      expect(JSON.stringify(await response.json())).not.toContain(knownHash);
    }
  });

  it("paginates only the authenticated account's resume metadata under workerd", async () => {
    type Entry = {
      hash: string;
      title: string;
      capturedAt: number;
      uploadedAt: string;
    };
    const entries: Entry[] = [];
    let cursor: string | null = null;
    let pages = 0;
    do {
      const response = await miniflare.dispatchFetch(
        `${origin}/api/saves?accountId=${other["X-Zigba-Account"]}${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ""}`,
        { headers: owner },
      );
      expect(response.status).toBe(200);
      expect(response.headers.get("cache-control")).toBe("no-store");
      const page = (await response.json()) as {
        saves: Entry[];
        cursor: string | null;
      };
      expect(page.saves.length).toBeLessThanOrEqual(100);
      entries.push(...page.saves);
      cursor = page.cursor;
      expect(++pages).toBeLessThan(5);
    } while (cursor);
    expect(pages).toBeGreaterThan(1);
    expect(entries).toHaveLength(102);
    expect(new Set(entries.map((entry) => entry.hash)).size).toBe(102);
    expect(entries.some((entry) => entry.hash === otherHash)).toBe(false);
    expect(entries.find((entry) => entry.hash === knownHash)).toMatchObject({
      title: "Pokémon Heart & Soul · 1.2.1",
      capturedAt: 12345,
    });
    expect(
      entries.every((entry) => Number.isFinite(Date.parse(entry.uploadedAt))),
    ).toBe(true);
    expect(
      entries.every(
        (entry) =>
          Object.keys(entry).sort().join() ===
          "capturedAt,hash,playthroughId,playthroughName,title,uploadedAt",
      ),
    ).toBe(true);
    const response = await miniflare.dispatchFetch(`${origin}/api/saves`, {
      headers: other,
    });
    expect(await response.json()).toMatchObject({
      saves: [{ hash: otherHash, title: "Game ffffffff" }],
      cursor: null,
    });
    const denied = await miniflare.dispatchFetch(
      `${origin}/api/saves/${knownHash}/default/resume`,
      { headers: other },
    );
    expect(denied.status).toBe(404);
  });

  it("retains private titles for unknown ROMs, validates them, and accepts older upload clients", async () => {
    const hash = "c".repeat(64);
    const body = gzipSync(Buffer.from("ZIGBAST1test"));
    const digest = createHash("sha256").update(body).digest("hex");
    const title = "My Pokémon adventure 🌙";
    const put = (headers: Record<string, string>) =>
      miniflare.dispatchFetch(`${origin}/api/saves/${hash}/default/resume`, {
        method: "PUT",
        body,
        headers: {
          ...other,
          Origin: origin,
          "Content-Type": "application/octet-stream",
          "X-Zigba-Revision": crypto.randomUUID(),
          "X-Zigba-Captured-At": "12345",
          "X-Zigba-Digest": digest,
          ...headers,
        },
      });
    const created = await put({
      "If-None-Match": "*",
      "X-Zigba-Title": encodeURIComponent(title),
    });
    expect(created.status).toBe(200);
    const { etag } = (await created.json()) as { etag: string };
    for (const invalid of [
      "%not-escaped",
      encodeURIComponent("x".repeat(161)),
      encodeURIComponent("line\nbreak"),
    ]) {
      const rejected = await put({
        "If-Match": etag,
        "X-Zigba-Title": invalid,
      });
      expect(rejected.status).toBe(400);
      expect(await rejected.json()).toMatchObject({
        error: { code: "invalid_title" },
      });
    }
    expect((await put({ "If-Match": etag })).status).toBe(200);
    const response = await miniflare.dispatchFetch(`${origin}/api/saves`, {
      headers: other,
    });
    const listing = (await response.json()) as {
      saves: { hash: string; title: string }[];
    };
    expect(listing.saves.find((save) => save.hash === hash)?.title).toBe(title);
    const forbidden = await miniflare.dispatchFetch(
      `${origin}/api/saves/${hash}/default/resume`,
      { headers: owner },
    );
    expect(forbidden.status).toBe(404);
  });
});

describe("playthrough quota and account erasure", () => {
  const hash = "a".repeat(64);
  const body = gzipSync(Buffer.from("ZIGBAST1checkpoint"));
  const digest = createHash("sha256").update(body).digest("hex");
  let owner: { Cookie: string; "X-Zigba-Account": string };
  let other: typeof owner;

  beforeAll(async () => {
    async function login(email: string, ip: string) {
      const started = await request(
        "/api/auth/sign-in/magic-link",
        post({ email }, ip),
      );
      expect(started.status).toBe(200);
      expect(messages.at(-1)?.to).toBe(email);
      const verified = await request(messageLink(), {
        headers: { "CF-Connecting-IP": ip },
      });
      expect(verified.headers.get("location")).toBe(`${origin}/`);
      const Cookie = sessionCookie(verified);
      const session = (await (
        await request("/api/auth/get-session", {
          headers: { Cookie },
        })
      ).json()) as { user: { id: string } };
      return { Cookie, "X-Zigba-Account": session.user.id };
    }
    owner = await login("runs-owner@example.com", "192.0.2.180");
    other = await login("runs-other@example.com", "192.0.2.181");
  });

  function upload(
    account: typeof owner,
    id: string,
    kind: "resume" | "battery" = "resume",
    name = "First run",
    revision = crypto.randomUUID(),
    etag?: string,
  ) {
    const bytes = kind === "resume" ? body : gzipSync(Buffer.alloc(512));
    return miniflare.dispatchFetch(
      `${origin}/api/saves/${hash}/${id}/${kind}`,
      {
        method: "PUT",
        headers: {
          ...account,
          Origin: origin,
          "Content-Type": "application/octet-stream",
          "X-Zigba-Revision": revision,
          "X-Zigba-Captured-At": "42",
          "X-Zigba-Digest":
            kind === "resume"
              ? digest
              : createHash("sha256").update(bytes).digest("hex"),
          "X-Zigba-Playthrough-Name": encodeURIComponent(name),
          ...(etag ? { "If-Match": etag } : { "If-None-Match": "*" }),
        },
        body: bytes,
      },
    );
  }

  it("reserves one slot per run, not per kind or upload; rejects the 101st under concurrency", async () => {
    const first = crypto.randomUUID();
    const created = await upload(owner, first);
    expect(created.status).toBe(200);
    const { etag } = (await created.json()) as { etag: string };
    expect(
      (
        await upload(
          owner,
          first,
          "resume",
          "Renamed run",
          crypto.randomUUID(),
          etag,
        )
      ).status,
    ).toBe(200);
    expect((await upload(owner, first, "battery")).status).toBe(200);
    const ids = Array.from({ length: 100 }, () => crypto.randomUUID());
    // Read each body with its response: 100 idle Miniflare response streams
    // can otherwise be reported as already consumed under GC pressure.
    const results = await Promise.all(
      ids.map(async (id) => {
        const response = await upload(owner, id);
        return { status: response.status, body: await response.json() };
      }),
    );
    expect(results.filter((response) => response.status === 200)).toHaveLength(
      99,
    );
    const rejected = results.find((response) => response.status !== 200)!;
    expect(rejected.status).toBe(409);
    expect(rejected.body).toMatchObject({
      error: { code: "playthrough_limit" },
    });
    const anotherHash = "b".repeat(64);
    const otherGame = await miniflare.dispatchFetch(
      `${origin}/api/saves/${anotherHash}/${crypto.randomUUID()}/resume`,
      {
        method: "PUT",
        body,
        headers: {
          ...owner,
          Origin: origin,
          "Content-Type": "application/octet-stream",
          "If-None-Match": "*",
          "X-Zigba-Revision": crypto.randomUUID(),
          "X-Zigba-Captured-At": "42",
          "X-Zigba-Digest": digest,
        },
      },
    );
    expect((await otherGame.json()) as object).toMatchObject({
      error: { code: "playthrough_limit" },
    });
    expect(otherGame.status).toBe(409);
    const current = await miniflare.dispatchFetch(
      `${origin}/api/saves/${hash}/${first}/resume`,
      { method: "HEAD", headers: owner },
    );
    expect(
      (
        await upload(
          owner,
          first,
          "resume",
          "At cap",
          crypto.randomUUID(),
          current.headers.get("etag")!,
        )
      ).status,
    ).toBe(200);
    for (const name of [
      "%zz",
      encodeURIComponent("a".repeat(161)),
      encodeURIComponent("line\nbreak"),
    ]) {
      const invalid = await miniflare.dispatchFetch(
        `${origin}/api/saves/${hash}/${first}/resume`,
        {
          method: "PUT",
          body,
          headers: {
            ...owner,
            Origin: origin,
            "Content-Type": "application/octet-stream",
            "If-None-Match": "*",
            "X-Zigba-Revision": crypto.randomUUID(),
            "X-Zigba-Captured-At": "42",
            "X-Zigba-Digest": digest,
            "X-Zigba-Playthrough-Name": name,
          },
        },
      );
      expect(invalid.status).toBe(400);
      expect(await invalid.json()).toMatchObject({
        error: { code: "invalid_playthrough_name" },
      });
    }
    expect((await upload(other, ids[0])).status).toBe(200);
    const listing = await miniflare.dispatchFetch(`${origin}/api/saves`, {
      headers: owner,
    });
    expect(listing.status).toBe(200);
    const page = (await listing.json()) as {
      saves: { playthroughId: string; playthroughName: string }[];
      usage: { playthroughs: number; limit: number };
    };
    expect(page.usage).toEqual({ playthroughs: 100, limit: 100 });
    expect(
      page.saves.find((save) => save.playthroughId === first)?.playthroughName,
    ).toBe("At cap");
    const forbidden = await miniflare.dispatchFetch(
      `${origin}/api/saves/${hash}/${first}/resume`,
      { headers: other },
    );
    expect(forbidden.status).toBe(404);
  });

  it("requires cookie, matching account, and exact Origin to delete; purges all pages and revokes auth", async () => {
    const id = owner["X-Zigba-Account"];
    await saves.put(`${id}/${hash}/resume`, "legacy backup");
    await saves.put(`${id}/miscellaneous/old-format`, "unknown old object");
    await request(
      "/api/auth/sign-in/magic-link",
      post({ email: "runs-owner@example.com" }, "192.0.2.190"),
    );
    const pendingLink = messageLink();
    let pages = 0;
    const pagedBucket = new Proxy(saves, {
      get(target, property) {
        if (property === "list")
          return (options: R2ListOptions) => {
            pages++;
            return target.list({ ...options, limit: 2 });
          };
        const value = Reflect.get(target, property);
        return typeof value === "function" ? value.bind(target) : value;
      },
    });
    const denied = [
      await request(
        "/api/account",
        {
          method: "DELETE",
          headers: { ...owner, Origin: "https://evil.example" },
        },
        { SAVES: saves },
      ),
      await request(
        "/api/account",
        { method: "DELETE", headers: { Cookie: owner.Cookie, Origin: origin } },
        { SAVES: saves },
      ),
      await request(
        "/api/account",
        {
          method: "DELETE",
          headers: { ...other, "X-Zigba-Account": id, Origin: origin },
        },
        { SAVES: saves },
      ),
    ];
    expect(denied.map((response) => response.status)).toEqual([403, 403, 403]);
    const erased = await request(
      "/api/account",
      { method: "DELETE", headers: { ...owner, Origin: origin } },
      { SAVES: pagedBucket },
    );
    expect(erased.status).toBe(202);
    expect(await erased.json()).toEqual({
      accountDeleted: true,
      cloudDeletion: "pending",
    });
    expect(erased.headers.get("cache-control")).toBe("no-store");
    expect(pages).toBeGreaterThan(2);
    expect((await saves.list({ prefix: `${id}/` })).objects).toHaveLength(0);
    for (const table of ["user", "session", "account", "playthrough"]) {
      const column = table === "user" ? "id" : "user_id";
      expect(
        (
          await database
            .prepare(
              `SELECT count(*) AS count FROM ${table} WHERE ${column} = ?`,
            )
            .bind(id)
            .first<{ count: number }>()
        )?.count,
      ).toBe(0);
    }
    expect(
      (
        await database
          .prepare("SELECT user_id FROM account_deletions WHERE user_id = ?")
          .bind(id)
          .first()
      )?.user_id,
    ).toBe(id);
    expect(
      await (
        await request("/api/auth/get-session", {
          headers: { Cookie: owner.Cookie },
        })
      ).json(),
    ).toBeNull();
    expect((await upload(owner, crypto.randomUUID())).status).toBe(401);
    const oldLink = await request(pendingLink, {
      headers: { "CF-Connecting-IP": "192.0.2.190" },
    });
    expect(oldLink.headers.get("location")).toContain("error=INVALID_TOKEN");
    await request(
      "/api/auth/sign-in/magic-link",
      post({ email: "runs-owner@example.com" }, "192.0.2.191"),
    );
    const newCookie = sessionCookie(
      await request(messageLink(), {
        headers: { "CF-Connecting-IP": "192.0.2.191" },
      }),
    );
    const newSession = (await (
      await request("/api/auth/get-session", { headers: { Cookie: newCookie } })
    ).json()) as { user: { id: string } };
    expect(newSession.user.id).not.toBe(id);
    expect(
      (
        await upload(
          { Cookie: newCookie, "X-Zigba-Account": newSession.user.id },
          crypto.randomUUID(),
        )
      ).status,
    ).toBe(200);
    expect(
      (
        await miniflare.dispatchFetch(
          `${origin}/api/saves/${hash}/${crypto.randomUUID()}/resume`,
          { headers: other },
        )
      ).status,
    ).toBe(404);
    expect(
      (
        await request("/api/auth/get-session", {
          headers: { Cookie: other.Cookie },
        })
      ).status,
    ).toBe(200);
    expect(
      (await saves.list({ prefix: `${other["X-Zigba-Account"]}/` })).objects
        .length,
    ).toBeGreaterThan(0);
  });

  it("sweeps a PUT that lands after the first empty sweep even when its Worker crashes", async () => {
    const email = "stalled-run@example.com";
    await request(
      "/api/auth/sign-in/magic-link",
      post({ email }, "192.0.2.182"),
    );
    const Cookie = sessionCookie(
      await request(messageLink(), {
        headers: { "CF-Connecting-IP": "192.0.2.182" },
      }),
    );
    const session = (await (
      await request("/api/auth/get-session", { headers: { Cookie } })
    ).json()) as { user: { id: string } };
    const headers = { Cookie, "X-Zigba-Account": session.user.id };
    let entered!: () => void;
    let release!: () => void;
    const atPut = new Promise<void>((resolve) => {
      entered = resolve;
    });
    const blocked = new Promise<void>((resolve) => {
      release = resolve;
    });
    const delayedBucket = new Proxy(saves, {
      get(target, property) {
        if (property === "put")
          return async (...args: Parameters<R2Bucket["put"]>) => {
            entered();
            await blocked;
            await Reflect.apply(target.put, target, args);
            throw new Error("simulated Worker loss after R2 commit");
          };
        const value = Reflect.get(target, property);
        return typeof value === "function" ? value.bind(target) : value;
      },
    });
    const id = crypto.randomUUID();
    const pending = request(
      `/api/saves/${hash}/${id}/resume`,
      {
        method: "PUT",
        body: new Uint8Array(body),
        headers: {
          ...headers,
          Origin: origin,
          "Content-Type": "application/octet-stream",
          "X-Zigba-Revision": crypto.randomUUID(),
          "X-Zigba-Captured-At": "42",
          "X-Zigba-Digest": digest,
          "If-None-Match": "*",
        },
      },
      { SAVES: delayedBucket },
    );
    await atPut;
    const deletion = await request(
      "/api/account",
      { method: "DELETE", headers: { ...headers, Origin: origin } },
      { SAVES: saves },
    );
    expect(deletion.status).toBe(202);
    expect(await deletion.json()).toEqual({
      accountDeleted: true,
      cloudDeletion: "pending",
    });
    expect(
      await (
        await request("/api/auth/get-session", { headers: { Cookie } })
      ).json(),
    ).toBeNull();
    expect((await upload(headers, crypto.randomUUID())).status).toBe(401);
    expect(
      (await saves.list({ prefix: `${session.user.id}/` })).objects,
    ).toHaveLength(0);
    const log = vi.spyOn(console, "error").mockImplementation(() => {});
    release();
    expect((await pending).status).toBe(500);
    expect(log).toHaveBeenCalled();
    expect(
      (await saves.list({ prefix: `${session.user.id}/` })).objects,
    ).toHaveLength(1);
    const rateKey = `cloud-save:requests:${session.user.id}:3600000`;
    await database
      .prepare(
        "INSERT INTO rate_limit (id, key, count, last_request) VALUES (?, ?, 1, ?)",
      )
      .bind(rateKey, rateKey, Date.now())
      .run();
    await sweepDeletedAccounts(sweepBindings(), Date.now() + 6 * 60_000);
    expect(
      (await saves.list({ prefix: `${session.user.id}/` })).objects,
    ).toHaveLength(0);
    expect(
      await database
        .prepare("SELECT key FROM rate_limit WHERE key = ?")
        .bind(rateKey)
        .first(),
    ).toBeNull();
    expect(
      (
        await database
          .prepare("SELECT user_id FROM account_deletions WHERE user_id = ?")
          .bind(session.user.id)
          .first()
      )?.user_id,
    ).toBe(session.user.id);
    await sweepDeletedAccounts(sweepBindings(), Date.now() + 25 * 60 * 60_000);
    expect(
      (
        await database
          .prepare("SELECT user_id FROM account_deletions WHERE user_id = ?")
          .bind(session.user.id)
          .first()
      )?.user_id,
    ).toBe(session.user.id);
  });

  it("keeps the tombstone when R2 purge fails and retries on the scheduled sweep", async () => {
    const id = other["X-Zigba-Account"];
    const key = `${id}/unknown-layout/backup`;
    await saves.put(key, "private data");
    const failingBucket = new Proxy(saves, {
      get(target, property) {
        if (property === "delete")
          return async () => {
            throw new Error("test R2 failure");
          };
        const value = Reflect.get(target, property);
        return typeof value === "function" ? value.bind(target) : value;
      },
    });
    const init: RequestInit = {
      method: "DELETE",
      headers: { ...other, Origin: origin },
    };
    const failed = await request("/api/account", init, {
      SAVES: failingBucket,
    });
    expect(failed.status).toBe(202);
    expect(failed.headers.get("cache-control")).toBe("no-store");
    expect(await failed.json()).toEqual({
      accountDeleted: true,
      cloudDeletion: "pending",
    });
    expect(await saves.head(key)).not.toBeNull();
    expect(
      await (
        await request("/api/auth/get-session", {
          headers: { Cookie: other.Cookie },
        })
      ).json(),
    ).toBeNull();
    expect((await upload(other, crypto.randomUUID())).status).toBe(401);
    const log = vi.spyOn(console, "error").mockImplementation(() => {});
    await sweepDeletedAccounts(
      sweepBindings(failingBucket),
      Date.now() + 6 * 60_000,
    );
    expect(log).toHaveBeenCalledWith("Account deletion sweep failed");
    expect(await saves.head(key)).not.toBeNull();
    await sweepDeletedAccounts(sweepBindings(), Date.now() + 11 * 60_000);
    expect(await saves.head(key)).toBeNull();
    expect(
      (
        await database
          .prepare("SELECT user_id FROM account_deletions WHERE user_id = ?")
          .bind(id)
          .first()
      )?.user_id,
    ).toBe(id);
  });

  it("completes auth removal after a crash immediately following the durable gate", async () => {
    await request(
      "/api/auth/sign-in/magic-link",
      post({ email: "interrupted@example.com" }, "192.0.2.183"),
    );
    const Cookie = sessionCookie(
      await request(messageLink(), {
        headers: { "CF-Connecting-IP": "192.0.2.183" },
      }),
    );
    const session = (await (
      await request("/api/auth/get-session", { headers: { Cookie } })
    ).json()) as { user: { id: string } };
    const id = session.user.id;
    await saves.put(`${id}/unknown/backup`, "private data");
    await database
      .prepare(
        "INSERT INTO account_deletions (user_id, requested_at) VALUES (?, ?)",
      )
      .bind(id, Date.now())
      .run();
    expect(
      (await upload({ Cookie, "X-Zigba-Account": id }, crypto.randomUUID()))
        .status,
    ).toBe(503);
    expect(
      await (
        await request("/api/auth/get-session", { headers: { Cookie } })
      ).json(),
    ).toMatchObject({ user: { id } });
    const tasks: Promise<unknown>[] = [];
    app.scheduled({} as ScheduledEvent, sweepBindings(), {
      waitUntil(promise) {
        tasks.push(promise);
      },
    } as ExecutionContext);
    await Promise.all(tasks);
    expect(
      await (
        await request("/api/auth/get-session", { headers: { Cookie } })
      ).json(),
    ).toBeNull();
    expect((await saves.list({ prefix: `${id}/` })).objects).toHaveLength(0);
  });
});

describe("cloud quick slots", () => {
  const hash = "d".repeat(64);
  const state = (marker: number) =>
    gzipSync(Buffer.from([...Buffer.from("ZIGBAST1"), marker]));
  let owner: { Cookie: string; "X-Zigba-Account": string };
  let other: typeof owner;

  beforeAll(async () => {
    async function login(email: string, ip: string) {
      await request("/api/auth/sign-in/magic-link", post({ email }, ip));
      expect(messages.at(-1)?.to).toBe(email);
      const Cookie = sessionCookie(
        await request(messageLink(), { headers: { "CF-Connecting-IP": ip } }),
      );
      const session = (await (
        await request("/api/auth/get-session", { headers: { Cookie } })
      ).json()) as { user: { id: string } };
      return { Cookie, "X-Zigba-Account": session.user.id };
    }
    owner = await login("quick-owner@example.com", "192.0.2.200");
    other = await login("quick-other@example.com", "192.0.2.201");
  });

  function putQuick(
    account: typeof owner,
    run: string,
    slot: string | number,
    bytes: Buffer = state(1),
    precondition: Record<string, string> = { "If-None-Match": "*" },
    revision = crypto.randomUUID(),
  ) {
    return miniflare.dispatchFetch(
      `${origin}/api/saves/${hash}/${run}/quick/${slot}`,
      {
        method: "PUT",
        body: bytes,
        headers: {
          ...account,
          Origin: origin,
          "Content-Type": "application/octet-stream",
          "X-Zigba-Revision": revision,
          "X-Zigba-Captured-At": "77",
          "X-Zigba-Digest": createHash("sha256").update(bytes).digest("hex"),
          "X-Zigba-Playthrough-Name": encodeURIComponent("Quick run"),
          ...precondition,
        },
      },
    );
  }
  async function listQuick(account: typeof owner, run: string) {
    const response = await miniflare.dispatchFetch(
      `${origin}/api/saves/${hash}/${run}/quick`,
      { headers: account },
    );
    expect(response.headers.get("cache-control")).toBe("no-store");
    return {
      status: response.status,
      body: (await response.json()) as {
        slots: {
          slot: number;
          revision: string;
          digest: string;
          capturedAt: number;
          etag: string;
          uploadedAt: string;
        }[];
        limit: number;
      },
    };
  }

  it("stores private, conditional per-slot states that share the run's quota entry", async () => {
    const run = crypto.randomUUID();
    const revision = crypto.randomUUID();
    const first = await putQuick(owner, run, 1, state(1), undefined, revision);
    expect(first.status).toBe(200);
    const { etag } = (await first.json()) as { etag: string };
    // Same revision and bytes is an idempotent retry of a lost acknowledgement.
    expect(
      (await putQuick(owner, run, 1, state(1), undefined, revision)).status,
    ).toBe(200);
    expect((await putQuick(owner, run, 10, state(10))).status).toBe(200);
    const stale = await putQuick(owner, run, 1, state(2), {
      "If-Match": '"not-the-current-etag"',
    });
    expect(stale.status).toBe(412);
    const replaced = await putQuick(owner, run, 1, state(3), {
      "If-Match": etag,
    });
    expect(replaced.status).toBe(200);

    const listing = await listQuick(owner, run);
    expect(listing.status).toBe(200);
    expect(listing.body.limit).toBe(10);
    expect(listing.body.slots.map((slot) => slot.slot)).toEqual([1, 10]);
    expect(listing.body.slots[0]).toMatchObject({
      capturedAt: 77,
      etag: ((await replaced.json()) as { etag: string }).etag,
      digest: createHash("sha256").update(state(3)).digest("hex"),
    });
    const body = await miniflare.dispatchFetch(
      `${origin}/api/saves/${hash}/${run}/quick/1`,
      { headers: owner },
    );
    expect(body.status).toBe(200);
    expect(body.headers.get("X-Zigba-Digest")).toBe(
      listing.body.slots[0].digest,
    );
    expect(Buffer.from(await body.arrayBuffer())).toEqual(state(3));
    const head = await miniflare.dispatchFetch(
      `${origin}/api/saves/${hash}/${run}/quick/10`,
      { method: "HEAD", headers: owner },
    );
    expect(head.status).toBe(200);
    expect(head.headers.get("ETag")).toBe(listing.body.slots[1].etag);

    // Quick slots are not resumable runs and never leak across accounts.
    const listed = (await (
      await miniflare.dispatchFetch(`${origin}/api/saves`, { headers: owner })
    ).json()) as { saves: unknown[]; usage: { playthroughs: number } };
    expect(listed.saves).toEqual([]);
    expect(listed.usage.playthroughs).toBe(1);
    expect(
      (
        await miniflare.dispatchFetch(
          `${origin}/api/saves/${hash}/${run}/quick/1`,
          { headers: other },
        )
      ).status,
    ).toBe(404);
    expect((await listQuick(other, run)).body.slots).toEqual([]);

    const resume = gzipSync(Buffer.from("ZIGBAST1resume"));
    const resumed = await miniflare.dispatchFetch(
      `${origin}/api/saves/${hash}/${run}/resume`,
      {
        method: "PUT",
        body: resume,
        headers: {
          ...owner,
          Origin: origin,
          "Content-Type": "application/octet-stream",
          "If-None-Match": "*",
          "X-Zigba-Revision": crypto.randomUUID(),
          "X-Zigba-Captured-At": "78",
          "X-Zigba-Digest": createHash("sha256").update(resume).digest("hex"),
        },
      },
    );
    expect(resumed.status).toBe(200);
    const usage = (await (
      await miniflare.dispatchFetch(`${origin}/api/saves`, { headers: owner })
    ).json()) as { usage: { playthroughs: number } };
    expect(usage.usage.playthroughs).toBe(1);
  });

  it("validates slot numbers, state magic, sizes, and origin like resume saves", async () => {
    const run = crypto.randomUUID();
    for (const slot of ["0", "01", "100", "abc"]) {
      const response = await putQuick(owner, run, slot);
      expect(response.status).toBe(400);
      expect(await response.json()).toMatchObject({
        error: { code: "invalid_save_key" },
      });
    }
    const beyond = await putQuick(owner, run, 11);
    expect(beyond.status).toBe(409);
    expect(await beyond.json()).toMatchObject({
      error: { code: "quick_slot_limit", limit: 10 },
    });
    const magic = await putQuick(
      owner,
      run,
      2,
      gzipSync(Buffer.from("NOTASTATE")),
    );
    expect(magic.status).toBe(400);
    expect(await magic.json()).toMatchObject({
      error: { code: "invalid_quick" },
    });
    const oversized = await putQuick(
      owner,
      run,
      2,
      gzipSync(
        Buffer.concat([Buffer.from("ZIGBAST1"), Buffer.alloc(2 * 1024 * 1024)]),
      ),
    );
    expect(oversized.status).toBe(413);
    expect(await oversized.json()).toMatchObject({
      error: { code: "decoded_too_large" },
    });
    const crossOrigin = await miniflare.dispatchFetch(
      `${origin}/api/saves/${hash}/${run}/quick/2`,
      {
        method: "PUT",
        body: state(2),
        headers: { ...owner, Origin: "https://evil.example" },
      },
    );
    expect(crossOrigin.status).toBe(403);
    expect(
      (
        await miniflare.dispatchFetch(
          `${origin}/api/saves/${hash}/${run}/quick/2`,
          { headers: {} },
        )
      ).status,
    ).toBe(401);
    expect((await listQuick(owner, run)).body.slots).toEqual([]);
  });

  it("applies per-account D1 overrides and the code default before spending upload budget", async () => {
    const run = crypto.randomUUID();
    const id = other["X-Zigba-Account"];
    await database
      .prepare(
        "INSERT INTO account_limits (user_id, quick_slots) VALUES (?, ?)",
      )
      .bind(id, 3)
      .run();
    const budget = () =>
      database
        .prepare("SELECT count FROM rate_limit WHERE key = ?")
        .bind(`cloud-save:requests:${id}:3600000`)
        .first<{ count: number }>();
    expect((await putQuick(other, run, 3)).status).toBe(200);
    const before = (await budget())?.count;
    const rejected = await putQuick(other, run, 4);
    expect(rejected.status).toBe(409);
    expect(await rejected.json()).toMatchObject({
      error: { code: "quick_slot_limit", limit: 3 },
    });
    expect((await budget())?.count).toBe(before);
    expect((await listQuick(other, run)).body.limit).toBe(3);
    await expect(
      database
        .prepare(
          "UPDATE account_limits SET quick_slots = 100 WHERE user_id = ?",
        )
        .bind(id)
        .run(),
    ).rejects.toThrow();

    // A NULL override, like a missing row, falls back to the code default.
    await database
      .prepare("UPDATE account_limits SET quick_slots = NULL WHERE user_id = ?")
      .bind(id)
      .run();
    expect((await listQuick(other, run)).body.limit).toBe(10);
    expect((await putQuick(other, run, 10)).status).toBe(200);
    expect((await putQuick(other, run, 11)).status).toBe(409);
  });

  it("erases quick slots and the account override with the account", async () => {
    const run = crypto.randomUUID();
    const id = other["X-Zigba-Account"];
    expect((await putQuick(other, run, 1)).status).toBe(200);
    expect(
      (await saves.list({ prefix: `${id}/${hash}/${run}/quick/` })).objects,
    ).toHaveLength(1);
    const erased = await request(
      "/api/account",
      { method: "DELETE", headers: { ...other, Origin: origin } },
      { SAVES: saves as unknown as R2Bucket },
    );
    expect(erased.status).toBe(202);
    expect((await saves.list({ prefix: `${id}/` })).objects).toHaveLength(0);
    expect(
      await database
        .prepare("SELECT user_id FROM account_limits WHERE user_id = ?")
        .bind(id)
        .first(),
    ).toBeNull();
    expect((await putQuick(other, run, 1)).status).toBe(401);
  });
});
