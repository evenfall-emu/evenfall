import type { Context, Hono, MiddlewareHandler } from "hono";
import type { ContentfulStatusCode } from "hono/utils/http-status";
import { createAuth, type AuthBindings } from "./auth";

const MAX_COMPRESSED = 2 * 1024 * 1024;
const MAX_RESUME = 2 * 1024 * 1024;
const MAX_BATTERY = 128 * 1024;
const PLAYTHROUGH_LIMIT = 100;
// Quick slots are numbered 1..limit per playthrough. The default lives here;
// D1 `account_limits` overrides it per account. The route accepts at most two
// digits, so an override can never address unbounded keys.
const QUICK_SLOT_DEFAULT = 10;
const QUICK_SLOT = /^[1-9][0-9]?$/;
// Uploaded states must be the core's own save states. Stored objects are never
// checked again, so states in an older format still list, download, delete
// and erase like any other object; the player decides what it can load.
const STATE_MAGIC = new TextEncoder().encode("EVENFALL");
/**
 * Deployed identifier: the cloud protocol's headers, shared with players
 * already in use. Renaming one needs a coordinated player and Worker change.
 */
const HEADER = {
  account: "X-Zigba-Account",
  rom: "X-Zigba-Rom",
  revision: "X-Zigba-Revision",
  capturedAt: "X-Zigba-Captured-At",
  digest: "X-Zigba-Digest",
  title: "X-Zigba-Title",
  playthroughName: "X-Zigba-Playthrough-Name",
  uploadedAt: "X-Zigba-Uploaded-At",
  romIncluded: "X-Zigba-Rom-Included",
} as const;
const BATTERY_SIZES = new Set([
  512,
  8 * 1024,
  32 * 1024,
  64 * 1024,
  128 * 1024,
]);
// Saves are keyed by the SHA-1 of the raw ROM; uploads carry a SHA-256 digest.
const HASH = /^[0-9a-f]{40}$/;
const DIGEST = /^[0-9a-f]{64}$/;
const UUID =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
// Shares stay readable until the bucket's lifecycle rule on SHARE_PREFIX
// deletes them: at least its age, and typically less than a day longer.
const SHARE_PREFIX = "share/";
// Abuse prevention, not a quota: new links per account per fixed window. It
// answers like the upload budget, so clients never present it separately.
const SHARE_LIMIT = 100;
const SHARE_WINDOW_MS = 24 * 60 * 60_000;
// 128 random bits as unpadded base64url.
const SHARE_ID = /^[A-Za-z0-9_-]{22}$/;
// A GBA ROM is at most 32 MiB; gzip can grow incompressible data slightly.
const MAX_ROM_COMPRESSED = 33 * 1024 * 1024;

export interface CloudSaveBindings extends AuthBindings {
  SAVES?: R2Bucket;
}

type SaveKind = "resume" | "battery" | "quick";
type Env = { Bindings: CloudSaveBindings };

