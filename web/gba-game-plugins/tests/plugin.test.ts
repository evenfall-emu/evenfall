import { describe, expect, it } from "vitest";
import { resolveGame } from "@evenfall/gba/bridge";
import { pokemonPlugin } from "../ui";
import { eliteRedux } from "../src/elite-redux";
import { heartNSoul } from "../src/heart-n-soul";
import {
  EliteReduxDashboard,
  HeartNSoulDashboard,
} from "../ui/pokemon-dashboard";
import { HeartNSoulLayout } from "../ui/party-hud";

const hnsHash =
  "7c4f90c8b68b64d4639a37306d5302a11cd4d7c44005114ec2faa2b28b210c3d";

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

  it("loads a dashboard per game and the Heart & Soul HUD", async () => {
    expect(await pokemonPlugin.dashboards()).toEqual({
      [heartNSoul.id]: HeartNSoulDashboard,
      [eliteRedux.id]: EliteReduxDashboard,
    });
    expect(await pokemonPlugin.huds()).toEqual({
      [heartNSoul.id]: HeartNSoulLayout,
    });
  });
});
