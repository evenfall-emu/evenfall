import { readFile, readdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { gunzipSync, gzipSync } from "node:zlib";
import { convertV4MiniflareOptions, Miniflare } from "miniflare";
import {
  afterAll,
  afterEach,
  beforeAll,
  describe,
  expect,
  it,
  vi,
} from "vite-plus/test";
import app, { type Bindings } from "../src/index";

const origin = "https://evenfall.test";
const adminSecret = "test-only-secret";
const posthogHost = "https://posthog.test";
const rom = "a".repeat(40);
let miniflare: Miniflare;
let database: D1Database;
let saves: R2Bucket;

beforeAll(async () => {
  miniflare = new Miniflare(
    convertV4MiniflareOptions({
      modules: true,
      script: "export default { fetch() { return new Response('test') } }",
      d1Databases: ["DB"],
      r2Buckets: ["SAVES"],
    }),
  );
  database = await miniflare.getD1Database("DB");
  saves = (await miniflare.getR2Bucket("SAVES")) as unknown as R2Bucket;
  const directory = fileURLToPath(new URL("../migrations", import.meta.url));
  for (const name of (await readdir(directory))
    .filter((file) => file.endsWith(".sql"))
    .sort()) {
    const migration = await readFile(`${directory}/${name}`, "utf8");
    for (const statement of migration.split("--> statement-breakpoint"))
      if (statement.trim()) await database.prepare(statement).run();
  }
});

afterAll(async () => {
  await miniflare.dispose();
});

afterEach(async () => {
  vi.restoreAllMocks();
  // Each test starts with empty rate-limit windows and no stored reports.
  await database.prepare("DELETE FROM rate_limit").run();
  const listed = await saves.list({ prefix: "report/" });
  if (listed.objects.length)
    await saves.delete(listed.objects.map((object) => object.key));
});

function bindings(extra: Partial<Bindings> = {}): Bindings {
  return {
    DB: database,
    SAVES: saves,
    ADMIN_SECRET: adminSecret,
    POSTHOG_KEY: "phc_test",
    POSTHOG_HOST: posthogHost,
    ...extra,
  };
}

/** Records PostHog requests and passes every other fetch through. */
function capturePostHog(status = 200) {
  const events: Record<string, unknown>[] = [];
  const original = globalThis.fetch;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    const url = input instanceof Request ? input.url : String(input);
    if (!url.startsWith(posthogHost)) return original(input, init);
    events.push(JSON.parse(init?.body as string));
    return new Response(null, { status });
  });
  return events;
}

function state(fill: string) {
  return gzipSync(Buffer.from(`EVENFALL${fill}`));
}

function form(fields: Record<string, string | Uint8Array>) {
  const body = new FormData();
  for (const [name, value] of Object.entries(fields))
    body.append(
      name,
      typeof value === "string" ? value : new Blob([new Uint8Array(value)]),
    );
  return body;
}

function report(
  fields: Record<string, string | Uint8Array>,
  headers: Record<string, string> = {},
  env = bindings(),
) {
  return app.request(
    `${origin}/api/reports`,
    {
      method: "POST",
      headers: { Origin: origin, "CF-Connecting-IP": "192.0.2.1", ...headers },
      body: form(fields),
    },
    env,
  );
}

function admin(path: string, authorized = true) {
  return app.request(
    `${origin}/api/admin/reports/${path}`,
    { headers: authorized ? { Authorization: `Bearer ${adminSecret}` } : {} },
    bindings(),
  );
}

async function storedKeys() {
  return (await saves.list({ prefix: "report/" })).objects.map(
    (object) => object.key,
  );
}