export function registerCloudSaveRoutes(app: Hono<Env>): void {
  const privateHeaders: MiddlewareHandler<Env> = async (c, next) => {
    try {
      await next();
    } finally {
      c.header("Cache-Control", "no-store");
    }
  };
  app.use("/api/saves*", privateHeaders);
  app.use("/api/shares*", privateHeaders);
  app.use("/api/account", privateHeaders);

  app.get("/api/saves", async (c) => {
    const account = await saveAccount(c);
    if (account instanceof Response) return account;
    const cursor = c.req.query("cursor");
    if (cursor && cursor.length > 2048)
      return failure(c, 400, "invalid_cursor", "Invalid listing cursor");
    const prefix = `${account.userId}/`;
    const page = await c.env.SAVES!.list({
      prefix,
      cursor,
      limit: 100,
      include: ["customMetadata"],
    });
    const saves = page.objects.flatMap((object) => {
      const parts = object.key.slice(prefix.length).split("/");
      const [hash, playthroughId, kind] =
        parts.length === 2 ? [parts[0], "default", parts[1]] : parts;
      if (
        !HASH.test(hash) ||
        !validPlaythroughId(playthroughId) ||
        kind !== "resume" ||
        parts.length > 3
      )
        return [];
      return [
        {
          hash,
          playthroughId,
          playthroughName:
            object.customMetadata?.playthroughName ??
            (playthroughId === "default" ? "First adventure" : "Playthrough"),
          title: object.customMetadata?.title,
          capturedAt: Number(object.customMetadata?.capturedAt ?? 0),
          uploadedAt: object.uploaded.toISOString(),
        },
      ];
    });
    const names = await catalogTitles(
      c.env.DB!,
      saves.map((save) => save.hash),
    );
    return c.json({
      saves: saves.map((save) => ({
        ...save,
        title:
          names.get(save.hash) ?? save.title ?? `Game ${save.hash.slice(0, 8)}`,
      })),
      cursor: page.truncated ? page.cursor : null,
      usage: {
        playthroughs: await playthroughCount(c.env.DB!, account.userId),
        limit: PLAYTHROUGH_LIMIT,
      },
    });
  });

  app.delete("/api/saves/:hash/:playthroughId", async (c) => {
    const account = await saveAccount(c);
    if (account instanceof Response) return account;
    if (c.req.header("Origin") !== new URL(account.baseURL).origin)
      return failure(
        c,
        403,
        "invalid_origin",
        "Same-origin requests are required",
      );
    const hash = c.req.param("hash");
    const playthroughId = c.req.param("playthroughId");
    if (!HASH.test(hash) || !validPlaythroughId(playthroughId))
      return failure(
        c,
        400,
        "invalid_save_key",
        "Invalid save hash or playthrough ID",
      );
    const prefix = `${account.userId}/${hash}/${playthroughId}/`;
    while (true) {
      const page = await c.env.SAVES!.list({ prefix, limit: 1000 });
      if (!page.objects.length) break;
      await c.env.SAVES!.delete(page.objects.map((object) => object.key));
    }
    if (playthroughId === "default")
      await c.env.SAVES!.delete([
        `${account.userId}/${hash}/resume`,
        `${account.userId}/${hash}/battery`,
      ]);
    await c.env
      .DB!.prepare(
        "DELETE FROM playthrough WHERE user_id = ? AND hash = ? AND id = ?",
      )
      .bind(account.userId, hash, playthroughId)
      .run();
    return c.body(null, 204);
  });

  // Registered before the generic kind route, which would otherwise claim
  // `quick` as an invalid kind. Quick slots never use the legacy namespace.
  app.get("/api/saves/:hash/:playthroughId/quick", async (c) => {
    const account = await saveAccount(c);
    if (account instanceof Response) return account;
    const hash = c.req.param("hash");
    const playthroughId = c.req.param("playthroughId");
    if (!HASH.test(hash) || !validPlaythroughId(playthroughId))
      return failure(
        c,
        400,
        "invalid_save_key",
        "Invalid save hash or playthrough ID",
      );
    const prefix = `${account.userId}/${hash}/${playthroughId}/quick/`;
    const slots = [];
    let cursor: string | undefined;
    do {
      const page = await c.env.SAVES!.list({
        prefix,
        cursor,
        limit: 100,
        include: ["customMetadata"],
      });
      for (const object of page.objects) {
        const slot = object.key.slice(prefix.length);
        if (!QUICK_SLOT.test(slot)) continue;
        slots.push({
          slot: Number(slot),
          revision: object.customMetadata?.revision ?? "",
          digest: object.customMetadata?.digest ?? "",
          capturedAt: Number(object.customMetadata?.capturedAt ?? 0),
          etag: object.httpEtag,
          uploadedAt: object.uploaded.toISOString(),
        });
      }
      cursor = page.truncated ? page.cursor : undefined;
    } while (cursor);
    return c.json({
      slots: slots.sort((a, b) => a.slot - b.slot),
      limit: await quickSlotLimit(c.env, account.userId),
    });
  });

  app.on(
    ["GET", "HEAD", "PUT"],
    "/api/saves/:hash/:playthroughId/quick/:slot",
    async (c) => {
      const account = await saveAccount(c);
      if (account instanceof Response) return account;
      const { userId, baseURL } = account;
      const hash = c.req.param("hash");
      const playthroughId = c.req.param("playthroughId");
      const slot = c.req.param("slot");
      if (
        !HASH.test(hash) ||
        !validPlaythroughId(playthroughId) ||
        !QUICK_SLOT.test(slot)
      )
        return failure(
          c,
          400,
          "invalid_save_key",
          "Invalid save hash, playthrough ID, or quick slot",
        );
      const key = `${userId}/${hash}/${playthroughId}/quick/${slot}`;
      if (c.req.method === "PUT")
        return putSave(c, baseURL, key, "quick", userId, hash, playthroughId, {
          slot: Number(slot),
        });
      return readSave(c, key);
    },
  );

  app.on(
    ["GET", "HEAD", "PUT"],
    "/api/saves/:hash/:playthroughId/:kind",
    async (c) => {
      const account = await saveAccount(c);
      if (account instanceof Response) return account;
      const { userId, baseURL } = account;

      const hash = c.req.param("hash");
      const playthroughId = c.req.param("playthroughId");
      const kind = c.req.param("kind");
      if (
        !HASH.test(hash) ||
        !validPlaythroughId(playthroughId) ||
        (kind !== "resume" && kind !== "battery")
      )
        return failure(
          c,
          400,
          "invalid_save_key",
          "Invalid save hash, playthrough ID, or kind",
        );

      // Legacy backups remain readable and writable through the reserved default
      // identity. New default runs use the four-segment namespace.
      const legacy = `${userId}/${hash}/${kind}`;
      const key =
        playthroughId === "default" && (await c.env.SAVES!.head(legacy))
          ? legacy
          : `${userId}/${hash}/${playthroughId}/${kind}`;
      if (c.req.method === "PUT")
        return putSave(c, baseURL, key, kind, userId, hash, playthroughId);
      return readSave(c, key);
    },
  );

  // A share is one uploaded state that anyone with its link can read until
  // the lifecycle rule on SHARE_PREFIX deletes it. Shares live outside account
  // prefixes so that one rule covers them; reads are gated on account deletion.
  app.post("/api/shares", async (c) => {
    const account = await saveAccount(c);
    if (account instanceof Response) return account;
    const { userId, baseURL } = account;
    const rejected = uploadRejected(c, baseURL);
    if (rejected) return rejected;
    const hash = c.req.header(HEADER.rom);
    const expectedDigest = c.req.header(HEADER.digest);
    if (
      !hash ||
      !HASH.test(hash) ||
      !expectedDigest ||
      !DIGEST.test(expectedDigest)
    )
      return failure(
        c,
        400,
        "invalid_metadata",
        "ROM hash or digest is invalid",
      );
    const title = readLabel(c, HEADER.title, "invalid_title", "Title");
    if (title instanceof Response) return title;
    const playthroughName = readLabel(
      c,
      HEADER.playthroughName,
      "invalid_playthrough_name",
      "Playthrough name",
    );
    if (playthroughName instanceof Response) return playthroughName;
    if (playthroughName === undefined)
      return failure(
        c,
        400,
        "invalid_playthrough_name",
        "A playthrough name is required",
      );

    const upload = await receiveUpload(c, userId, expectedDigest, MAX_RESUME);
    if (upload instanceof Response) return upload;
    if (!startsWith(upload.decoded, STATE_MAGIC))
      return failure(c, 400, "invalid_state", "State magic is invalid");
    // Counted only for valid states, so rejected uploads cost no links.
    if (
      !(await consumeLimits(c.env.DB!, userId, SHARE_WINDOW_MS, [
        ["shares", 1, SHARE_LIMIT],
      ]))
    )
      return failure(
        c,
        429,
        "save_rate_limited",
        "Cloud-save upload budget exceeded",
      );
    const id = newShareId();
    const key = shareKey(id);
    const object = await c.env.SAVES!.put(key, upload.compressed, {
      onlyIf: { etagDoesNotMatch: "*" },
      httpMetadata: { contentType: "application/octet-stream" },
      customMetadata: {
        userId,
        hash,
        playthroughName,
        digest: upload.digest,
        ...(title ? { title } : {}),
      },
      sha256: upload.digest,
    });
    // A deletion that began during the upload must not leave a readable share.
    if (await accountDeleting(c.env.DB!, userId)) {
      if (object) await c.env.SAVES!.delete(key).catch(() => {});
      return failure(
        c,
        503,
        "deletion_in_progress",
        "Account deletion is in progress",
      );
    }
    if (!object)
      return failure(c, 500, "share_failed", "The state could not be shared");
    return c.json(
      {
        id,
        url: `${new URL(baseURL).origin}/share/state/${id}`,
        canIncludeRom: await isAdmin(c.env.DB!, userId),
      },
      201,
    );
  });

  // Attaches the game's ROM to one of an admin account's shares. R2 checks
  // the body's SHA-256; readers check the ROM's SHA-1 against the share
  // before storing it, so the Worker neither decodes nor hashes the body.
  app.put("/api/shares/:id/rom", async (c) => {
    const account = await saveAccount(c);
    if (account instanceof Response) return account;
    const { userId, baseURL } = account;
    const rejected = uploadRejected(c, baseURL);
    if (rejected) return rejected;
    if (!(await isAdmin(c.env.DB!, userId)))
      return failure(
        c,
        403,
        "rom_sharing_forbidden",
        "This account cannot share games",
      );
    const id = c.req.param("id");
    const share = SHARE_ID.test(id)
      ? await c.env.SAVES!.head(shareKey(id))
      : null;
    if (!share?.customMetadata?.hash || share.customMetadata.userId !== userId)
      return failure(
        c,
        404,
        "share_not_found",
        "This shared state has expired or does not exist",
      );
    const expectedDigest = c.req.header(HEADER.digest);
    if (!expectedDigest || !DIGEST.test(expectedDigest))
      return failure(c, 400, "invalid_metadata", "Digest is invalid");
    // One attempt from the request budget; ROM bytes bypass the byte budget,
    // which is sized for saves.
    if (!(await consumeBudget(c.env.DB!, userId, 1, 0)))
      return failure(
        c,
        429,
        "save_rate_limited",
        "Cloud-save upload budget exceeded",
      );
    let body: Uint8Array<ArrayBuffer>;
    try {
      body = await readBounded(c.req.raw.body, MAX_ROM_COMPRESSED);
    } catch {
      return failure(
        c,
        413,
        "compressed_too_large",
        "Compressed ROM exceeds 33 MiB",
      );
    }
    const key = shareRomKey(id);
    try {
      await c.env.SAVES!.put(key, body, {
        httpMetadata: { contentType: "application/octet-stream" },
        customMetadata: {
          userId,
          hash: share.customMetadata.hash,
          digest: expectedDigest,
        },
        sha256: expectedDigest,
      });
    } catch {
      return failure(
        c,
        400,
        "digest_mismatch",
        "Compressed ROM digest does not match",
      );
    }
    if (await accountDeleting(c.env.DB!, userId)) {
      await c.env.SAVES!.delete(key).catch(() => {});
      return failure(
        c,
        503,
        "deletion_in_progress",
        "Account deletion is in progress",
      );
    }
    return c.body(null, 204);
  });

  // Public like the state: the link is the capability. Readable while its
  // share is, and only while the sharer is still an admin.
  app.get("/api/shares/:id/rom", async (c) => {
    if (!c.env.DB || !c.env.SAVES)
      return failure(
        c,
        503,
        "saves_unavailable",
        "Cloud saves are not configured",
      );
    const id = c.req.param("id");
    const [share, rom] = SHARE_ID.test(id)
      ? await Promise.all([
          c.env.SAVES.head(shareKey(id)),
          c.env.SAVES.get(shareRomKey(id)),
        ])
      : [null, null];
    const userId = rom?.customMetadata?.userId;
    if (
      !share ||
      !rom ||
      !userId ||
      share.customMetadata?.userId !== userId ||
      !(await activeAdminAccount(c.env.DB, userId))
    ) {
      await rom?.body.cancel();
      return failure(
        c,
        404,
        "rom_not_found",
        "This shared game has expired or does not exist",
      );
    }
    return new Response(rom.body, {
      headers: {
        "Cache-Control": "no-store",
        "Content-Type": "application/octet-stream",
        "Referrer-Policy": "no-referrer",
        [HEADER.digest]: rom.customMetadata?.digest ?? "",
      },
    });
  });

  // Public: the link is the capability. Sends no account identity.
  app.get("/api/shares/:id", async (c) => {
    if (!c.env.DB || !c.env.SAVES)
      return failure(
        c,
        503,
        "saves_unavailable",
        "Cloud saves are not configured",
      );
    const id = c.req.param("id");
    const object = SHARE_ID.test(id)
      ? await c.env.SAVES.get(shareKey(id))
      : null;
    const metadata = object?.customMetadata;
    if (
      !object ||
      !metadata?.userId ||
      !metadata.hash ||
      (await accountDeleting(c.env.DB, metadata.userId))
    ) {
      await object?.body.cancel();
      return failure(
        c,
        404,
        "share_not_found",
        "This shared state has expired or does not exist",
      );
    }
    const title =
      (await catalogTitles(c.env.DB, [metadata.hash])).get(metadata.hash) ??
      metadata.title ??
      `Game ${metadata.hash.slice(0, 8)}`;
    const romIncluded =
      (await c.env.SAVES.head(shareRomKey(id)))?.customMetadata?.userId ===
        metadata.userId &&
      (await activeAdminAccount(c.env.DB, metadata.userId));
    return new Response(object.body, {
      headers: {
        "Cache-Control": "no-store",
        "Content-Type": "application/octet-stream",
        "Referrer-Policy": "no-referrer",
        ...(romIncluded ? { [HEADER.romIncluded]: "1" } : {}),
        [HEADER.rom]: metadata.hash,
        [HEADER.title]: encodeURIComponent(title),
        [HEADER.playthroughName]: encodeURIComponent(
          metadata.playthroughName ?? "Playthrough",
        ),
        [HEADER.digest]: metadata.digest ?? "",
      },
    });
  });

  app.delete("/api/account", async (c) => {
    const account = await saveAccount(c, true);
    if (account instanceof Response) return account;
    if (c.req.header("Origin") !== new URL(account.baseURL).origin)
      return failure(
        c,
        403,
        "invalid_origin",
        "Same-origin requests are required",
      );
    const db = c.env.DB!;
    const userId = account.userId;
    await db
      .prepare(
        `INSERT INTO account_deletions (user_id, requested_at) VALUES (?, ?)
      ON CONFLICT(user_id) DO NOTHING`,
      )
      .bind(userId, Date.now())
      .run();
    // The independent tombstone gates new admissions before auth is removed.
    // A scheduled pass finishes this step if the Worker stops here.
    try {
      await removeAccountRecords(db, userId);
    } catch {
      return failure(
        c,
        503,
        "deletion_pending",
        "Account deletion is pending; retry or wait for automatic cleanup",
      );
    }
    // R2 has no transaction with D1. An empty sweep cannot retire the job:
    // an already-issued R2 PUT may complete after this response.
    try {
      await sweepPrefix(c.env, userId);
      await markSwept(db, userId, Date.now());
    } catch {
      // Cron retries; auth has already been deleted and cannot be restored.
    }
    return c.json({ accountDeleted: true, cloudDeletion: "pending" }, 202);
  });
}

