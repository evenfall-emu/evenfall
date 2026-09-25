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

  it("returns exact release identity and its No-Intro base", async () => {
    const response = await request("/api/games/heart-n-soul");
    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({
      game: {
        internalName: "pokemon/heart-n-soul",
        imageUrl: null,
        versions: [
          {
            outputSha1: "a4be81f8330d7f223a3ab847c3da822fe6f8a2e0",
            requiredBase: {
              id: "pokemon-emerald-us",
              sha1: "f3ae088181bf583e55daf962a92bb46f4f1d07b7",
              crc32: "1f1c08fb",
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
        outputSha1: "a".repeat(40),
        requiredBaseId: "pokemon-emerald-us",
        sourceRepositoryUrl: null,
        sourceRevision: null,
        releasedAt: null,
        releaseNotes: null,
        patch: null,
        saveCompatibleFrom: [],
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
        outputSha1: "not-a-hash",
        requiredBaseId: null,
        sourceRepositoryUrl: null,
        sourceRevision: null,
        releasedAt: null,
        releaseNotes: null,
        patch: null,
        saveCompatibleFrom: [],
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
        outputSha1: "b".repeat(40),
        requiredBaseId: null,
        sourceRepositoryUrl: null,
        sourceRevision: null,
        releasedAt: null,
        releaseNotes: null,
        patch: null,
        saveCompatibleFrom: [],
      }),
    });
    expect(missingGame.status).toBe(409);
  });

  it("returns franchise, tags and cloud playthroughs with each game", async () => {
    const redux = seedVersions.find(
      (version) => version.gameId === "elite-redux",
    )!;
    for (const [userId, hash] of [
      ["alice", redux.outputSha1],
      ["bob", redux.outputSha1],
      ["bob", "c".repeat(40)],
    ])
      await database
        .prepare(
          "insert into playthrough (user_id, hash, id, name) values (?, ?, 'default', 'Run')",
        )
        .bind(userId, hash)
        .run();

    const listed = (await (await request("/api/games")).json()) as {
      games: Array<{
        id: string;
        franchise: string | null;
        tags: string[];
        cloudPlaythroughs: number;
      }>;
    };
    expect(
      Object.fromEntries(
        listed.games.map(({ id, franchise, tags, cloudPlaythroughs }) => [
          id,
          { franchise, tags, cloudPlaythroughs },
        ]),
      ),
    ).toEqual({
      "elite-redux": {
        franchise: "Pokémon",
        tags: ["Pokémon", "Difficulty Hack"],
        cloudPlaythroughs: 2,
      },
      "heart-n-soul": {
        franchise: "Pokémon",
        tags: ["Pokémon", "Remake"],
        cloudPlaythroughs: 0,
      },
    });
    expect(
      await (await request("/api/games/elite-redux")).json(),
    ).toMatchObject({ game: { cloudPlaythroughs: 2 } });
  });

  it("keeps tags in order, replaces them on write and rejects duplicates", async () => {
    const game = seedGames.find((item) => item.id === "heart-n-soul")!;
    const { id, ...input } = game;
    const write = (tags: string[]) =>
      request(`/api/admin/games/${id}`, {
        method: "PUT",
        headers: adminHeaders(),
        body: JSON.stringify({ ...input, tags }),
      });

    const reordered = await write(["Remake", "Johto", "Pokémon"]);
    expect(reordered.status).toBe(200);
    expect(await reordered.json()).toMatchObject({
      game: { tags: ["Remake", "Johto", "Pokémon"] },
    });
    expect((await write(["Remake", "remake"])).status).toBe(400);
    expect((await write([])).status).toBe(200);
    expect(await (await request(`/api/games/${id}`)).json()).toMatchObject({
      game: { tags: [] },
    });
    expect((await write(input.tags)).status).toBe(200);
  });

  it("returns release dates, notes, hosted patches and save compatibility", async () => {
    const previous = seedVersions.find(
      (version) => version.gameId === "elite-redux",
    )!;
    const next = {
      gameId: "elite-redux",
      internalName: "v2.7",
      displayName: "2.7",
      outputSha1: "d".repeat(40),
      requiredBaseId: "pokemon-emerald-us",
      sourceRepositoryUrl: null,
      sourceRevision: null,
      releasedAt: "2026-09-20",
      releaseNotes: "Rebalanced late-game trainers.",
      patch: {
        url: "https://patches.example/elite-redux-2.7.bps",
        format: "bps",
        size: 4_194_304,
      },
      saveCompatibleFrom: [previous.id],
    };
    const write = await request("/api/admin/versions/elite-redux-2.7", {
      method: "PUT",
      headers: adminHeaders(),
      body: JSON.stringify(next),
    });
    expect(write.status).toBe(200);

    const read = (await (
      await request("/api/versions/elite-redux-2.7")
    ).json()) as { version: Record<string, unknown> };
    expect(read.version).toMatchObject({
      releasedAt: "2026-09-20",
      releaseNotes: "Rebalanced late-game trainers.",
      patch: next.patch,
      saveCompatibleFrom: [previous.id],
    });
    for (const column of ["patchUrl", "patchFormat", "patchSize"])
      expect(read.version).not.toHaveProperty(column);
    expect(
      await (await request("/api/games/elite-redux")).json(),
    ).toMatchObject({
      game: {
        versions: expect.arrayContaining([
          expect.objectContaining({
            id: "elite-redux-2.7",
            saveCompatibleFrom: [previous.id],
          }),
          expect.objectContaining({
            id: previous.id,
            patch: null,
            saveCompatibleFrom: [],
          }),
        ]),
      },
    });

    // Removing the previous version removes it from the list that names it.
    const cleared = await request(`/api/admin/versions/elite-redux-2.7`, {
      method: "PUT",
      headers: adminHeaders(),
      body: JSON.stringify({ ...next, saveCompatibleFrom: [] }),
    });
    expect(await cleared.json()).toMatchObject({
      version: { saveCompatibleFrom: [] },
    });
  });

  it("enforces patch, date and save-compatibility rules", async () => {
    const valid = {
      gameId: "elite-redux",
      internalName: "rules",
      displayName: "Rules",
      outputSha1: "e".repeat(40),
      requiredBaseId: "pokemon-emerald-us",
      sourceRepositoryUrl: null,
      sourceRevision: null,
      releasedAt: null,
      releaseNotes: null,
      patch: null,
      saveCompatibleFrom: [],
    };
    const write = (changes: Record<string, unknown>) =>
      request("/api/admin/versions/elite-redux-rules", {
        method: "PUT",
        headers: adminHeaders(),
        body: JSON.stringify({ ...valid, ...changes }),
      });
    const patch = {
      url: "https://patches.example/rules.bps",
      format: "bps",
      size: 1,
    };

    expect((await write({ requiredBaseId: null, patch })).status).toBe(400);
    expect(
      (await write({ patch: { ...patch, format: "xdelta" } })).status,
    ).toBe(400);
    expect((await write({ patch: { ...patch, size: 0 } })).status).toBe(400);
    expect((await write({ releasedAt: "2026-9-20" })).status).toBe(400);
    expect(
      (await write({ saveCompatibleFrom: ["elite-redux-rules"] })).status,
    ).toBe(400);
    expect(
      (await write({ saveCompatibleFrom: ["heart-n-soul-1.2.1"] })).status,
    ).toBe(400);
    expect(
      (await write({ saveCompatibleFrom: ["elite-redux-missing"] })).status,
    ).toBe(409);
    expect((await write({ patch })).status).toBe(200);
  });
});
