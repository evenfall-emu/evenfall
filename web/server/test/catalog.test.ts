import { readFile, readdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { convertV4MiniflareOptions, Miniflare } from "miniflare";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import app, { type Bindings } from "../src/index";
import { seedBases, seedGames, seedVersions } from "../src/seed";

const adminSecret = "test-only-secret";
let miniflare: Miniflare;
let database: D1Database;

beforeAll(async () => {
  miniflare = new Miniflare(
    convertV4MiniflareOptions({
      modules: true,
      script: "export default { fetch() { return new Response('test') } }",
      d1Databases: ["DB"],
    }),
  );
  database = await miniflare.getD1Database("DB");
  const migrationsDirectory = fileURLToPath(
    new URL("../migrations", import.meta.url),
  );
  const migrationFiles = (await readdir(migrationsDirectory))
    .filter((file) => file.endsWith(".sql"))
    .sort();
  if (!migrationFiles.length) throw new Error("Generated migration is missing");
  for (const migrationFile of migrationFiles) {
    const migration = await readFile(
      `${migrationsDirectory}/${migrationFile}`,
      "utf8",
    );
    for (const statement of migration.split("--> statement-breakpoint")) {
      if (statement.trim()) await database.prepare(statement).run();
    }
  }
});

afterAll(async () => {
  await miniflare.dispose();
});

function request(
  path: string,
  init: RequestInit = {},
  bindings: Bindings = { DB: database, ADMIN_SECRET: adminSecret },
) {
  return app.request(`https://zigba.test${path}`, init, bindings);
}

function adminHeaders(extra: Record<string, string> = {}) {
  return {
    Authorization: `Bearer ${adminSecret}`,
    "Content-Type": "application/json",
    ...extra,
  };
}

describe("catalog API", () => {
  it("fails cleanly when the D1 binding is absent", async () => {
    const response = await request("/api/games", {}, {});
    expect(response.status).toBe(503);
    expect(await response.json()).toMatchObject({
      error: { code: "database_unavailable" },
    });
  });

  it("fails closed when management is unconfigured or unauthorized", async () => {
    const noSecret = await request(
      "/api/admin/seed",
      { method: "POST" },
      { DB: database },
    );
    expect(noSecret.status).toBe(503);
    expect(await noSecret.json()).toMatchObject({
      error: { code: "management_disabled" },
    });

    const unauthorized = await request("/api/admin/seed", { method: "POST" });
    expect(unauthorized.status).toBe(401);
  });

  it("rejects cross-origin management writes even with credentials", async () => {
    const response = await request("/api/admin/seed", {
      method: "POST",
      headers: adminHeaders({ Origin: "https://attacker.example" }),
    });
    expect(response.status).toBe(403);
    expect(await response.json()).toMatchObject({
      error: { code: "cross_origin_write" },
    });
  });

  it("applies the explicit seed twice without duplicates", async () => {
    for (let run = 0; run < 2; run++) {
      const response = await request("/api/admin/seed", {
        method: "POST",
        headers: adminHeaders(),
      });
      expect(response.status).toBe(200);
    }

    const gamesResponse = await request("/api/games");
    expect(gamesResponse.status).toBe(200);
    const body = (await gamesResponse.json()) as {
      games: Array<{ id: string; versionCount: number }>;
    };
    expect(body.games).toHaveLength(2);
    expect(body.games).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ id: "heart-n-soul", versionCount: 1 }),
        expect.objectContaining({ id: "elite-redux", versionCount: 1 }),
      ]),
    );
  });

  it("returns exact release identity and an explicitly unverified base", async () => {
    const response = await request("/api/games/heart-n-soul");
    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({
      game: {
        internalName: "pokemon/heart-n-soul",
        imageUrl: null,
        versions: [
          {
            outputSha256:
              "7c4f90c8b68b64d4639a37306d5302a11cd4d7c44005114ec2faa2b28b210c3d",
            requiredBase: {
              id: "pokemon-emerald-us",
              sha256: null,
              verified: false,
            },
          },
        ],
      },
    });
  });

  it("addresses every seeded row by ID through reads and management", async () => {
    for (const game of seedGames) {
      const read = await request(`/api/games/${game.id}`);
      expect(read.status).toBe(200);
      expect(await read.json()).toMatchObject({ game: { id: game.id } });

      const { id, ...input } = game;
      const write = await request(`/api/admin/games/${id}`, {
        method: "PUT",
        headers: adminHeaders(),
        body: JSON.stringify(input),
      });
      expect(write.status).toBe(200);
      expect(await write.json()).toMatchObject({ game: { id } });
    }

    for (const base of seedBases) {
      const read = await request(`/api/bases/${base.id}`);
      expect(read.status).toBe(200);
      expect(await read.json()).toMatchObject({ base: { id: base.id } });

      const { id, ...input } = base;
      const write = await request(`/api/admin/bases/${id}`, {
        method: "PUT",
        headers: adminHeaders(),
        body: JSON.stringify(input),
      });
      expect(write.status).toBe(200);
      expect(await write.json()).toMatchObject({ base: { id } });
    }

    for (const version of seedVersions) {
      const read = await request(`/api/versions/${version.id}`);
      expect(read.status).toBe(200);
      expect(await read.json()).toMatchObject({ version: { id: version.id } });

      const { id, ...input } = version;
      const write = await request(`/api/admin/versions/${id}`, {
        method: "PUT",
        headers: adminHeaders(),
        body: JSON.stringify(input),
      });
      expect(write.status).toBe(200);
      expect(await write.json()).toMatchObject({ version: { id } });
    }
  });

  it("rejects malformed dot-separated IDs", async () => {
    for (const id of [".version", "version.", "version..1", "Version.1"]) {
      const response = await request(`/api/versions/${id}`);
      expect(response.status).toBe(400);
    }
  });

  it("supports multiple versions of one game with base relationships", async () => {
    const response = await request("/api/admin/versions/heart-n-soul-next", {
      method: "PUT",
      headers: adminHeaders({ Origin: "https://zigba.test" }),
      body: JSON.stringify({
        gameId: "heart-n-soul",
        internalName: "test-second-version",
        displayName: "Test second version",
        outputSha256: "a".repeat(64),
        requiredBaseId: "pokemon-emerald-us",
        sourceRepositoryUrl: null,
        sourceRevision: null,
      }),
    });
    expect(response.status).toBe(200);

    const detail = (await (
      await request("/api/games/heart-n-soul")
    ).json()) as { game: { versions: Array<{ requiredBase: unknown }> } };
    expect(detail.game.versions).toHaveLength(2);
    expect(detail.game.versions.every((version) => version.requiredBase)).toBe(
      true,
    );
  });

  it("validates management JSON and referenced records", async () => {
    const invalidHash = await request("/api/admin/versions/invalid", {
      method: "PUT",
      headers: adminHeaders(),
      body: JSON.stringify({
        gameId: "heart-n-soul",
        internalName: "invalid",
        displayName: "Invalid",
        outputSha256: "not-a-hash",
        requiredBaseId: null,
        sourceRepositoryUrl: null,
        sourceRevision: null,
      }),
    });
    expect(invalidHash.status).toBe(400);

    const missingGame = await request("/api/admin/versions/orphan", {
      method: "PUT",
      headers: adminHeaders(),
      body: JSON.stringify({
        gameId: "does-not-exist",
        internalName: "orphan",
        displayName: "Orphan",
        outputSha256: "b".repeat(64),
        requiredBaseId: null,
        sourceRepositoryUrl: null,
        sourceRevision: null,
      }),
    });
    expect(missingGame.status).toBe(409);
  });
});
