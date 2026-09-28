import { drizzle } from "drizzle-orm/d1";
import { eq } from "drizzle-orm";
import { Hono } from "hono";
import type { MiddlewareHandler } from "hono";
import { z, ZodError, type ZodType } from "zod";
import {
  CatalogRuleError,
  getBase,
  getGame,
  getVersion,
  listBases,
  listGames,
  listVersions,
  putBase,
  putGame,
  putVersion,
} from "./catalog";
import { bases, games, versions } from "./schema";
import * as schema from "./schema";
import { seedCatalog } from "./seed";
import { baseInput, gameInput, resourceId, versionInput } from "./types";
import { createAuth, type AuthBindings } from "./auth";
import {
  activeAdminAccount,
  registerCloudSaveRoutes,
  sweepDeletedAccounts,
  type CloudSaveBindings,
} from "./cloud-saves";

export interface Bindings extends AuthBindings, CloudSaveBindings {
  ADMIN_SECRET?: string;
  PATCHES?: R2Bucket;
  PATCHES_URL?: string;
  /** The player's static build (Workers Static Assets). */
  ASSETS?: Fetcher;
}

type AppEnv = { Bindings: Bindings };

const app = new Hono<AppEnv>();

registerCloudSaveRoutes(app);

app.use("/api/auth/*", async (c, next) => {
  await next();
  // Set these on the final response, including native Workers responses from
  // Better Auth. Preparing headers before its handler is not sufficient.
  c.header("Cache-Control", "no-store");
  c.header("Referrer-Policy", "no-referrer");
});

app.use("/api/*", async (c, next) => {
  if (!c.env.DB)
    return c.json(
      {
        error: {
          code: "database_unavailable",
          message: "Catalog database is not configured",
        },
      },
      503,
    );
  await next();
});

app.all("/api/auth/*", async (c) => {
  const auth = createAuth(c.env);
  if (!auth)
    return c.json(
      {
        error: {
          code: "auth_unavailable",
          message: "Sign-in is not configured",
        },
      },
      503,
    );
  if (
    !["GET", "HEAD"].includes(c.req.method) &&
    c.req.header("Origin") !== auth.options.baseURL
  ) {
    return c.json(
      {
        error: {
          code: "invalid_origin",
          message: "Same-origin requests are required",
        },
      },
      403,
    );
  }
  if (
    c.req.path === "/api/auth/sign-in/magic-link" &&
    (!c.env.EMAIL || !c.env.EMAIL_FROM)
  ) {
    return c.json(
      {
        error: {
          code: "email_unavailable",
          message: "Email sign-in is not configured",
        },
      },
      503,
    );
  }
  return auth.handler(c.req.raw);
});

const requireAdmin: MiddlewareHandler<AppEnv> = async (c, next) => {
  const secret = c.env.ADMIN_SECRET;
  const auth = createAuth(c.env);
  if (!secret && !auth)
    return c.json(
      {
        error: {
          code: "management_disabled",
          message: "Catalog management is not configured",
        },
      },
      503,
    );

  const origin = c.req.header("Origin");
  if (origin && !["GET", "HEAD"].includes(c.req.method)) {
    let sameOrigin = false;
    try {
      sameOrigin = new URL(origin).origin === new URL(c.req.url).origin;
    } catch {
      // Invalid and opaque origins are cross-origin for management writes.
    }
    if (!sameOrigin)
      return c.json(
        {
          error: {
            code: "cross_origin_write",
            message: "Cross-origin management writes are forbidden",
          },
        },
        403,
      );
  }

  const authorization = c.req.header("Authorization");
  const bearer = Boolean(
    secret &&
    authorization &&
    (await secretsEqual(authorization, `Bearer ${secret}`)),
  );
  if (!bearer) {
    const session =
      auth && (await auth.api.getSession({ headers: c.req.raw.headers }));
    if (!session)
      return c.json(
        {
          error: {
            code: "unauthorized",
            message: "Valid administrator credentials are required",
          },
        },
        401,
      );
    if (!(await activeAdminAccount(c.env.DB!, session.user.id)))
      return c.json(
        {
          error: {
            code: "forbidden",
            message: "Administrator access is required",
          },
        },
        403,
      );
    if (
      !["GET", "HEAD"].includes(c.req.method) &&
      origin !== auth!.options.baseURL
    )
      return c.json(
        {
          error: {
            code: "invalid_origin",
            message: "Same-origin requests are required",
          },
        },
        403,
      );
  }
  await next();
};