async function saveAccount(c: Context<Env>, allowDeleting = false) {
  if (!c.env.DB || !c.env.SAVES)
    return failure(
      c,
      503,
      "saves_unavailable",
      "Cloud saves are not configured",
    );
  const auth = createAuth(c.env);
  if (!auth)
    return failure(c, 503, "auth_unavailable", "Sign-in is not configured");
  const session = await auth.api.getSession({ headers: c.req.raw.headers });
  if (!session)
    return failure(c, 401, "unauthorized", "A valid session is required");
  if (c.req.header(HEADER.account) !== session.user.id)
    return failure(
      c,
      403,
      "account_mismatch",
      "Account does not match the session",
    );
  if (!allowDeleting && (await accountDeleting(c.env.DB, session.user.id)))
    return failure(
      c,
      503,
      "deletion_in_progress",
      "Account deletion is in progress",
    );
  return { userId: session.user.id, baseURL: auth.options.baseURL };
}

/** The account's deletion tombstone closes admissions and hides its shares. */
async function accountDeleting(
  db: D1Database,
  userId: string,
): Promise<boolean> {
  return Boolean(
    await db
      .prepare("SELECT user_id FROM account_deletions WHERE user_id = ?")
      .bind(userId)
      .first(),
  );
}

/**
 * Names exact catalog outputs of public releases (hosted patch, not hidden),
 * so uploads that predate or ignore the catalog show its title. Other ROMs are
 * absent and keep their uploaded label.
 */
