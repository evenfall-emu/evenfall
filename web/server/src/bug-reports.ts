import type { Hono } from "hono";
import { bodyLimit } from "hono/body-limit";
import { z } from "zod";
import { createAuth } from "./auth";
import {
  BATTERY_SIZES,
  consumeLimits,
  failure,
  gunzipBounded,
  MAX_RESUME,
  sha256Hex,
  startsWith,
  STATE_MAGIC,
  type CloudSaveBindings,
} from "./cloud-saves";
import { sha1 } from "./types";

// Reports stay readable until the bucket's lifecycle rule on this prefix
// deletes them. `report.json` is written last, so it marks a complete report.
const REPORT_PREFIX = "report/";
const DAY = 24 * 60 * 60_000;
// Abuse prevention: per hashed client address, and one instance-wide ceiling
// that bounds storage however many addresses send reports.
const ADDRESS_DAILY_LIMIT = 10;
const INSTANCE_DAILY_LIMIT = 500;

const reportForm = z
  .object({
    message: z.string().trim().min(1).max(2000),
    title: z.string().trim().max(160).optional(),
    build: z.string().max(64).optional(),
    rom: sha1.optional(),
    includeAccount: z.literal("1").optional(),
    state: z.instanceof(File).optional(),
    battery: z.instanceof(File).optional(),
  })
  .refine((form) => form.rom || !(form.state || form.battery));

export interface BugReportBindings extends CloudSaveBindings {
  /** PostHog project key. Without it and POSTHOG_HOST, no event is sent. */
  POSTHOG_KEY?: string;
  /** PostHog ingestion host, such as https://eu.i.posthog.com. */
  POSTHOG_HOST?: string;
}

/** What `report.json` holds, and what the admin route returns. */
export type BugReport = {
  id: string;
  createdAt: string;
  message: string;
  rom: string | null;
  title: string | null;
  build: string | null;
  userAgent: string | null;
  /** Set only when a signed-in reporter chose to include their account. */
  accountId: string | null;
  state: boolean;
  /** The battery save's size in bytes, when one is attached. */
  battery: number | null;
};

/**
 * `POST /api/reports` accepts a bug report from anyone; the admin routes read
 * it back. Register after the `/api/admin/*` guard so it covers them.
 */
