import { describe, expect, it } from "vitest";
import { previewCatalog, previewDevice } from "../src/pages/library/fixtures";
import type { LibraryDevice } from "../src/pages/library/catalog";
import { formatAgo, formatMoment } from "../src/pages/library/format";
import {
  buildLibrary,
  filterGames,
  franchisesOf,
  noFilters,
  saveCompatible,
  tagsOf,
} from "../src/pages/library/model";

const now = new Date(2026, 8, 25, 16, 0).getTime();
const device = previewDevice(now);
const games = buildLibrary(previewCatalog, device);
const game = (id: string) => games.find((item) => item.id === id)!;

describe("buildLibrary", () => {
  it("derives availability from ROMs and base ROMs on this device", () => {
    expect(
      Object.fromEntries(games.map((item) => [item.id, item.availability])),
    ).toEqual({
      "elite-redux": "installed",
      "emerald-seaglass": "installed",
      "vision-quest": "ready",
      "golden-sun-reloaded": "needs-file",
      unbound: "ready",
    });
    expect(game("golden-sun-reloaded").missingFile).toBe(
      "Golden Sun: The Lost Age (US)",
    );
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
  const ids = (filters: Partial<typeof noFilters>) =>
    filterGames(games, { ...noFilters, ...filters }).map((item) => item.id);

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