async function catalogTitles(
  db: D1Database,
  hashes: string[],
): Promise<Map<string, string>> {
  if (!hashes.length) return new Map();
  const { results } = await db
    .prepare(
      `SELECT v.output_sha1 AS hash, g.display_name || ' · ' || v.display_name AS title
       FROM versions v JOIN games g ON g.id = v.game_id
       WHERE v.patch_url IS NOT NULL AND v.visibility != 'hidden'
         AND v.output_sha1 IN (${hashes.map(() => "?").join(",")})`,
    )
    .bind(...hashes)
    .all<{ hash: string; title: string }>();
  return new Map(results.map((row) => [row.hash, row.title]));
}

/** Whether the operator flagged the account as admin in `account_limits`. */
async function isAdmin(db: D1Database, userId: string): Promise<boolean> {
  return Boolean(
    await db
      .prepare("SELECT 1 FROM account_limits WHERE user_id = ? AND admin = 1")
      .bind(userId)
      .first(),
  );
}

/** The account retains admin privileges only while it is not being deleted. */
export async function activeAdminAccount(
  db: D1Database,
  userId: string,
): Promise<boolean> {
  return (await isAdmin(db, userId)) && !(await accountDeleting(db, userId));
}

function shareKey(id: string): string {
  return `${SHARE_PREFIX}state/${id}`;
}