export function registerBugReportRoutes(
  app: Hono<{ Bindings: BugReportBindings }>,
): void {
  app.post(
    "/api/reports",
    bodyLimit({
      // Both files at their largest, plus the text fields and framing.
      maxSize: MAX_RESUME + 192 * 1024,
      onError: (c) =>
        failure(c, 413, "report_too_large", "The report is too large"),
    }),
    async (c) => {
      c.header("Cache-Control", "no-store");
      const { SAVES: bucket, DB: db } = c.env;
      if (!bucket || !db)
        return failure(
          c,
          503,
          "reports_unavailable",
          "Bug reports are not configured",
        );
      if (c.req.header("Origin") !== new URL(c.req.url).origin)
        return failure(
          c,
          403,
          "invalid_origin",
          "Same-origin requests are required",
        );
      const parsed = reportForm.safeParse(
        Object.fromEntries(await c.req.formData().catch(() => new FormData())),
      );
      if (!parsed.success)
        return failure(c, 400, "invalid_report", "The report is invalid");
      const form = parsed.data;

      const state =
        form.state && new Uint8Array(await form.state.arrayBuffer());
      const battery =
        form.battery && new Uint8Array(await form.battery.arrayBuffer());
      if (
        (state &&
          !startsWith(
            await gunzipBounded(state, MAX_RESUME).catch(
              () => new Uint8Array(),
            ),
            STATE_MAGIC,
          )) ||
        (battery && !BATTERY_SIZES.has(battery.byteLength))
      )
        return failure(c, 400, "invalid_report", "The report is invalid");

      // Attached only on request and from a valid session; a stale or missing
      // session sends the report anonymously.
      const session =
        form.includeAccount &&
        (await createAuth(c.env)
          ?.api.getSession({ headers: c.req.raw.headers })
          .catch(() => null));

      // Counted only for valid reports, so malformed requests cost nothing.
      const address = await sha256Hex(
        new TextEncoder().encode(c.req.header("CF-Connecting-IP") ?? ""),
      );
      if (
        !(await consumeLimits(db, `address-${address}`, DAY, [
          ["reports", 1, ADDRESS_DAILY_LIMIT],
        ])) ||
        !(await consumeLimits(db, "instance", DAY, [
          ["reports", 1, INSTANCE_DAILY_LIMIT],
        ]))
      )
        return failure(
          c,
          429,
          "reports_rate_limited",
          "Too many reports. Try again later.",
        );

      const report: BugReport = {
        id: crypto.randomUUID(),
        createdAt: new Date().toISOString(),
        message: form.message,
        rom: form.rom ?? null,
        title: form.title || null,
        build: form.build || null,
        userAgent: c.req.header("User-Agent")?.slice(0, 512) ?? null,
        accountId: session ? session.user.id : null,
        state: Boolean(state),
        battery: battery?.byteLength ?? null,
      };
      const prefix = `${REPORT_PREFIX}${report.id}/`;
      if (state) await bucket.put(`${prefix}state`, state);
      if (battery) await bucket.put(`${prefix}battery`, battery);
      await bucket.put(`${prefix}report.json`, JSON.stringify(report), {
        httpMetadata: { contentType: "application/json" },
      });

      // The stored report is the record; the event only announces it, so a
      // PostHog failure never fails the request.
      const announced = announce(
        c.env,
        report,
        new URL(c.req.url).origin,
      ).catch(() => console.error("Bug report event failed"));
      try {
        c.executionCtx.waitUntil(announced);
      } catch {
        await announced;
      }
      return c.json({ id: report.id }, 201);
    },
  );

  // The state downloads decompressed, so Import state loads it as-is.
  app.get("/api/admin/reports/:id/:file?", async (c) => {
    const { id, file = "report.json" } = c.req.param();
    const extension = {
      "report.json": "json",
      state: "state",
      battery: "sav",
    }[file];
    const object =
      c.env.SAVES && extension && z.uuid().safeParse(id).success
        ? await c.env.SAVES.get(`${REPORT_PREFIX}${id}/${file}`)
        : null;
    if (!object)
      return failure(
        c,
        404,
        "report_not_found",
        "This report has expired or is absent",
      );
    if (extension === "json")
      return new Response(object.body, {
        headers: { "Content-Type": "application/json" },
      });
    return new Response(
      file === "state"
        ? object.body.pipeThrough(new DecompressionStream("gzip"))
        : object.body,
      {
        headers: {
          "Content-Type": "application/octet-stream",
          "Content-Disposition": `attachment; filename="report-${id}.${extension}"`,
        },
      },
    );
  });
}

/**
 * Sends the `bug_report` event with the report's text and links to its files,
 * which only admins can open. The Worker sends it, so PostHog never sees the
 * reporter's address, and the event creates no person profile.
 */
async function announce(
  env: BugReportBindings,
  report: BugReport,
  origin: string,
): Promise<void> {
  if (!env.POSTHOG_KEY || !env.POSTHOG_HOST) return;
  const links = `${origin}/api/admin/reports/${report.id}`;
  const response = await fetch(new URL("/i/v0/e/", env.POSTHOG_HOST), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      api_key: env.POSTHOG_KEY,
      event: "bug_report",
      distinct_id: `bug-report:${report.id}`,
      timestamp: report.createdAt,
      properties: {
        $process_person_profile: false,
        $geoip_disable: true,
        report_id: report.id,
        message: report.message,
        rom_sha1: report.rom,
        title: report.title,
        build: report.build,
        user_agent: report.userAgent,
        account_id: report.accountId,
        report_url: links,
        state_url: report.state ? `${links}/state` : null,
        battery_url: report.battery ? `${links}/battery` : null,
      },
    }),
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) throw new Error(`PostHog answered ${response.status}`);
}
