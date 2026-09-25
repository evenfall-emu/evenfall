import { useMemo } from "react";
import { CreatureDetails, type GameCreature } from "./game-layout";
import type { PokemonResources } from "../src/bridges";
import {
  statKeys,
  type PokemonArtworkRef,
  type PokemonItemArtworkRef,
  type PokemonTrainerArtworkRef,
  type PokemonPartyMember,
  type PokemonPartyView,
} from "../src/model";

/** Bounded, resource-scoped PNG cache. No ROM bytes or pixels enter snapshots. */
export function usePokemonArtwork(resources: PokemonResources | undefined) {
  return useMemo(() => {
    const cache = new Map<string, { src?: string; reason?: string }>();
    return (
      ref:
        | PokemonArtworkRef
        | PokemonItemArtworkRef
        | PokemonTrainerArtworkRef
        | undefined,
    ): { src?: string; reason?: string } => {
      if (!ref || !resources) return {};
      const key = JSON.stringify(ref);
      const cached = cache.get(key);
      if (cached) return cached;
      let result: { src?: string; reason?: string };
      try {
        const image = resources.artwork(ref);
        const canvas = document.createElement("canvas");
        canvas.width = image.width;
        canvas.height = image.height;
        const context = canvas.getContext("2d");
        if (!context) throw new Error("Canvas is unavailable");
        const data = context.createImageData(image.width, image.height);
        data.data.set(image.pixels);
        context.putImageData(data, 0, 0);
        result = { src: canvas.toDataURL() };
      } catch (error) {
        result = {
          reason: error instanceof Error ? error.message : String(error),
        };
      }
      if (cache.size >= 64) cache.delete(cache.keys().next().value!);
      cache.set(key, result);
      return result;
    };
  }, [resources]);
}

type Artwork = ReturnType<typeof usePokemonArtwork>;

/** Shared semantic mapping; internal IDs are never treated as National Dex IDs. */
export function partyCreatures(
  { party }: PokemonPartyView,
  artwork: Artwork,
): GameCreature[] {
  const seen = new Map<string, number>();
  return party.map((pokemon) => {
    const identity =
      pokemon.id ??
      `${pokemon.personality}:${pokemon.trainerId}:${pokemon.species}`;
    const occurrence = seen.get(identity) ?? 0;
    seen.set(identity, occurrence + 1);
    return {
      id: `${identity}:${occurrence}`,
      name: pokemon.isEgg ? "Egg" : pokemon.nickname || pokemon.speciesName,
      level: pokemon.level,
      hp: pokemon.hp,
      maxHp: pokemon.maxHp,
      artwork: artwork(pokemon.artwork?.icon).src,
      subtitle: pokemon.isEgg
        ? undefined
        : `${pokemon.speciesName} · ${pokemon.gender === "male" ? "♂" : pokemon.gender === "female" ? "♀" : "—"}${pokemon.shiny ? " · Shiny" : ""}`,
      tags: pokemon.types?.map(({ name }) => name),
      status:
        pokemon.hp === 0
          ? "Fainted"
          : pokemon.ailments.join(" · ") || undefined,
    };
  });
}

const statLabels = {
  hp: "HP",
  attack: "Attack",
  defense: "Defense",
  speed: "Speed",
  specialAttack: "Sp. Atk",
  specialDefense: "Sp. Def",
};

/** Pokémon presentation shared by every game connector, with no addresses or HnS policy. */
export function PokemonDetails({
  pokemon,
  creature,
  artwork,
}: {
  pokemon: PokemonPartyMember;
  creature: GameCreature;
  artwork: Artwork;
}) {
  const portrait = artwork(pokemon.artwork?.front);
  const stats = pokemon.stats.actual;
  return (
    <>
      <CreatureDetails
        creature={{ ...creature, artwork: portrait.src ?? creature.artwork }}
        stats={
          stats
            ? statKeys.map((key) => ({
                label: statLabels[key],
                value: stats[key],
              }))
            : undefined
        }
        moves={pokemon.moves.flatMap((move) =>
          move
            ? [
                {
                  name: move.name,
                  type: move.type.name,
                  category: move.category ?? undefined,
                  current: move.pp ?? undefined,
                  maximum: move.maxPp,
                },
              ]
            : [],
        )}
      />
      <dl className="pokemon-facts">
        <dt>Nature</dt>
        <dd>{pokemon.nature?.name ?? "Unavailable"}</dd>
        <dt>Ability</dt>
        <dd>
          {pokemon.abilities === null
            ? "Unavailable"
            : pokemon.abilities.length
              ? pokemon.abilities.map((ability) => (
                  <span
                    className={`pokemon-ability ${ability.state}`}
                    title={ability.reason}
                    key={`${ability.slot}:${ability.kind}`}
                  >
                    {ability.name}
                    {ability.state !== "available" && ` (${ability.state})`}
                  </span>
                ))
              : "None"}
        </dd>
        <dt>Item</dt>
        <dd>{pokemon.heldItem?.name ?? "None"}</dd>
        <dt>Friendship</dt>
        <dd>{pokemon.friendship ?? "Unavailable"}</dd>
        <dt>Experience</dt>
        <dd>
          {pokemon.experience?.total ?? "Unavailable"}
          {pokemon.experience &&
            pokemon.experience.nextLevel !== null &&
            ` / ${pokemon.experience.nextLevel}`}
        </dd>
      </dl>
      <details className="pokemon-training">
        <summary>IVs & EVs</summary>
        <table>
          <thead>
            <tr>
              <th>Stat</th>
              <th>IV</th>
              <th>EV</th>
            </tr>
          </thead>
          <tbody>
            {statKeys.map((key) => (
              <tr key={key}>
                <th>{statLabels[key]}</th>
                <td>{pokemon.stats.ivs[key]}</td>
                <td>{pokemon.stats.evs[key]}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </details>
      {portrait.reason && (
        <p className="hud-note">Portrait unavailable: {portrait.reason}</p>
      )}
      {pokemon.warnings.map((warning) => (
        <p className="hud-note" key={warning}>
          {warning}
        </p>
      ))}
    </>
  );
}