function shareRomKey(id: string): string {
  return `${SHARE_PREFIX}rom/${id}`;
}

function newShareId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

/** Every upload is a same-origin, raw `application/octet-stream` gzip body. */
function uploadRejected(
  c: Context<Env>,
  baseURL: string,
): Response | undefined {
  if (c.req.header("Origin") !== new URL(baseURL).origin)
    return failure(
      c,
      403,
      "invalid_origin",
      "Same-origin requests are required",
    );
  if (
    c.req.header("Content-Type")?.split(";", 1)[0].trim().toLowerCase() !==
      "application/octet-stream" ||
    c.req.header("Content-Encoding")
  )
    return failure(
      c,
      415,
      "invalid_content_type",
      "A raw application/octet-stream gzip body is required",
    );
}

/**
 * An optional `encodeURIComponent` header of at most 160 code points of
 * non-blank text without control characters.
 */
function readLabel(
  c: Context<Env>,
  header: string,
  code: string,
  noun: string,
): string | undefined | Response {
  const encoded = c.req.header(header);
  if (encoded === undefined) return undefined;
  try {
    const value = decodeURIComponent(encoded);
    if (
      value.trim() &&
      [...value].length <= 160 &&
      !/[\u0000-\u001f\u007f]/.test(value)
    )
      return value;
  } catch {
    // Malformed escapes fail like any other invalid label.
  }
  return failure(
    c,
    400,
    code,
    `${noun} must be URI-encoded text of at most 160 characters`,
  );
}

/**
 * Spends the account's upload budget on one request and then on the body's
 * compressed bytes, checks the body's SHA-256 and decodes at most
 * `decodedLimit` bytes. Rejected bodies have still consumed budget.
 */
async function receiveUpload(
  c: Context<Env>,
  userId: string,
  expectedDigest: string,
  decodedLimit: number,
): Promise<
  | {
      compressed: Uint8Array<ArrayBuffer>;
      decoded: Uint8Array<ArrayBuffer>;
      digest: string;
    }
  | Response
