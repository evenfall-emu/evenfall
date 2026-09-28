import { describe, expect, it } from "vitest";
import { previewCatalog, previewDevice } from "./library-fixtures";
import type { LibraryDevice } from "../src/pages/library/catalog";
import { formatAgo, formatMoment } from "../src/pages/library/format";
import {
  buildLibrary,
  filterGames,
  franchisesOf,
  noFilters,
  saveCompatible,
  tagsOf,
  type LibraryGame,
} from "../src/pages/library/model";

const now = new Date(2026, 8, 25, 16, 0).getTime();
const device = previewDevice(now);
const games = buildLibrary(previewCatalog, device);
const game = (id: string) => games.find((item) => item.id === id)!;
const sha = (byte: number) => byte.toString(16).padStart(2, "0").repeat(20);

describe("buildLibrary", () => {
  it("keeps a short hook separate from the optional full description", () => {
    expect(game("vision-quest")).toMatchObject({
      summary:
        "A complete new campaign built on Fire Emblem, with its own cast, maps and story.",
      description: expect.stringContaining("Storch's band"),
    });
    expect(game("elite-redux").description).toBeNull();
    expect(game(sha(0xe0)).description).toBeNull();
  });

  it("derives availability from ROMs and base ROMs on this device", () => {
    expect(
      Object.fromEntries(games.map((item) => [item.id, item.availability])),
    ).toEqual({
      "elite-redux": "installed",
      "emerald-seaglass": "installed",
      "vision-quest": "ready",
      "golden-sun-reloaded": "needs-file",
      unbound: "ready",
      // Base ROMs the catalog lists only as bases.
      [sha(0xe0)]: "installed",
      [sha(0xf0)]: "installed",
      [sha(0xfe)]: "installed",
    });
    expect(game("golden-sun-reloaded").missingFile).toBe(
      "Golden Sun: The Lost Age (US)",
    );
  });

  it("accepts any release ROM, and a base only when a patch needs it", () => {
    expect(game("golden-sun-reloaded").wantedFiles).toEqual([
      { sha1: expect.any(String), title: "Golden Sun: Reloaded" },
      { sha1: sha(0x65), title: "Golden Sun: The Lost Age (US)" },
    ]);
    expect(game("elite-redux").wantedFiles.map((file) => file.sha1)).toEqual(
      game("elite-redux").releases.map((item) => item.hash),
    );
  });

  it("lists ROMs and cloud copies the catalog does not know", () => {
    const homebrew = sha(0x42);
    const library = buildLibrary(previewCatalog, {
      ...device,
      cloud: [
        ...device.cloud,
        {
          ...device.cloud[0],
          hash: homebrew,
          title: "Homebrew",
          playthroughId: "demo",
          playthroughName: "Demo run",
        },
      ],
    });
    const emerald = library.find((item) => item.id === sha(0xe0))!;
    // A base keeps the catalog's name for it, not the stored file name.
    expect(emerald).toMatchObject({
      title: "Pokémon Emerald (US)",
      availability: "installed",
      latest: { hash: sha(0xe0), label: "", installed: true },
      playthroughs: [],
    });
    const cloudOnly = library.find((item) => item.id === homebrew)!;
    expect(cloudOnly).toMatchObject({
      title: "Homebrew",
      availability: "needs-file",
      missingFile: "the Homebrew ROM",
      wantedFiles: [{ sha1: homebrew, title: "Homebrew" }],
    });
    expect(cloudOnly.playthroughs.map((run) => [run.name, run.sync])).toEqual([
      ["Demo run", "cloud-only"],
    ]);
  });

  it("titles other unlisted ROMs by their No-Intro dump name", () => {
    const emerald = "Pokemon - Emerald Version (USA, Europe)";
    const dumps = {
      [sha(0xe0)]: emerald,
      [sha(0x25)]: "A dump name for a catalog release",
      toString: "An inherited key",
    };
    const title = (library: LibraryGame[], id: string) =>
      library.find((item) => item.id === id)?.title;
    const named = buildLibrary(previewCatalog, device, dumps);
    expect(title(named, sha(0xe0))).toBe("Pokémon Emerald (US)");
    expect(title(named, "elite-redux")).toBe("Elite Redux");
    const unlisted = buildLibrary({ games: [], versions: [] }, device, dumps);
    expect(title(unlisted, sha(0xe0))).toBe(emerald);
    expect(title(unlisted, sha(0x25))).toBe(
      "A dump name for a catalog release",
    );
    expect(title(unlisted, sha(0xf0))).toBe("Pokemon FireRed");
  });

  it("orders releases by date, not by display name", () => {
    const redux = game("elite-redux");
    expect(redux.releases.map((item) => item.label)).toEqual([
      "v2.6",
      "v2.5",
      "v2.4",
    ]);
    expect(redux.latest.installed).toBe(false);
    expect(redux.releases[1].installed).toBe(true);
  });

  it("orders releases the catalog has not dated after the dated ones", () => {
    const [newest] = previewCatalog.versions.filter(
      (version) => version.gameId === "elite-redux",
    );
    const undated = {
      ...newest,
      id: "elite-redux-nightly",
      displayName: "nightly",
      outputSha1: "f".repeat(40),
      releasedAt: null,
    };
    const library = buildLibrary(
      { ...previewCatalog, versions: [undated, ...previewCatalog.versions] },
      device,
    );
    expect(
      library
        .find((item) => item.id === "elite-redux")!
        .releases.map((item) => item.label),
    ).toEqual(["v2.6", "v2.5", "v2.4", "vnightly"]);
  });

  it("joins local runs with cloud copies and outbox state", () => {
    const runs = game("elite-redux").playthroughs;
    expect(runs.map((run) => [run.name, run.release.label, run.sync])).toEqual([
      ["Main adventure", "v2.5", "synced"],
      ["Randomizer run", "v2.5", "unsynced"],
      ["Challenge run", "v2.4", "cloud-only"],
    ]);
    expect(runs[0].conflict).toBeUndefined();
    expect(runs[1].conflict).toEqual({
      deviceLastPlayed: runs[1].lastPlayed,
      cloudCapturedAt: expect.any(Number),
      cloudUploadedAt: expect.any(Number),
    });
    expect(game("elite-redux").lastPlayed).toBe(runs[0].lastPlayed);
  });

  it("offers the latest release to runs on any other release", () => {
    expect(game("elite-redux").updatable.map((run) => run.name)).toEqual([
      "Main adventure",
      "Randomizer run",
      "Challenge run",
    ]);
    expect(game("emerald-seaglass").updatable).toEqual([]);
  });

  it("tells a newer cloud copy from backups being off", () => {
    const quiet: LibraryDevice = {
      ...device,
      jobs: device.jobs.filter((job) => job.playthroughId !== "main"),
      cloud: device.cloud.filter((copy) => copy.playthroughId !== "seaglass"),
    };
    const library = buildLibrary(previewCatalog, quiet);
    const find = (gameId: string, runId: string) =>
      library
        .find((item) => item.id === gameId)!
        .playthroughs.find((run) => run.id === runId)!;
    // A cloud copy this device never synced with.
    expect(find("elite-redux", "main").sync).toBe("cloud-newer");
    expect(find("elite-redux", "main").conflict).toBeDefined();
    // Synced once, and now no cloud copy: backups are off or it was deleted.
    expect(find("emerald-seaglass", "seaglass").sync).toBe("device-only");
  });

  it("verifies save compatibility only when the catalog declares it", () => {
    const [latest, previous] = game("elite-redux").releases;
    expect(saveCompatible(previous, latest)).toBe(false);
    expect(
      saveCompatible(previous, {
        ...latest,
        saveCompatibleFrom: [previous.id],
      }),
    ).toBe(true);
    expect(saveCompatible(latest, latest)).toBe(true);
  });
});

