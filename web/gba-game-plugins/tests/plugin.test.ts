import { describe, expect, it } from "vitest";
import { resolveGame } from "@evenfall/gba/bridge";
import { pokemonPlugin } from "../ui";
import { eliteRedux } from "../src/elite-redux";
import { heartNSoul } from "../src/heart-n-soul";
import {
  EliteReduxDashboard,
  HeartNSoulDashboard,
} from "../ui/pokemon-dashboard";

const hnsHash = "a4be81f8330d7f223a3ab847c3da822fe6f8a2e0";

describe("player plugin", () => {
  it("registers exact builds only", () => {
    expect(resolveGame(pokemonPlugin.games, hnsHash)).toBe(heartNSoul);
    expect(
      resolveGame(pokemonPlugin.games, eliteRedux.profiles[0].romHash),
    ).toBe(eliteRedux);
    expect(resolveGame(pokemonPlugin.games, `${hnsHash.slice(0, -1)}e`)).toBe(
      null,
    );
  });

  it("loads a dashboard per game", async () => {
    expect(await pokemonPlugin.dashboards()).toEqual({
      [heartNSoul.id]: HeartNSoulDashboard,
      [eliteRedux.id]: EliteReduxDashboard,
    });
  });
});