> {
  if (!(await consumeBudget(c.env.DB!, userId, 1, 0)))
    return failure(
      c,
      429,
      "save_rate_limited",
      "Cloud-save upload budget exceeded",
    );

  let compressed: Uint8Array<ArrayBuffer>;
  try {
    compressed = await readBounded(c.req.raw.body, MAX_COMPRESSED);
  } catch {
    return failure(
      c,
      413,
      "compressed_too_large",
      "Compressed save exceeds 2 MiB",
    );
  }
  if (!(await consumeBudget(c.env.DB!, userId, 0, compressed.byteLength)))
    return failure(
      c,
      429,
      "save_rate_limited",
      "Cloud-save upload budget exceeded",
    );
  const digest = await sha256Hex(compressed);
  if (digest !== expectedDigest)
    return failure(
      c,
      400,
      "digest_mismatch",
      "Compressed save digest does not match",
    );

  try {
    const stream = new Blob([compressed.buffer])
      .stream()
      .pipeThrough(new DecompressionStream("gzip"));
    const decoded = await readBounded(stream, decodedLimit);
    return { compressed, decoded, digest };
  } catch (error) {
    const oversized = error instanceof RangeError;
    return failure(
      c,
      oversized ? 413 : 400,
      oversized ? "decoded_too_large" : "invalid_gzip",
      oversized ? "Decoded save is too large" : "Save body is not valid gzip",
    );
  }
}

async function readSave(c: Context<Env>, key: string) {
  const object =
    c.req.method === "HEAD"
      ? await c.env.SAVES!.head(key)
      : await c.env.SAVES!.get(key);
  if (!object) return failure(c, 404, "save_not_found", "Cloud save not found");
  const headers = saveHeaders(object);
  return c.req.method === "HEAD"
    ? new Response(null, { status: 200, headers })
    : new Response((object as R2ObjectBody).body, { status: 200, headers });
}

async function putSave(
  c: Context<Env>,
  baseURL: string,
  key: string,
  kind: SaveKind,
  userId: string,
  hash: string,
  playthroughId: string,
  quick?: { slot: number },
) {
  const rejected = uploadRejected(c, baseURL);
  if (rejected) return rejected;

  const revision = c.req.header(HEADER.revision);
  const capturedAt = c.req.header(HEADER.capturedAt);
  const expectedDigest = c.req.header(HEADER.digest);
  if (
    !revision ||
    !UUID.test(revision) ||
    !capturedAt ||
    !/^(0|[1-9][0-9]*)$/.test(capturedAt) ||
    !Number.isSafeInteger(Number(capturedAt)) ||
    !expectedDigest ||
    !DIGEST.test(expectedDigest)
  )
    return failure(
      c,
      400,
      "invalid_metadata",
      "Revision, captured timestamp, or digest is invalid",
    );

  const titleLabel = readLabel(c, HEADER.title, "invalid_title", "Title");
  if (titleLabel instanceof Response) return titleLabel;
  const nameLabel = readLabel(
    c,
    HEADER.playthroughName,
    "invalid_playthrough_name",
    "Playthrough name",
  );
  if (nameLabel instanceof Response) return nameLabel;
  let title: string | undefined = titleLabel;
  let playthroughName: string | undefined = nameLabel;

  const ifNoneMatch = c.req.header("If-None-Match");
  const ifMatch = c.req.header("If-Match");
  if (
    !(
      (ifNoneMatch === "*" && ifMatch === undefined) ||
      (ifMatch !== undefined &&
        /^"[^"\s]+"$/.test(ifMatch) &&
        ifNoneMatch === undefined)
    )
  )
    return failure(
      c,
      400,
      "invalid_precondition",
      'Use exactly If-None-Match: * or If-Match: "etag"',
    );

  // Checked before the upload budget: a slot the account cannot use should
  // not consume requests while a client backs off.
  if (quick) {
    const limit = await quickSlotLimit(c.env, userId);
    if (quick.slot > limit)
      return c.json(
        {
          error: {
            code: "quick_slot_limit",
            message: `This account allows ${limit} cloud quick slots per playthrough`,
            limit,
          },
        },
        409,
      );
  }

  const upload = await receiveUpload(
    c,
    userId,
    expectedDigest,
    kind === "battery" ? MAX_BATTERY : MAX_RESUME,
  );
  if (upload instanceof Response) return upload;
  const { compressed, decoded, digest } = upload;
  if (kind === "resume" && !startsWith(decoded, STATE_MAGIC))
    return failure(c, 400, "invalid_resume", "Resume state magic is invalid");
  if (kind === "quick" && !startsWith(decoded, STATE_MAGIC))
    return failure(
      c,
      400,
      "invalid_quick",
      "Quick-slot state magic is invalid",
    );
  if (kind === "battery" && !BATTERY_SIZES.has(decoded.length))
    return failure(
      c,
      400,
      "invalid_battery_size",
      "Battery save length is unsupported",
    );

  const db = c.env.DB!;
  return writeSave();

  async function writeSave() {
    const claimed = await db
      .prepare(
        `INSERT INTO playthrough (user_id, hash, id, name)
    SELECT ?, ?, ?, ? FROM user WHERE id = ?
      AND NOT EXISTS (SELECT 1 FROM account_deletions WHERE user_id = ?)
      AND (SELECT count(*) FROM playthrough WHERE user_id = ?) < ?
    ON CONFLICT(user_id, hash, id) DO NOTHING RETURNING id`,
      )
      .bind(
        userId,
        hash,
        playthroughId,
        playthroughName ??
          (playthroughId === "default" ? "First adventure" : "Playthrough"),
        userId,
        userId,
        userId,
        PLAYTHROUGH_LIMIT,
      )
      .first();
    if (await accountDeleting(db, userId))
      return failure(
        c,
        503,
        "deletion_in_progress",
        "Account deletion is in progress; retry deletion",
      );
    if (
      !claimed &&
      !(await db
        .prepare(
          "SELECT id FROM playthrough WHERE user_id = ? AND hash = ? AND id = ?",
        )
        .bind(userId, hash, playthroughId)
        .first())
    )
      return failure(
        c,
        409,
        "playthrough_limit",
        "This account has reached the 100-playthrough limit",
      );

    const existing = await c.env.SAVES!.head(key);
    if (existing && existing.customMetadata?.revision === revision) {
      if (existing.customMetadata?.digest !== digest)
        return failure(
          c,
          409,
          "revision_conflict",
          "Revision was already used for different bytes",
        );
      return putSuccess(c, existing, revision!);
    }

    const onlyIf: R2Conditional =
      ifNoneMatch === "*"
        ? { etagDoesNotMatch: "*" }
        : { etagMatches: ifMatch!.slice(1, -1) };
    title ??= existing?.customMetadata?.title;
    playthroughName ??= existing?.customMetadata?.playthroughName;
    const object = await c.env.SAVES!.put(key, compressed, {
      onlyIf,
      httpMetadata: { contentType: "application/octet-stream" },
      customMetadata: {
        revision: revision!,
        capturedAt: capturedAt!,
        digest,
        ...(title ? { title } : {}),
        playthroughName:
          playthroughName ??
          (playthroughId === "default" ? "First adventure" : "Playthrough"),
      },
      sha256: await crypto.subtle.digest("SHA-256", compressed),
    });
    if (await accountDeleting(db, userId)) {
      if (object) await c.env.SAVES!.delete(key).catch(() => {});
      return failure(
        c,
        503,
        "deletion_in_progress",
        "Account deletion is in progress; retry deletion",
      );
    }
    if (!object) {
      const winner = await c.env.SAVES!.head(key);
      if (winner && winner.customMetadata?.revision === revision) {
        if (winner.customMetadata?.digest !== digest)
          return failure(
            c,
            409,
            "revision_conflict",
            "Revision was already used for different bytes",
          );
        return putSuccess(c, winner, revision!);
      }
      return failure(
        c,
        412,
        "precondition_failed",
        "Cloud save changed; existing save was kept",
      );
    }
    if (playthroughName)
      await db
        .prepare(
          "UPDATE playthrough SET name = ? WHERE user_id = ? AND hash = ? AND id = ?",
        )
        .bind(playthroughName!, userId, hash, playthroughId)
        .run();
    return putSuccess(c, object, revision!);
  }
}