describe("filterGames", () => {
  // The catalog's games; the base ROMs on the device sort among them.
  const listed = games.filter((item) => item.latest.label);
  const ids = (filters: Partial<typeof noFilters>) =>
    filterGames(listed, { ...noFilters, ...filters }).map((item) => item.id);

  it("places unplayed local ROMs among unplayed games by title", () => {
    expect(
      filterGames(games, noFilters)
        .slice(2)
        .map((item) => item.title),
    ).toEqual([
      "Fire Emblem (US)",
      "Fire Emblem: Vision Quest",
      "Golden Sun: Reloaded",
      "Pokémon Emerald (US)",
      "Pokémon FireRed (US 1.0)",
      "Pokémon Unbound",
    ]);
  });

  it("sorts by play, then unplayed games by title", () => {
    expect(ids({})).toEqual([
      "elite-redux",
      "emerald-seaglass",
      "vision-quest",
      "golden-sun-reloaded",
      "unbound",
    ]);
  });

  it("sorts by latest release date or by launches", () => {
    expect(ids({ sort: "updated" })).toEqual([
      "elite-redux",
      "emerald-seaglass",
      "golden-sun-reloaded",
      "vision-quest",
      "unbound",
    ]);
    expect(ids({ sort: "popularity" })).toEqual([
      "unbound",
      "elite-redux",
      "vision-quest",
      "emerald-seaglass",
      "golden-sun-reloaded",
    ]);
  });

  it("combines search, availability, franchise and every selected tag", () => {
    expect(ids({ query: " SEA " })).toEqual(["emerald-seaglass"]);
    expect(ids({ show: "installed" })).toEqual([
      "elite-redux",
      "emerald-seaglass",
    ]);
    expect(ids({ show: "ready" })).toEqual(["vision-quest", "unbound"]);
    expect(ids({ show: "ready", franchise: "Pokémon" })).toEqual(["unbound"]);
    expect(ids({ tags: ["Difficulty Hack"] })).toEqual([
      "elite-redux",
      "golden-sun-reloaded",
    ]);
    expect(ids({ tags: ["Difficulty Hack", "Randomizer"] })).toEqual([
      "elite-redux",
    ]);
  });

  it("lists franchises and tags once, alphabetically", () => {
    expect(franchisesOf(games)).toEqual([
      "Fire Emblem",
      "Golden Sun",
      "Pokémon",
    ]);
    expect(tagsOf(games)).toContain("Quality of life");
    expect(new Set(tagsOf(games)).size).toBe(tagsOf(games).length);
  });
});

describe("dates", () => {
  const at = (day: number, hour: number, minute = 0) =>
    new Date(2026, 8, day, hour, minute).getTime();

  it("describes recent play relatively, then by day", () => {
    expect(formatAgo(now - 30_000, now)).toBe("just now");
    expect(formatAgo(now - 60_000, now)).toBe("1 minute ago");
    expect(formatAgo(now - 2 * 3_600_000, now)).toBe("2 hours ago");
    expect(formatAgo(at(22, 14, 20), now)).toBe(
      new Date(at(22, 14, 20)).toLocaleDateString(undefined, {
        month: "short",
        day: "numeric",
      }),
    );
    expect(formatAgo(new Date(2025, 8, 22).getTime(), now)).toContain("2025");
  });

  it("names today and yesterday", () => {
    expect(formatMoment(at(25, 14, 20), now)).toMatch(/^today, /);
    expect(formatMoment(at(24, 21, 10), now)).toMatch(/^yesterday, /);
    expect(formatMoment(at(20, 9, 5), now)).not.toMatch(/^(today|yesterday)/);
  });
});
