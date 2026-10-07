import { describe, expect, it } from "vite-plus/test";
import { locatable, resolveGame } from "@evenfall/gba/bridge";
import { pokemonPlugin } from "../ui";
import { fireRedUpgrade } from "../src/cfru";
import { eliteRedux } from "../src/elite-redux";
import { heartNSoul } from "../src/heart-n-soul";
import {
  EliteReduxDashboard,
  FireRedUpgradeDashboard,
  HeartNSoulDashboard,
} from "../ui/pokemon-dashboard";

const hnsHash = "a4be81f8330d7f223a3ab847c3da822fe6f8a2e0";

describe("player plugin", () => {
  it("registers exact reviewed builds only", () => {
    // Heart & Soul 1.2.1, the Redux release and Unbound 2.1.1.1.
    expect(resolveGame(pokemonPlugin.games, hnsHash)).toBe(heartNSoul);
    expect(
      resolveGame(
        pokemonPlugin.games,
        "f1323126bc043f3ce931bcc670957930559eb515",
      ),
    ).toBe(eliteRedux);
    expect(
      resolveGame(
        pokemonPlugin.games,
        "b4776b82a4c7915d0fadeaa27e013523f99dfd94",
      ),
    ).toBe(fireRedUpgrade);
    expect(resolveGame(pokemonPlugin.games, `${hnsHash.slice(0, -1)}e`)).toBe(
      null,
    );
    // Heart & Soul 1.2.0 and the Redux debug build are located by the player.
    for (const hash of [
      "b8a4b491ca67a76f079a8e7c19fd5512a2147dd2",
      "299165fb56e19ac8afe5d66f83dd5bada823d58c",
    ])
      expect(resolveGame(pokemonPlugin.games, hash)).toBe(null);
    for (const game of pokemonPlugin.games)
      expect(locatable(game)?.template.provenance.method).toBe("reviewed");
  });

  it("loads a dashboard per game", async () => {
    expect(await pokemonPlugin.dashboards()).toEqual({
      [heartNSoul.id]: HeartNSoulDashboard,
      [eliteRedux.id]: EliteReduxDashboard,
      [fireRedUpgrade.id]: FireRedUpgradeDashboard,
    });
  });
});