describe("bug reports", () => {
  it("stores an anonymous report that only admins can read back", async () => {
    const events = capturePostHog();
    const saved = state("game memory");
    const battery = new Uint8Array(8 * 1024).fill(7);
    const created = await report(
      {
        message: "  The cave music stops  ",
        title: "Test Game",
        build: "0123abc",
        rom,
        state: saved,
        battery,
      },
      { "User-Agent": "TestBrowser/1" },
    );
    expect(created.status).toBe(201);
    const { id } = (await created.json()) as { id: string };
    expect(id).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
    );

    expect((await admin(id, false)).status).toBe(401);
    const json = await admin(id);
    expect(json.status).toBe(200);
    expect(await json.json()).toMatchObject({
      id,
      message: "The cave music stops",
      rom,
      title: "Test Game",
      build: "0123abc",
      userAgent: "TestBrowser/1",
      accountId: null,
      state: true,
      battery: 8 * 1024,
    });
    // The state downloads decompressed, ready for the player's Import state.
    const stateDownload = await admin(`${id}/state`);
    expect(stateDownload.headers.get("Content-Disposition")).toBe(
      `attachment; filename="report-${id}.state"`,
    );
    expect(Buffer.from(await stateDownload.arrayBuffer())).toEqual(
      gunzipSync(saved),
    );
    const batteryDownload = await admin(`${id}/battery`);
    expect(new Uint8Array(await batteryDownload.arrayBuffer())).toEqual(
      battery,
    );
    expect(batteryDownload.headers.get("Cache-Control")).toBe("no-store");

    expect(events).toHaveLength(1);
    expect(events[0]).toMatchObject({
      api_key: "phc_test",
      event: "bug_report",
      distinct_id: `bug-report:${id}`,
      properties: {
        $process_person_profile: false,
        report_id: id,
        message: "The cave music stops",
        rom_sha1: rom,
        account_id: null,
        state_url: `${origin}/api/admin/reports/${id}/state`,
        battery_url: `${origin}/api/admin/reports/${id}/battery`,
      },
    });
  });

  it("accepts a message without game files and links no files", async () => {
    const events = capturePostHog();
    const created = await report({ message: "The menu is unreadable" });
    expect(created.status).toBe(201);
    const { id } = (await created.json()) as { id: string };
    expect(await storedKeys()).toEqual([`report/${id}/report.json`]);
    expect(await (await admin(id)).json()).toMatchObject({
      rom: null,
      state: false,
      battery: null,
    });
    expect((await admin(`${id}/state`)).status).toBe(404);
    expect(events[0]).toMatchObject({
      properties: { state_url: null, battery_url: null },
    });
  });

  it("rejects invalid reports before storing or counting anything", async () => {
    capturePostHog();
    const valid = { message: "Bug", rom, state: state("x") };
    const statuses = [
      // Cross-origin pages cannot send reports through a visitor's browser.
      (await report(valid, { Origin: "https://evil.example" })).status,
      (
        await app.request(
          `${origin}/api/reports`,
          {
            method: "POST",
            headers: { Origin: origin, "Content-Type": "application/json" },
            body: "{}",
          },
          bindings(),
        )
      ).status,
      (await report({ ...valid, message: "   " })).status,
      (await report({ ...valid, message: "x".repeat(2001) })).status,
      (await report({ ...valid, state: gzipSync(Buffer.from("NOTSTATE")) }))
        .status,
      (await report({ ...valid, state: Buffer.from("EVENFALL raw") })).status,
      // Battery saves have the sizes of real save chips only.
      (await report({ ...valid, battery: new Uint8Array(1000) })).status,
      (await report({ message: "Bug", state: state("x") })).status,
      (await report({ ...valid, build: "x".repeat(65) })).status,
      (await report({ ...valid, state: "not a file" })).status,
    ];
    expect(statuses).toEqual([
      403, 400, 400, 400, 400, 400, 400, 400, 400, 400,
    ]);
    expect(await storedKeys()).toEqual([]);
    const counted = await database
      .prepare("SELECT count(*) AS n FROM rate_limit")
      .first<{ n: number }>();
    expect(counted?.n).toBe(0);
  });

  it("refuses bodies larger than both files at their largest", async () => {
    const response = await report({
      message: "Huge",
      rom,
      state: new Uint8Array(2.5 * 1024 * 1024),
    });
    expect(response.status).toBe(413);
    expect(await storedKeys()).toEqual([]);
  });

  it("limits reports per address and keeps the address only as a hash", async () => {
    capturePostHog();
    for (let index = 0; index < 10; index++)
      expect((await report({ message: `Bug ${index}` })).status).toBe(201);
    const limited = await report({ message: "One more" });
    expect(limited.status).toBe(429);
    expect(await limited.json()).toMatchObject({
      error: { code: "reports_rate_limited" },
    });
    expect(
      (
        await report(
          { message: "Another address" },
          { "CF-Connecting-IP": "192.0.2.2" },
        )
      ).status,
    ).toBe(201);
    const { results } = await database
      .prepare("SELECT key FROM rate_limit")
      .all<{ key: string }>();
    expect(results.length).toBeGreaterThan(0);
    for (const { key } of results) expect(key).not.toContain("192.0.2.");
  });

  it("attaches no account without a valid session", async () => {
    const events = capturePostHog();
    const created = await report({ message: "Bug", includeAccount: "1" });
    expect(created.status).toBe(201);
    expect(events[0]).toMatchObject({ properties: { account_id: null } });
  });

  it("keeps the report when PostHog fails or is not configured", async () => {
    const events = capturePostHog(500);
    vi.spyOn(console, "error").mockImplementation(() => {});
    expect((await report({ message: "PostHog is down" })).status).toBe(201);
    expect(events).toHaveLength(1);
    expect(
      (
        await report(
          { message: "No PostHog" },
          {},
          bindings({ POSTHOG_KEY: undefined }),
        )
      ).status,
    ).toBe(201);
    expect(events).toHaveLength(1);
    expect(
      (await storedKeys()).filter((key) => key.endsWith("report.json")),
    ).toHaveLength(2);
  });
});