async function removeAccountRecords(
  db: D1Database,
  userId: string,
): Promise<void> {
  const user = await db
    .prepare("SELECT email FROM user WHERE id = ?")
    .bind(userId)
    .first<{ email: string }>();
  const emailHash = user
    ? await sha256Hex(new TextEncoder().encode(user.email.trim().toLowerCase()))
    : null;
  await db.batch([
    ...(user
      ? [
          db
            .prepare(
              `DELETE FROM verification WHERE identifier = ? OR
      CASE WHEN json_valid(value) THEN json_extract(value, '$.email') = ? ELSE 0 END`,
            )
            .bind(user.email, user.email),
        ]
      : []),
    db
      .prepare(
        "DELETE FROM rate_limit WHERE (instr(key, 'cloud-save:') = 1 AND instr(key, ?) > 0) OR (? IS NOT NULL AND instr(key, ?) = 1)",
      )
      .bind(`:${userId}:`, emailHash, `recipient:${emailHash}:`),
    db.prepare("DELETE FROM playthrough WHERE user_id = ?").bind(userId),
    db.prepare("DELETE FROM account_limits WHERE user_id = ?").bind(userId),
    ...(user ? [db.prepare("DELETE FROM user WHERE id = ?").bind(userId)] : []),
  ]);
}

async function sweepPrefix(
  env: CloudSaveBindings,
  userId: string,
): Promise<void> {
  while (true) {
    const page = await env.SAVES!.list({ prefix: `${userId}/`, limit: 1000 });
    if (!page.objects.length) break;
    await env.SAVES!.delete(page.objects.map((object) => object.key));
  }
}

async function markSwept(
  db: D1Database,
  userId: string,
  now: number,
): Promise<void> {
  await db
    .prepare("UPDATE account_deletions SET last_swept_at = ? WHERE user_id = ?")
    .bind(now, userId)
    .run();
}

