// Turn the three local collection formats into one release inventory and the
// metadata-only seed. Neither the base dumps nor the patched ROMs are output.
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const defaultPacks = {
  pokemon: join(root, ".amp/in/rom-hacks"),
  goldenSun: join(root, ".amp/in/rom-hacks-golden-sun-fire-emblem/golden-sun"),
  fireEmblem: join(
    root,
    ".amp/in/rom-hacks-golden-sun-fire-emblem/fire-emblem",
  ),
};
const seedPath = join(root, "server/src/romhacks.json");
const json = (path) => JSON.parse(readFileSync(path, "utf8"));
const hooks = json(join(root, "tools/catalog/hooks.json"));
// These Golden Sun records point to the author's forum post, release page or
// repository. Other pageUrl values are third-party listings, not attribution.
const goldenSunAuthorPages = new Set([0, 1, 2, 3, 4, 6, 7, 10, 11, 12, 13]);
const sha1 = (path) =>
  createHash("sha1").update(readFileSync(path)).digest("hex");
const slug = (value) =>
  value
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");

/** Each patch-backed release has the same baseFile/baseSha1/patch/patchSha1/outputSha1 fields. */
export function normalizeCollections(
  packs = defaultPacks,
  basesDir = join(root, "bases"),
) {
  const bases = json(join(basesDir, "manifest.json")).bases;
  const byHash = new Map(bases.map((base) => [base.sha1, base]));
  const releases = [];

  function add(pack, family, entry, rom, facts) {
    const base = facts?.baseSha1 && byHash.get(facts.baseSha1);
    if (facts?.patch && (!base || base.crc32 !== facts.baseCrc32))
      throw new Error(`Unknown base for ${entry.name}: ${facts.baseSha1}`);
    const patch = facts?.patch ?? null;
    const patchSha1 = facts?.patchSha1 ?? null;
    if (patch) {
      if (
        !facts.verified ||
        !patchSha1 ||
        !["bps", "ups", "ips"].includes(patch.split(".").at(-1))
      )
        throw new Error(`Unverified or unsupported patch for ${entry.name}`);
      const file = join(pack, patch);
      if (
        sha1(file) !== patchSha1 ||
        (facts.patchBytes && readFileSync(file).length !== facts.patchBytes)
      )
        throw new Error(`Patch bytes do not match the inventory: ${file}`);
    }
    if (!/^[0-9a-f]{40}$/.test(rom.sha1))
      throw new Error(`Missing output SHA-1 for ${entry.name}`);
    releases.push({
      collection: pack,
      family,
      name: entry.name,
      // The author labels these selected FE patches differently from the
      // collection's earlier research labels (see its attribution evidence).
      version:
        family === "Fire Emblem" && entry.index === 0
          ? "2.1"
          : family === "Fire Emblem" && entry.index === 12
            ? "Vanilla patch (2021-11-04)"
            : entry.version,
      description: entry.description,
      sourceUrl:
        family === "Fire Emblem"
          ? entry.attribution?.status === "confirmed"
            ? entry.attribution.releasePostUrl
            : null
          : family === "Golden Sun" && goldenSunAuthorPages.has(entry.index)
            ? entry.pageUrl
            : null,
      variant: rom.variant ?? null,
      gameSlug:
        family === "Pokémon" || family === "Fire Emblem"
          ? basename(rom.file, ".gba").replace(/^\d+-/, "")
          : `${slug(entry.name)}${entry.roms.length > 1 ? `-${rom.variant}` : ""}`,
      splitGame: family === "Golden Sun" && entry.roms.length > 1,
      baseFile: base ? join(basesDir, base.file) : null,
      baseSha1: base?.sha1 ?? null,
      baseCrc32: base?.crc32 ?? null,
      patch: patch ? join(pack, patch) : null,
      patchSha1,
      patchBytes: patch ? readFileSync(join(pack, patch)).length : null,
      outputSha1: rom.sha1,
      outputCrc32: rom.crc32 ?? null,
      verified: Boolean(patch),
    });
  }

  {
    const pack = packs.pokemon;
    const patches = new Map(
      json(join(pack, "ups/patches.json")).map((p) => [p.index, p]),
    );
    const official = json(join(pack, "official-patches.json"));
    for (const entry of json(join(pack, "catalog.json")).entries) {
      const fact = patches.get(entry.index);
      if (
        Boolean(fact) !== Boolean(entry.patch) ||
        (fact && fact.romSha1 !== entry.sha1)
      )
        throw new Error(`Pokémon release and patch disagree: ${entry.name}`);
      // Prefer the author's own patch when its round trip produced this release.
      const own = official[entry.index];
      if (
        own &&
        (!own.verified ||
          own.romSha1 !== entry.sha1 ||
          own.baseSha1 !== fact.baseSha1)
      )
        throw new Error(`Official patch disagrees: ${entry.name}`);
      add(
        pack,
        "Pokémon",
        entry,
        { file: entry.gameFile, sha1: entry.sha1, crc32: entry.crc32 },
        own
          ? {
              ...fact,
              patch: own.patch,
              patchSha1: sha1(join(pack, own.patch)),
              patchBytes: null,
            }
          : fact,
      );
    }
  }
  {
    const pack = packs.goldenSun;
    for (const entry of json(join(pack, "catalog.json")).entries)
      for (const rom of entry.roms) {
        add(
          pack,
          "Golden Sun",
          entry,
          {
            file: rom.gameFile,
            sha1: rom.sha1,
            crc32: rom.crc32,
            variant: rom.variant,
          },
          rom.patch
            ? {
                baseSha1: rom.baseSha1,
                baseCrc32: rom.baseCrc32,
                patch: rom.patch,
                patchSha1: rom.patchSha1,
                verified: true,
              }
            : null,
        );
      }
  }
  {
    const pack = packs.fireEmblem;
    const patched = json(join(pack, "patched.json"));
    for (const entry of json(join(pack, "catalog.json")).entries) {
      if (
        !entry.attribution ||
        !["confirmed", "ambiguous"].includes(entry.attribution.status)
      )
        throw new Error(`Missing release attribution audit: ${entry.name}`);
      const fact = patched[entry.index];
      if (
        !fact ||
        !fact.verified ||
        fact.outputSha1 !== entry.rom?.sha1 ||
        fact.patchSha1 !== entry.patch?.sha1 ||
        fact.patch !== entry.patch?.file
      )
        throw new Error(
          `Fire Emblem release and patch disagree: ${entry.name}`,
        );
      add(
        pack,
        "Fire Emblem",
        entry,
        { file: entry.rom.file, sha1: entry.rom.sha1, crc32: entry.rom.crc32 },
        fact,
      );
    }
  }
  return { bases, releases };
}