app.use("/api/admin/*", requireAdmin);
app.use("/api/admin/*", async (c, next) => {
  await next();
  c.header("Cache-Control", "no-store");
  c.header("Referrer-Policy", "no-referrer");
});

app.get("/api/games", async (c) =>
  c.json({ games: await listGames(database(c.env.DB!)) }),
);

app.get("/api/games/:id", async (c) => {
  const id = parseId(c.req.param("id"));
  const game = await getGame(database(c.env.DB!), id);
  return game ? c.json({ game }) : notFound(c, "game");
});

app.get("/api/versions", async (c) =>
  c.json({ versions: await listVersions(database(c.env.DB!)) }),
);

app.get("/api/versions/:id", async (c) => {
  const id = parseId(c.req.param("id"));
  const version = await getVersion(database(c.env.DB!), id);
  return version ? c.json({ version }) : notFound(c, "version");
});

app.get("/api/bases", async (c) =>
  c.json({ bases: await listBases(database(c.env.DB!)) }),
);

app.get("/api/bases/:id", async (c) => {
  const id = parseId(c.req.param("id"));
  const base = await getBase(database(c.env.DB!), id);
  return base ? c.json({ base }) : notFound(c, "base");
});

app.get("/api/admin/games", async (c) =>
  c.json({ games: await listGames(database(c.env.DB!), false) }),
);
app.get("/api/admin/games/:id", async (c) => {
  const game = await getGame(
    database(c.env.DB!),
    parseId(c.req.param("id")),
    false,
  );
  return game ? c.json({ game }) : notFound(c, "game");
});
app.get("/api/admin/versions", async (c) =>
  c.json({ versions: await listVersions(database(c.env.DB!), false) }),
);
app.get("/api/admin/versions/:id", async (c) => {
  const version = await getVersion(
    database(c.env.DB!),
    parseId(c.req.param("id")),
    false,
  );
  return version ? c.json({ version }) : notFound(c, "version");
});
app.get("/api/admin/bases", async (c) =>
  c.json({ bases: await listBases(database(c.env.DB!)) }),
);
app.get("/api/admin/bases/:id", async (c) => {
  const base = await getBase(database(c.env.DB!), parseId(c.req.param("id")));
  return base ? c.json({ base }) : notFound(c, "base");
});

app.post("/api/admin/seed", async (c) => {
  await seedCatalog(database(c.env.DB!));
  return c.json({ seeded: true });
});

app.put("/api/admin/games/:id", async (c) => {
  const id = parseId(c.req.param("id"));
  const input = await parseJson(c, gameInput);
  await putGame(database(c.env.DB!), id, input);
  return c.json({ game: await getGame(database(c.env.DB!), id, false) });
});

app.put("/api/admin/bases/:id", async (c) => {
  const id = parseId(c.req.param("id"));
  const input = await parseJson(c, baseInput);
  await putBase(database(c.env.DB!), id, input);
  return c.json({ base: await getBase(database(c.env.DB!), id) });
});

app.put("/api/admin/versions/:id", async (c) => {
  const id = parseId(c.req.param("id"));
  const input = await parseJson(c, versionInput);
  await putVersion(database(c.env.DB!), id, input);
  return c.json({ version: await getVersion(database(c.env.DB!), id, false) });
});

app.patch("/api/admin/games/:id", async (c) => {
  const id = parseId(c.req.param("id"));
  const current = await getGame(database(c.env.DB!), id, false);
  if (!current) return notFound(c, "game");
  const {
    id: _id,
    versions: _versions,
    cloudPlaythroughs: _played,
    ...input
  } = current;
  const changes = await parseJson(c, gameInput.partial());
  await putGame(
    database(c.env.DB!),
    id,
    gameInput.parse({ ...input, ...changes }),
  );
  return c.json({ game: await getGame(database(c.env.DB!), id, false) });
});

