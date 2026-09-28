import { readFile, readdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { convertV4MiniflareOptions, Miniflare } from "miniflare";
import { expect, it } from "vite-plus/test";

it("adds visibility to a populated catalog without replacing its versions or relationships", async () => {
  const miniflare = new Miniflare(
    convertV4MiniflareOptions({
      modules: true,
      script: "export default { fetch() { return new Response('test') } }",
      d1Databases: ["DB"],
    }),
  );
  try {
    const db = await miniflare.getD1Database("DB");
    const directory = fileURLToPath(new URL("../migrations", import.meta.url));
    const files = (await readdir(directory))
      .filter((name) => name.endsWith(".sql"))
      .sort();
    const apply = async (file: string) => {
      const migration = await readFile(`${directory}/${file}`, "utf8");
      for (const statement of migration.split("--> statement-breakpoint"))
        if (statement.trim()) await db.prepare(statement).run();
    };
    const visibilityMigration = "0007_needy_the_liberteens.sql";
    for (const file of files.filter((name) => name < visibilityMigration))
      await apply(file);
    await db
      .prepare(
        "insert into games (id, internal_name, display_name) values ('game', 'game', 'Game')",
      )
      .run();
    await db
      .prepare(
        "insert into versions (id, game_id, internal_name, display_name, output_sha1) values ('old', 'game', 'old', 'Old', ?)",
      )
      .bind("a".repeat(40))
      .run();
    await db
      .prepare(
        "insert into versions (id, game_id, internal_name, display_name, output_sha1) values ('new', 'game', 'new', 'New', ?)",
      )
      .bind("b".repeat(40))
      .run();
    await db
      .prepare(
        "insert into version_save_compatibility (version_id, from_version_id) values ('new', 'old')",
      )
      .run();

    await apply(visibilityMigration);
    expect(
      await db.prepare("select id, visibility from versions order by id").all(),
    ).toMatchObject({
      results: [
        { id: "new", visibility: "experimental" },
        { id: "old", visibility: "experimental" },
      ],
    });
    expect(
      await db
        .prepare(
          "select version_id, from_version_id from version_save_compatibility",
        )
        .all(),
    ).toMatchObject({
      results: [{ version_id: "new", from_version_id: "old" }],
    });
    await db
      .prepare("update versions set visibility = 'hidden' where id = 'old'")
      .run();
    await expect(
      db
        .prepare(
          "update versions set visibility = 'unreviewed' where id = 'old'",
        )
        .run(),
    ).rejects.toThrow();
    await expect(
      db
        .prepare(
          "insert into version_save_compatibility (version_id, from_version_id) values ('new', 'missing')",
        )
        .run(),
    ).rejects.toThrow();
  } finally {
    await miniflare.dispose();
  }
});