export function buildSeed({ bases, releases }) {
  const usedBases = new Set(
    releases.map((release) => release.baseSha1).filter(Boolean),
  );
  const baseId = (base) => slug(base.noIntroName);
  const baseRows = bases
    .filter((base) => usedBases.has(base.sha1))
    .map((base) => ({
      id:
        base.sha1 === "f3ae088181bf583e55daf962a92bb46f4f1d07b7"
          ? "pokemon-emerald-us"
          : baseId(base),
      internalName: base.noIntroName,
      displayName: base.noIntroName,
      sha1: base.sha1,
      crc32: base.crc32,
    }));
  const byHash = new Map(baseRows.map((base) => [base.sha1, base.id]));
  const games = new Map();
  const versions = [];
  const patches = new Map();
  const outputs = new Set();
  for (const release of releases) {
    const id =
      release.family === "Pokémon" && release.gameSlug === "elite-redux"
        ? "elite-redux"
        : `${slug(release.family)}-${release.gameSlug}`;
    const hook = hooks[id];
    if (!hook || hook.length > 120 || hook === release.description)
      throw new Error(`Missing distinct, concise hook for ${id}`);
    if (!games.has(id))
      games.set(id, {
        id,
        internalName: `${slug(release.family)}/${release.gameSlug}`,
        displayName: release.splitGame
          ? `${release.name} (${release.variant === "tla" ? "The Lost Age" : "Golden Sun"})`
          : release.name,
        summary: hook,
        description: release.description,
        imageUrl: null,
        franchise: release.family,
        tags:
          id === "elite-redux"
            ? ["Pokémon", "Difficulty Hack"]
            : [release.family],
      });
    if (outputs.has(release.outputSha1))
      throw new Error(`Two catalog releases have output ${release.outputSha1}`);
    outputs.add(release.outputSha1);
    const versionId =
      id === "elite-redux"
        ? "elite-redux-2.65-beta2"
        : `${id}-${release.outputSha1.slice(0, 12)}`;
    const patch = release.patch && {
      url: `https://patches.evenfall-emu.org/${release.patchSha1}.${release.patch.split(".").at(-1)}`,
      format: release.patch.split(".").at(-1),
      size: release.patchBytes,
    };
    versions.push({
      id: versionId,
      gameId: id,
      internalName:
        id === "elite-redux"
          ? "v2.65-beta2-1048"
          : release.variant && !release.splitGame
            ? `${release.version} (${release.variant})`
            : release.version,
      displayName:
        id === "elite-redux"
          ? "2.65 Beta2 (1048)"
          : release.variant && !release.splitGame
            ? `${release.version} (${release.variant})`
            : release.version,
      outputSha1: release.outputSha1,
      requiredBaseId: release.baseSha1 ? byHash.get(release.baseSha1) : null,
      sourceRepositoryUrl:
        id === "elite-redux"
          ? "https://github.com/Elite-Redux/eliteredux-source"
          : (release.sourceUrl ?? null),
      sourceRevision:
        id === "elite-redux"
          ? "06da575972cfed20d471f8bb5b90ea9e4736b16a"
          : null,
      releasedAt: null,
      releaseNotes: null,
      patch: patch || null,
      saveCompatibleFrom: [],
    });
    if (patch) {
      const key = patch.url.split("/").at(-1);
      if (patches.has(key) && sha1(patches.get(key)) !== release.patchSha1)
        throw new Error(`Conflicting CDN key ${key}`);
      patches.set(key, release.patch);
    }
  }
  return { games: [...games.values()], bases: baseRows, versions, patches };
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const command = process.argv[2];
  if (!["build", "check", "upload", "verify"].includes(command)) {
    console.error(
      "Usage: node tools/catalog/romhacks.mjs build|check|upload|verify",
    );
    process.exitCode = 2;
  } else {
    const inventory = normalizeCollections();
    const { games, bases, versions, patches } = buildSeed(inventory);
    const data = `${JSON.stringify({ games, bases, versions }, null, 2)}\n`;
    if (command === "build") {
      writeFileSync(seedPath, data);
      for (const pack of Object.values(defaultPacks))
        writeFileSync(
          join(pack, "normalized.json"),
          `${JSON.stringify(
            {
              entries: inventory.releases.filter(
                (release) => release.collection === pack,
              ),
            },
            null,
            2,
          )}\n`,
        );
    } else if (readFileSync(seedPath, "utf8") !== data)
      throw new Error(
        "Catalog seed is stale; run build before publishing patches",
      );
    if (command === "upload" || command === "verify") {
      const { execFileSync } = await import("node:child_process");
      for (const [key, file] of patches) {
        const url = `https://patches.evenfall-emu.org/${key}`;
        let response = await fetch(url, {
          headers: { Origin: "https://play.evenfall-emu.org" },
        });
        if (response.status === 404 && command === "upload") {
          execFileSync(
            "npx",
            [
              "wrangler",
              "r2",
              "object",
              "put",
              `evenfall-patches/${key}`,
              "--file",
              file,
              "--content-type",
              "application/octet-stream",
              "--cache-control",
              "public, max-age=31536000, immutable",
              "--jurisdiction",
              "eu",
              "--remote",
            ],
            { cwd: root, stdio: "inherit" },
          );
          response = await fetch(url, {
            headers: { Origin: "https://play.evenfall-emu.org" },
          });
        }
        if (
          !response.ok ||
          response.headers.get("access-control-allow-origin") !==
            "https://play.evenfall-emu.org"
        )
          throw new Error(
            `CDN GET or browser CORS failed: ${url} (HTTP ${response.status})`,
          );
        const digest = createHash("sha1");
        let size = 0;
        for await (const chunk of response.body) {
          digest.update(chunk);
          size += chunk.length;
        }
        if (
          digest.digest("hex") !== key.split(".")[0] ||
          size !== readFileSync(file).length
        )
          throw new Error(`CDN bytes do not match the catalog: ${url}`);
      }
    }
    console.log(
      `${games.length} games, ${bases.length} bases, ${versions.length} releases, ${patches.size} patches`,
    );
  }
}