app.patch("/api/admin/versions/:id", async (c) => {
  const id = parseId(c.req.param("id"));
  const current = await getVersion(database(c.env.DB!), id, false);
  if (!current) return notFound(c, "version");
  const { id: _id, requiredBase: _base, ...input } = current;
  const changes = await parseJson(
    c,
    z.strictObject(versionInput.shape).partial(),
  );
  await putVersion(
    database(c.env.DB!),
    id,
    versionInput.parse({ ...input, ...changes }),
  );
  return c.json({ version: await getVersion(database(c.env.DB!), id, false) });
});

app.patch("/api/admin/bases/:id", async (c) => {
  const id = parseId(c.req.param("id"));
  const current = await getBase(database(c.env.DB!), id);
  if (!current) return notFound(c, "base");
  const { id: _id, ...input } = current;
  const changes = await parseJson(c, baseInput.partial());
  await putBase(
    database(c.env.DB!),
    id,
    baseInput.parse({ ...input, ...changes }),
  );
  return c.json({ base: await getBase(database(c.env.DB!), id) });
});

// Patches are immutable, content-addressed objects. Upload before publishing
// metadata: a failed D1 write may leave an orphan, never a broken catalog URL.
app.put("/api/admin/versions/:id/patch", async (c) => {
  const id = parseId(c.req.param("id"));
  const version = await getVersion(database(c.env.DB!), id, false);
  if (!version) return notFound(c, "version");
  if (!version.requiredBaseId)
    return c.json(
      {
        error: {
          code: "invalid_request",
          message: "A patch needs a required base",
        },
      },
      400,
    );
  if (!c.env.PATCHES || !c.env.PATCHES_URL)
    return c.json(
      {
        error: {
          code: "patches_unavailable",
          message: "Patch uploads are not configured",
        },
      },
      503,
    );
  const format = z
    .enum(["bps", "ups", "ips"])
    .safeParse(c.req.header("X-Patch-Format"));
  if (
    !format.success ||
    c.req.header("Content-Type")?.split(";", 1)[0].trim().toLowerCase() !==
      "application/octet-stream" ||
    c.req.header("Content-Encoding")
  )
    return c.json(
      {
        error: {
          code: "invalid_request",
          message: "Use a raw patch body and X-Patch-Format: bps, ups or ips",
        },
      },
      400,
    );
  const limit = 40 * 1024 * 1024;
  if (Number(c.req.header("Content-Length")) > limit)
    return c.json(
      { error: { code: "patch_too_large", message: "Patch exceeds 40 MiB" } },
      413,
    );
  const reader = c.req.raw.body?.getReader();
  if (!reader)
    return c.json(
      { error: { code: "invalid_patch", message: "A patch body is required" } },
      400,
    );
  const chunks: Uint8Array[] = [];
  let size = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.length;
    if (size > limit) {
      await reader.cancel();
      return c.json(
        { error: { code: "patch_too_large", message: "Patch exceeds 40 MiB" } },
        413,
      );
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  const magic = { bps: "BPS1", ups: "UPS1", ips: "PATCH" }[format.data];
  if (
    bytes.length <= magic.length ||
    new TextDecoder().decode(bytes.subarray(0, magic.length)) !== magic
  )
    return c.json(
      {
        error: {
          code: "invalid_patch",
          message: "Patch header does not match its format",
        },
      },
      400,
    );
  const hash = Array.from(
    new Uint8Array(await crypto.subtle.digest("SHA-1", bytes)),
    (byte) => byte.toString(16).padStart(2, "0"),
  ).join("");
  const key = `${hash}.${format.data}`;
  const existing = await c.env.PATCHES.head(key);
  if (existing && existing.size !== bytes.length)
    return c.json(
      {
        error: {
          code: "conflict",
          message: "Patch object has a different size",
        },
      },
      409,
    );
  if (!existing) await c.env.PATCHES.put(key, bytes);
  const patchUrl = `${c.env.PATCHES_URL.replace(/\/$/, "")}/${key}`;
  await database(c.env.DB!)
    .update(versions)
    .set({ patchUrl, patchFormat: format.data, patchSize: bytes.length })
    .where(eq(versions.id, id));
  return c.json({ version: await getVersion(database(c.env.DB!), id, false) });
});

app.delete("/api/admin/games/:id", async (c) => {
  await database(c.env.DB!)
    .delete(games)
    .where(eq(games.id, parseId(c.req.param("id"))));
  return c.body(null, 204);
});

app.delete("/api/admin/versions/:id", async (c) => {
  await database(c.env.DB!)
    .delete(versions)
    .where(eq(versions.id, parseId(c.req.param("id"))));
  return c.body(null, 204);
});

app.delete("/api/admin/bases/:id", async (c) => {
  await database(c.env.DB!)
    .delete(bases)
    .where(eq(bases.id, parseId(c.req.param("id"))));
  return c.body(null, 204);
});

// The player's pages are client-side routes that all load its index.html;
// wrangler.jsonc sends them to the Worker first. Other paths stay 404s.
for (const path of [
  "/library",
  "/library/:gameId",
  "/player",
  "/share/state/:shareId",
])
  app.get(path, (c) =>
    c.env.ASSETS ? c.env.ASSETS.fetch(new URL("/", c.req.url)) : c.notFound(),
  );

app.notFound((c) =>
  c.json({ error: { code: "not_found", message: "Route not found" } }, 404),
);

app.onError((error, c) => {
  if (error instanceof ZodError)
    return c.json(
      {
        error: {
          code: "invalid_request",
          message: "Request validation failed",
          issues: error.issues,
        },
      },
      400,
    );
  if (error instanceof CatalogRuleError)
    return c.json(
      { error: { code: "invalid_request", message: error.message } },
      400,
    );
  if (error instanceof SyntaxError)
    return c.json(
      {
        error: {
          code: "invalid_json",
          message: "Request body must be valid JSON",
        },
      },
      400,
    );
  const message = errorMessages(error);
  if (message.includes("UNIQUE constraint failed"))
    return c.json(
      {
        error: { code: "conflict", message: "Catalog identity already exists" },
      },
      409,
    );
  if (message.includes("FOREIGN KEY constraint failed"))
    return c.json(
      {
        error: {
          code: "invalid_relation",
          message:
            "Referenced catalog record does not exist or is still in use",
        },
      },
      409,
    );
  console.error("Catalog request failed", error);
  return c.json(
    { error: { code: "database_error", message: "Catalog request failed" } },
    500,
  );
});

function database(binding: D1Database) {
  return drizzle(binding, { schema });
}

function parseId(value: string): string {
  return resourceId.parse(value);
}

async function parseJson<T>(
  c: Parameters<MiddlewareHandler<AppEnv>>[0],
  validator: ZodType<T>,
): Promise<T> {
  if (
    !c.req.header("Content-Type")?.toLowerCase().startsWith("application/json")
  )
    throw new ZodError([
      {
        code: "custom",
        path: [],
        message: "Content-Type must be application/json",
      },
    ]);
  return validator.parse(await c.req.json());
}

function notFound(
  c: Parameters<MiddlewareHandler<AppEnv>>[0],
  resource: string,
) {
  return c.json(
    { error: { code: "not_found", message: `Catalog ${resource} not found` } },
    404,
  );
}

async function secretsEqual(left: string, right: string): Promise<boolean> {
  const encoder = new TextEncoder();
  const [leftHash, rightHash] = await Promise.all([
    crypto.subtle.digest("SHA-256", encoder.encode(left)),
    crypto.subtle.digest("SHA-256", encoder.encode(right)),
  ]);
  const a = new Uint8Array(leftHash);
  const b = new Uint8Array(rightHash);
  let difference = 0;
  for (let index = 0; index < a.length; index++)
    difference |= a[index] ^ b[index];
  return difference === 0;
}

function errorMessages(error: unknown): string {
  const messages: string[] = [];
  let current: unknown = error;
  while (current && typeof current === "object") {
    if ("message" in current && typeof current.message === "string")
      messages.push(current.message);
    current = "cause" in current ? current.cause : null;
  }
  return messages.join("\n");
}

export default Object.assign(app, {
  scheduled(_event: ScheduledEvent, env: Bindings, ctx: ExecutionContext) {
    ctx.waitUntil(sweepDeletedAccounts(env));
  },
});