export async function sweepDeletedAccounts(
  env: CloudSaveBindings,
  now = Date.now(),
): Promise<void> {
  if (!env.DB || !env.SAVES)
    throw new Error("Cloud deletion bindings unavailable");
  // Frequent early/failure retries; old empty tombstones remain and are swept
  // daily because a stalled upload can finish after any earlier empty pass.
  let cursor = "";
  while (true) {
    const jobs = await env.DB.prepare(
      `SELECT user_id FROM account_deletions
      WHERE user_id > ? AND (last_swept_at IS NULL OR last_swept_at < CASE
        WHEN requested_at > ? THEN ? ELSE ? END)
      ORDER BY user_id LIMIT 100`,
    )
      .bind(cursor, now - 60 * 60_000, now - 5 * 60_000, now - 24 * 60 * 60_000)
      .all<{ user_id: string }>();
    for (const { user_id } of jobs.results) {
      try {
        await removeAccountRecords(env.DB, user_id);
        await sweepPrefix(env, user_id);
        await markSwept(env.DB, user_id, now);
      } catch {
        // Leave the job due for the next cron rather than retiring its obligation.
        console.error("Account deletion sweep failed");
      }
    }
    if (jobs.results.length < 100) break;
    cursor = jobs.results.at(-1)!.user_id;
  }
}

function validPlaythroughId(id: string): boolean {
  return id === "default" || UUID.test(id);
}

async function quickSlotLimit(
  env: CloudSaveBindings,
  userId: string,
): Promise<number> {
  // The table's CHECK constraint bounds overrides to 0-99.
  const override = await env
    .DB!.prepare("SELECT quick_slots FROM account_limits WHERE user_id = ?")
    .bind(userId)
    .first<{ quick_slots: number | null }>();
  return override?.quick_slots ?? QUICK_SLOT_DEFAULT;
}

async function playthroughCount(
  db: D1Database,
  userId: string,
): Promise<number> {
  return (
    (
      await db
        .prepare("SELECT count(*) AS count FROM playthrough WHERE user_id = ?")
        .bind(userId)
        .first<{ count: number }>()
    )?.count ?? 0
  );
}

async function consumeBudget(
  db: D1Database,
  userId: string,
  requests: number,
  bytes: number,
): Promise<boolean> {
  return consumeLimits(db, userId, 60 * 60_000, [
    ["requests", requests, 120],
    ["bytes", bytes, 20 * 1024 * 1024],
  ]);
}

/**
 * Atomically adds each positive amount to the account's fixed-window counter
 * of that name unless it would pass the maximum. The `cloud-save:` keys hold
 * the user ID, so account deletion removes them.
 */
async function consumeLimits(
  db: D1Database,
  userId: string,
  window: number,
  limits: readonly (readonly [name: string, amount: number, max: number])[],
): Promise<boolean> {
  const now = Date.now();
  const cutoff = now - window;
  const results = await db.batch(
    limits
      .filter(([, amount]) => amount > 0)
      .map(([name, amount, max]) => {
        const key = `cloud-save:${name}:${userId}:${window}`;
        return db
          .prepare(
            `
      INSERT INTO rate_limit (id, key, count, last_request) VALUES (?, ?, ?, ?)
      ON CONFLICT(key) DO UPDATE SET
        count = CASE WHEN last_request <= ? THEN excluded.count ELSE count + excluded.count END,
        last_request = CASE WHEN last_request <= ? THEN excluded.last_request ELSE last_request END
      WHERE last_request <= ? OR count + excluded.count <= ?
      RETURNING count
    `,
          )
          .bind(key, key, amount, now, cutoff, cutoff, cutoff, max);
      }),
  );
  return results.every((result) => result.results.length === 1);
}

async function readBounded(
  stream: ReadableStream<Uint8Array> | null,
  limit: number,
): Promise<Uint8Array<ArrayBuffer>> {
  if (!stream) return new Uint8Array();
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let length = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > limit) throw new RangeError("body too large");
      chunks.push(value);
    }
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
  const output = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    output.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return output;
}

function saveHeaders(object: R2Object): Headers {
  return new Headers({
    "Cache-Control": "no-store",
    "Content-Type": "application/octet-stream",
    ETag: object.httpEtag,
    [HEADER.revision]: object.customMetadata?.revision ?? "",
    [HEADER.capturedAt]: object.customMetadata?.capturedAt ?? "",
    [HEADER.digest]: object.customMetadata?.digest ?? "",
    [HEADER.uploadedAt]: object.uploaded.toISOString(),
  });
}

function putSuccess(c: Context<Env>, object: R2Object, revision: string) {
  return c.json({
    etag: object.httpEtag,
    revision,
    uploadedAt: object.uploaded.toISOString(),
  });
}

function failure(
  c: Context<Env>,
  status: ContentfulStatusCode,
  code: string,
  message: string,
) {
  return c.json({ error: { code, message } }, status);
}

async function sha256Hex(bytes: Uint8Array<ArrayBuffer>): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

function startsWith(bytes: Uint8Array, prefix: Uint8Array): boolean {
  return (
    bytes.length >= prefix.length &&
    prefix.every((byte, index) => bytes[index] === byte)
  );
}
