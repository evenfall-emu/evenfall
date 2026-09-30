import { readFile, readdir } from "node:fs/promises";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { convertV4MiniflareOptions, Miniflare } from "miniflare";
import { afterAll, beforeAll, describe, expect, it } from "vite-plus/test";
import app, { type Bindings } from "../src/index";
import { seedBases, seedGames, seedVersions } from "../src/seed";
import { baseInput, gameInput, resourceId, versionInput } from "../src/types";

const adminSecret = "test-only-secret";
let miniflare: Miniflare;
let database: D1Database;
let patches: R2Bucket;

beforeAll(async () => {
  miniflare = new Miniflare(
    convertV4MiniflareOptions({
      modules: true,
      script: "export default { fetch() { return new Response('test') } }",
      d1Databases: ["DB"],
      r2Buckets: ["PATCHES"],
    }),
  );
  database = await miniflare.getD1Database("DB");
  // Miniflare proxies use Node's Headers types rather than workerd's.
  patches = (await miniflare.getR2Bucket("PATCHES")) as unknown as R2Bucket;
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
    expect(body.games).toHaveLength(seedGames.length);
    expect(body.games).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ id: "elite-redux", versionCount: 1 }),
      ]),
    );
    expect(
      body.games.reduce((total, game) => total + game.versionCount, 0),
    ).toBe(seedVersions.length);
  }, 60_000);

  it("returns exact release identity and its No-Intro base", async () => {
    const response = await request("/api/games/elite-redux");
    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({
      game: {
        internalName: "pokemon/elite-redux",
        imageUrl: null,
        versions: [
          {
            outputSha1: "f1323126bc043f3ce931bcc670957930559eb515",
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

  it("serves concise hooks, full descriptions and verified creator pages separately", async () => {
    const response = await request("/api/games");
    const { games } = (await response.json()) as {
      games: Array<{
        id: string;
        summary: string | null;
        description: string | null;
      }>;
    };
    const unbound = games.find((game) => game.id === "pokemon-unbound")!;
    expect(unbound.summary).toBe(
      "Explore Borrius in a story-rich adventure built for your own pace.",
    );
    expect(unbound.description).toContain("extensive side content");
    expect(unbound.summary).not.toBe(unbound.description);

    const fireEmblem = await request("/api/games/fire-emblem-vision-quest");
    expect(await fireEmblem.json()).toMatchObject({
      game: {
        versions: [
          expect.objectContaining({
            sourceRepositoryUrl:
              "https://feuniverse.us/t/fe8-complete-fire-emblem-vision-quest-v3-by-pushwall-1-oct-22/3815/1",
          }),
        ],
      },
    });
    const ambiguous = await request("/api/games/fire-emblem-elibean-nights");
    expect(await ambiguous.json()).toMatchObject({
      game: {
        versions: [expect.objectContaining({ sourceRepositoryUrl: null })],
      },
    });
    const lastPromise = await request(
      "/api/games/fire-emblem-the-last-promise",
    );
    expect(await lastPromise.json()).toMatchObject({
      game: {
        versions: [
          expect.objectContaining({
            displayName: "2.1",
            sourceRepositoryUrl:
              "https://www.feshrine.net/thelastpromise/download/",
          }),
        ],
      },
    });
    const pokemon = await request("/api/games/pokemon-unbound");
    expect(await pokemon.json()).toMatchObject({
      game: {
        versions: [expect.objectContaining({ sourceRepositoryUrl: null })],
      },
    });
  });

  it("offers both Golden Sun games on their own bases and omits patchless releases", async () => {
    for (const [variant, outputSha1, baseSha1, size] of [
      [
        "gs1",
        "448e73f566679fa87980d0eb46b32f395fbc231d",
        "5c4695205413df7db52b9a184815a07783999971",
        58352,
      ],
      [
        "tla",
        "e045d22fabbed374dd0e1a73f7a1f8d2c5cdba40",
        "b500663220cb9bf56b9f8e8c0c544f1d6fa3a824",
        207091,
      ],
    ] as const) {
      const response = await request(
        `/api/games/golden-sun-golden-sun-reloaded-${variant}`,
      );
      expect(response.status).toBe(200);
      expect(await response.json()).toMatchObject({
        game: {
          versions: [
            expect.objectContaining({
              outputSha1,
              requiredBase: expect.objectContaining({ sha1: baseSha1 }),
              patch: expect.objectContaining({
                url: expect.stringMatching(
                  /^https:\/\/patches\.evenfall-emu\.org\//,
                ),
                format: "ips",
                size,
              }),
            }),
          ],
        },
      });
    }

    const patchless = await request("/api/games/pokemon-light-platinum");
    expect(patchless.status).toBe(404);
    expect((await request("/api/games/heart-n-soul")).status).toBe(404);
  });

  it("keeps hidden and unpatched releases out of every public catalog read", async () => {
    const source = seedVersions.find(
      (version) => version.gameId === "fire-emblem-vision-quest",
    )!;
    const { id: _id, ...input } = source;
    const put = (id: string, changes: Record<string, unknown>) =>
      request(`/api/admin/versions/${id}`, {
        method: "PUT",
        headers: adminHeaders(),
        body: JSON.stringify({
          ...input,
          internalName: id,
          outputSha1:
            id === "hidden-test"
              ? "1".repeat(40)
              : id === "unpatched-test"
                ? "2".repeat(40)
                : "3".repeat(40),
          ...changes,
        }),
      });
    const hidden = await put("hidden-test", { visibility: "hidden" });
    expect(hidden.status).toBe(200);
    expect(await hidden.json()).toMatchObject({
      version: { visibility: "hidden" },
    });
    expect(
      (await put("unpatched-test", { patch: null, visibility: "normal" }))
        .status,
    ).toBe(200);
    expect(
      (
        await put("featured-test", {
          visibility: "featured",
          saveCompatibleFrom: ["hidden-test"],
        })
      ).status,
    ).toBe(200);
    for (const id of ["hidden-test", "unpatched-test"])
      expect((await request(`/api/versions/${id}`)).status).toBe(404);

    const versions = (await (await request("/api/versions")).json()) as {
      versions: Array<{
        id: string;
        visibility: string;
        saveCompatibleFrom: string[];
      }>;
    };
    expect(versions.versions.map((version) => version.id)).toContain(
      "featured-test",
    );
    expect(versions.versions.map((version) => version.id)).toContain(
      "elite-redux-2.65-beta2",
    ); // Experimental remains available for opt-in.
    expect(versions.versions.map((version) => version.id)).not.toContain(
      "hidden-test",
    );
    expect(versions.versions.map((version) => version.id)).not.toContain(
      "unpatched-test",
    );
    expect(
      versions.versions.find((version) => version.id === "featured-test")
        ?.saveCompatibleFrom,
    ).toEqual([]);
    const detail = (await (
      await request("/api/games/fire-emblem-vision-quest")
    ).json()) as {
      game: { versions: Array<{ id: string; saveCompatibleFrom: string[] }> };
    };
    expect(
      detail.game.versions.find((version) => version.id === "featured-test")
        ?.saveCompatibleFrom,
    ).toEqual([]);
    expect(detail.game.versions.map((version) => version.id)).not.toContain(
      "hidden-test",
    );
    const listed = (await (await request("/api/games")).json()) as {
      games: Array<{ id: string; versionCount: number }>;
    };
    expect(
      listed.games.find((game) => game.id === "fire-emblem-vision-quest")
        ?.versionCount,
    ).toBe(2);

    const { id: _gameId, ...gameInput } = seedGames[0];
    expect(
      (
        await request("/api/admin/games/hidden-game-test", {
          method: "PUT",
          headers: adminHeaders(),
          body: JSON.stringify({
            ...gameInput,
            internalName: "hidden/game/test",
          }),
        })
      ).status,
    ).toBe(200);
    expect(
      (
        await put("hidden-only-test", {
          gameId: "hidden-game-test",
          visibility: "hidden",
          outputSha1: "4".repeat(40),
        })
      ).status,
    ).toBe(200);
    expect((await request("/api/games/hidden-game-test")).status).toBe(404);
    const after = (await (await request("/api/games")).json()) as {
      games: Array<{ id: string }>;
    };
    expect(after.games.map((game) => game.id)).not.toContain(
      "hidden-game-test",
    );
  });

  it("accepts every seed's ID and management input with valid relationships", () => {
    // Seeding exercises database constraints, but bypasses the HTTP schemas.
    // Validate all rows here rather than repeating every database write via HTTP.
    for (const [rows, schema] of [
      [seedGames, gameInput],
      [seedBases, baseInput],
      [seedVersions, versionInput],
    ] as const) {
      expect(new Set(rows.map(({ id }) => id)).size).toBe(rows.length);
      for (const { id, ...input } of rows) {
        expect(resourceId.parse(id), id).toBe(id);
        expect(schema.parse(input), id).toEqual(input);
      }
    }
    const games = new Set(seedGames.map(({ id }) => id));
    const bases = new Set(seedBases.map(({ id }) => id));
    const versions = new Map(
      seedVersions.map((version) => [version.id, version]),
    );
    for (const version of seedVersions) {
      expect(games.has(version.gameId), version.id).toBe(true);
      if (version.requiredBaseId !== null)
        expect(bases.has(version.requiredBaseId), version.id).toBe(true);
      for (const previous of version.saveCompatibleFrom) {
        expect(previous, version.id).not.toBe(version.id);
        expect(versions.get(previous)?.gameId, version.id).toBe(version.gameId);
      }
    }
  });

  it("addresses representative seeded IDs through management and public readback", async () => {
    // Slug IDs and dotted release IDs use the same validator on all routes.
    // Other tests cover compatibility edges, hidden rows and missing references.
    const representativeVersions = [
      "elite-redux-2.65-beta2",
      "fire-emblem-vision-quest-16d772195bd3",
    ].map((id) => {
      const version = seedVersions.find((row) => row.id === id);
      expect(version, id).toBeDefined();
      return version!;
    });
    const games = seedGames.filter((game) =>
      representativeVersions.some((version) => version.gameId === game.id),
    );
    const bases = seedBases.filter((base) =>
      representativeVersions.some(
        (version) => version.requiredBaseId === base.id,
      ),
    );
    expect(games).toHaveLength(2);
    expect(bases).toHaveLength(2);
    for (const game of games) {
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
      expect(await write.json()).toMatchObject({ game });
      expect(await (await request(`/api/games/${id}`)).json()).toMatchObject({
        game,
      });
    }

    for (const base of bases) {
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
      expect(await write.json()).toMatchObject({ base });
      expect(await (await request(`/api/bases/${id}`)).json()).toMatchObject({
        base,
      });
    }

    for (const version of representativeVersions) {
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
      const expected = {
        ...version,
        requiredBase: seedBases.find(
          (base) => base.id === version.requiredBaseId,
        ),
      };
      expect(await write.json()).toMatchObject({ version: expected });
      expect(await (await request(`/api/versions/${id}`)).json()).toMatchObject(
        {
          version: expected,
        },
      );
    }
  });

  it("rejects malformed dot-separated IDs", async () => {
    for (const id of [".version", "version.", "version..1", "Version.1"]) {
      const response = await request(`/api/versions/${id}`);
      expect(response.status).toBe(400);
    }
  });

  it("supports multiple versions of one game with base relationships", async () => {
    const response = await request("/api/admin/versions/elite-redux-next", {
      method: "PUT",
      headers: adminHeaders({ Origin: "https://zigba.test" }),
      body: JSON.stringify({
        gameId: "elite-redux",
        internalName: "test-second-version",
        displayName: "Test second version",
        outputSha1: "a".repeat(40),
        requiredBaseId: "pokemon-emerald-us",
        sourceRepositoryUrl: null,
        sourceRevision: null,
        releasedAt: null,
        releaseNotes: null,
        patch: {
          url: "https://patches.example/next.ups",
          format: "ups",
          size: 42,
        },
        saveCompatibleFrom: [],
      }),
    });
    expect(response.status).toBe(200);

    const detail = (await (await request("/api/games/elite-redux")).json()) as {
      game: { versions: Array<{ requiredBase: unknown }> };
    };
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
        gameId: "elite-redux",
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
    expect(listed.games).toHaveLength(seedGames.length);
    expect(listed.games.find(({ id }) => id === "elite-redux")).toMatchObject({
      franchise: "Pokémon",
      tags: ["Pokémon", "Difficulty Hack"],
      cloudPlaythroughs: 2,
    });
    expect(
      await (await request("/api/games/elite-redux")).json(),
    ).toMatchObject({ game: { cloudPlaythroughs: 2 } });
  });

  it("keeps tags in order, replaces them on write and rejects duplicates", async () => {
    const game = seedGames.find((item) => item.id === "elite-redux")!;
    const { id, ...input } = game;
    const write = (tags: string[]) =>
      request(`/api/admin/games/${id}`, {
        method: "PUT",
        headers: adminHeaders(),
        body: JSON.stringify({ ...input, tags }),
      });

    const reordered = await write(["Difficulty Hack", "Johto", "Pokémon"]);
    expect(reordered.status).toBe(200);
    expect(await reordered.json()).toMatchObject({
      game: { tags: ["Difficulty Hack", "Johto", "Pokémon"] },
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
            patch: previous.patch,
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
      (
        await write({
          saveCompatibleFrom: ["fire-emblem-vision-quest-16d772195bd3"],
        })
      ).status,
    ).toBe(400);
    expect(
      (await write({ saveCompatibleFrom: ["elite-redux-missing"] })).status,
    ).toBe(409);
    expect((await write({ patch })).status).toBe(200);
  });

  it("lists drafts privately, edits selected fields, and uploads a content-addressed patch", async () => {
    const id = "admin-upload-test";
    const { id: _id, ...seed } = seedVersions.find(
      (item) => item.gameId === "elite-redux",
    )!;
    const create = await request(`/api/admin/versions/${id}`, {
      method: "PUT",
      headers: adminHeaders(),
      body: JSON.stringify({
        ...seed,
        internalName: id,
        outputSha1: "9".repeat(40),
        patch: null,
        visibility: "hidden",
      }),
    });
    expect(create.status).toBe(200);
    expect((await request(`/api/versions/${id}`)).status).toBe(404);
    expect((await request(`/api/admin/versions/${id}`)).status).toBe(401);
    const privateRead = await request(`/api/admin/versions/${id}`, {
      headers: adminHeaders(),
    });
    expect(privateRead.headers.get("Cache-Control")).toBe("no-store");
    expect(await privateRead.json()).toMatchObject({
      version: { patch: null, visibility: "hidden" },
    });
    const list = await request("/api/admin/versions", {
      headers: adminHeaders(),
    });
    expect(
      ((await list.json()) as { versions: Array<{ id: string }> }).versions.map(
        (item) => item.id,
      ),
    ).toContain(id);
    const games = await request("/api/admin/games", {
      headers: adminHeaders(),
    });
    expect(
      ((await games.json()) as { games: Array<{ id: string }> }).games.map(
        (item) => item.id,
      ),
    ).toContain("hidden-game-test");

    const edit = await request(`/api/admin/versions/${id}`, {
      method: "PATCH",
      headers: adminHeaders(),
      body: JSON.stringify({ releaseNotes: "Reviewed notes" }),
    });
    expect(edit.status).toBe(200);
    expect(await edit.json()).toMatchObject({
      version: {
        releaseNotes: "Reviewed notes",
        visibility: "hidden",
        requiredBaseId: seed.requiredBaseId,
        patch: null,
      },
    });
    const rejected = await request(`/api/admin/versions/${id}`, {
      method: "PATCH",
      headers: adminHeaders(),
      body: JSON.stringify({ requiredBaseId: null, patch: seed.patch }),
    });
    expect(rejected.status).toBe(400);
    const gameEdit = await request("/api/admin/games/elite-redux", {
      method: "PATCH",
      headers: adminHeaders(),
      body: JSON.stringify({ description: "Updated description" }),
    });
    expect(gameEdit.status).toBe(200);
    expect(await gameEdit.json()).toMatchObject({
      game: {
        description: "Updated description",
        tags: ["Pokémon", "Difficulty Hack"],
      },
    });

    const bytes = new Uint8Array([
      80, 65, 84, 67, 72, 0, 0, 1, 0, 1, 90, 69, 79, 70,
    ]); // PATCH, one byte at offset 1, EOF
    const hash = createHash("sha1").update(bytes).digest("hex");
    const bindings: Bindings = {
      DB: database,
      ADMIN_SECRET: adminSecret,
      PATCHES: patches,
      PATCHES_URL: "https://patches.test",
    };
    const upload = (body: Uint8Array, format = "ips") =>
      request(
        `/api/admin/versions/${id}/patch`,
        {
          method: "PUT",
          headers: adminHeaders({
            "Content-Type": "application/octet-stream",
            "X-Patch-Format": format,
          }),
          body: new Uint8Array(body),
        },
        bindings,
      );
    expect(
      (
        await request(`/api/admin/versions/${id}/patch`, {
          method: "PUT",
          headers: adminHeaders({
            "Content-Type": "application/octet-stream",
            "X-Patch-Format": "ips",
          }),
          body: new Uint8Array(bytes),
        })
      ).status,
    ).toBe(503);
    expect((await upload(bytes, "bps")).status).toBe(400);
    expect(await patches.head(`${hash}.bps`)).toBeNull();
    const uploaded = await upload(bytes);
    expect(uploaded.status).toBe(200);
    expect(await uploaded.json()).toMatchObject({
      version: {
        patch: {
          url: `https://patches.test/${hash}.ips`,
          format: "ips",
          size: bytes.length,
        },
        visibility: "hidden",
        releaseNotes: "Reviewed notes",
      },
    });
    expect(
      new Uint8Array(await (await patches.get(`${hash}.ips`))!.arrayBuffer()),
    ).toEqual(bytes);
    expect((await upload(bytes)).status).toBe(200);
    expect((await request(`/api/versions/${id}`)).status).toBe(404);
    const visible = await request(`/api/admin/versions/${id}`, {
      method: "PATCH",
      headers: adminHeaders(),
      body: JSON.stringify({ visibility: "experimental" }),
    });
    expect(visible.status).toBe(200);
    expect(await (await request(`/api/versions/${id}`)).json()).toMatchObject({
      version: { patch: { url: `https://patches.test/${hash}.ips` } },
    });
  });
});
