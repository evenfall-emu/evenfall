import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Combobox } from "@base-ui/react/combobox";
import { useViews } from "@evenfall/gba/bridge/react";
import { HpBar, hpTone, TypeBadge } from "./game-layout";
import { usePokemonArtwork } from "./pokemon-ui";
import { InfoTip } from "./info-tip";
import type { BridgeState, GameArenaStatus } from "@evenfall/gba/bridge";
import type { Gen3Game } from "../src/gen3";
import { heartNSoul } from "../src/heart-n-soul";
import { eliteRedux } from "../src/elite-redux";
import { searchKey } from "../src/search";
import type { PokemonResources } from "../src/bridges";
import type { BuildInfo, GameLayoutProps } from "./layout";
import type {
  PokemonAbility,
  PokemonDetails,
  PokemonLinksFactory,
  PokemonPartyMember,
  PokemonPartyView,
  PokemonStorageView,
  PokemonBattleView,
  PokemonStat,
  PokemonBattleBattler,
  PokemonTrainerPreview,
  PokemonTrainerSearch,
  DescribedValue,
  MoveCategory,
} from "../src/model";

type Pokemon = PokemonDetails &
  Partial<Pick<PokemonPartyMember, "hp" | "maxHp" | "ailments" | "slot">> &
  Partial<Pick<PokemonBattleBattler, "statStages" | "stageAdjustedStats">>;
type Artwork = ReturnType<typeof usePokemonArtwork>;
const nameOf = (pokemon: PokemonDetails) =>
  pokemon.isEgg
    ? "Egg"
    : pokemon.nickname && pokemon.nickname !== pokemon.speciesName
      ? pokemon.nickname
      : displayName(pokemon.speciesName);
const identity = (pokemon: PokemonDetails) =>
  pokemon.id ??
  `${pokemon.personality}:${pokemon.trainerId}:${pokemon.species}`;
const displayName = (name: string) =>
  name.toLowerCase().replace(/\b\w/g, (c) => c.toUpperCase());
const priorityText = (priority: number | null | undefined) =>
  priority == null ? null : priority > 0 ? `+${priority}` : String(priority);
const stats: readonly [PokemonStat, string, string][] = [
  ["hp", "HP", "HP"],
  ["attack", "Atk", "Attack"],
  ["defense", "Def", "Defense"],
  ["specialAttack", "SpA", "Special Attack"],
  ["specialDefense", "SpD", "Special Defense"],
  ["speed", "Spe", "Speed"],
];
const ailmentNames: Record<string, string> = {
  Poison: "PSN",
  "Bad poison": "TOX",
  Burn: "BRN",
  Freeze: "FRZ",
  Paralysis: "PAR",
  Sleep: "SLP",
};
// UI conventions, not ROM metadata. Unknown ailments keep the CSS fallback.
const ailmentColors: readonly [RegExp, string][] = [
  [/^(?:brn|burn(?:ed|t)?)$/i, "#f67843"],
  [/^(?:par|para|paralysis|paraly[sz]ed)$/i, "#f4d628"],
  [/^(?:frz|freeze|frozen|frostb[iy]te)$/i, "#98d9da"],
  [/^bleed(?:ing)?$/i, "#ff786a"],
  [/^(?:slp|sleep|asleep)$/i, "#ffffff"],
];
// Original ZiGBA glyphs drawn on a 3:2 grid: a starburst for contact, a
// sparkle for special attacks and a half-filled disc for status effects.
const categoryGlyphs: Record<MoveCategory, ReactNode> = {
  physical: (
    <polygon
      fill="#ff8a4c"
      stroke="#4a1a08"
      strokeWidth="0.8"
      strokeLinejoin="round"
      points="15,0.8 16.7,5.9 21.5,3.5 19.1,8.3 24.2,10 19.1,11.7 21.5,16.5 16.7,14.1 15,19.2 13.3,14.1 8.5,16.5 10.9,11.7 5.8,10 10.9,8.3 8.5,3.5 13.3,5.9"
    />
  ),
  special: (
    <g fill="#74b4ff">
      <path d="M15 1Q16.2 8.8 24 10Q16.2 11.2 15 19Q13.8 11.2 6 10Q13.8 8.8 15 1Z" />
      <circle cx="23" cy="4" r="1.6" />
      <circle cx="7.5" cy="15.8" r="1.2" />
    </g>
  ),
  status: (
    <g fill="#c9d1d9">
      <circle
        cx="15"
        cy="10"
        r="7.6"
        fill="none"
        stroke="#c9d1d9"
        strokeWidth="1.8"
      />
      <path d="M15 2.4A7.6 7.6 0 0 0 15 17.6Z" />
    </g>
  ),
};

function Condition({ pokemon }: { pokemon: Pokemon }) {
  const value = pokemon.hp === 0 ? "FNT" : pokemon.ailments?.[0];
  if (!value) return null;
  const color = ailmentColors.find(([pattern]) =>
    pattern.test(value.trim()),
  )?.[1];
  return (
    <span
      className="condition affected"
      style={color ? { backgroundColor: color, color: "#122127" } : undefined}
    >
      {ailmentNames[value] ?? value}
    </span>
  );
}

function Description({ value }: { value: DescribedValue }) {
  return (
    <>
      <p className="tip-description">
        {value.description || "Description unavailable for this profile."}
      </p>
      {value.warnings?.map((warning) => (
        <small key={warning}>{warning}</small>
      ))}
    </>
  );
}

const abilityKind = (ability: PokemonAbility) =>
  ability.kind === "ability"
    ? "Ability"
    : ability.kind === "innate"
      ? "Innate"
      : "Extra";

function AbilityTip({ ability }: { ability: PokemonAbility }) {
  const state =
    ability.state === "available" ? null : displayName(ability.state);
  return (
    <InfoTip
      label={`${abilityKind(ability)}: ${ability.name}${state ? ` (${state})` : ""}`}
      content={
        <>
          <h3>{displayName(ability.name)}</h3>
          <p className="muted">
            {abilityKind(ability)}
            {state && ` · ${state}`}
          </p>
          {ability.reason && <p className="ability-reason">{ability.reason}</p>}
          <Description value={ability} />
        </>
      }
    >
      <span className="ability-name">{ability.name}</span>
      {state && <small className={ability.state}>{state}</small>}
    </InfoTip>
  );
}

function Category({
  category,
  detail = false,
}: {
  category: MoveCategory | null;
  detail?: boolean;
}) {
  return (
    <span
      className={`move-category ${category ?? "unknown"}`}
      aria-label={category ?? "Category unavailable"}
    >
      <i aria-hidden="true">
        {category ? (
          <svg viewBox="0 0 30 20" width="60" height="40" focusable="false">
            {categoryGlyphs[category]}
          </svg>
        ) : (
          "?"
        )}
      </i>
      {detail && (
        <span>{category ? displayName(category) : "Category unavailable"}</span>
      )}
    </span>
  );
}

/** A layout can display any game-authored field without a UI-specific switch. */
export function ArenaStatus({
  arena,
  build,
}: {
  arena: GameArenaStatus;
  build?: BuildInfo;
}) {
  return (
    <header className="scene-heading arena-heading">
      <h1>{arena.title}</h1>
      {build?.provenance === "located" && (
        <p
          className="build-warning"
          title={[
            "Recognized from this game's locators; nobody has reviewed this build.",
            ...(build.review.length
              ? [`Unconfirmed: ${build.review.join(", ")}`]
              : []),
          ].join("\n")}
        >
          Detected build · not reviewed
        </p>
      )}
      <dl className="arena-status" aria-label="Arena status">
        {arena.fields.map((field) => (
          <div key={field.id} title={field.reason}>
            <dt>{field.label}</dt>
            <dd>{field.value ?? "Unavailable"}</dd>
          </div>
        ))}
      </dl>
    </header>
  );
}

export function PokemonInspector({
  pokemon,
  artwork,
  linksFactory,
  active = false,
}: {
  pokemon?: Pokemon;
  artwork: Artwork;
  linksFactory?: PokemonLinksFactory;
  active?: boolean;
}) {
  if (!pokemon)
    return (
      <section className="inspector" aria-label="Selected Pokémon">
        <h2 className="strip">Selected Pokémon</h2>
        <p className="panel-empty">Select a Pokémon to inspect.</p>
      </section>
    );
  const image = artwork(pokemon.artwork?.front);
  const itemImage = artwork(pokemon.heldItem?.artwork ?? undefined);
  const totals = active ? pokemon.stageAdjustedStats : pokemon.stats.actual;
  const pokemonLink = linksFactory?.pokemon?.(pokemon);
  return (
    <section className="inspector" aria-label="Selected Pokémon">
      <h2 className="strip">Selected Pokémon</h2>
      <div className="selected-summary">
        {image.src ? (
          <img src={image.src} alt={nameOf(pokemon)} />
        ) : (
          <span className="sprite-missing" title={image.reason}>
            ?
          </span>
        )}
        <div className="selected-info">
          <div className="selected-name">
            <h3
              title={`${pokemon.nature?.name ?? "Unknown"} nature · ${pokemon.speciesName}`}
            >
              {nameOf(pokemon)}
            </h3>
            <span>Lv{pokemon.level}</span>
            {pokemonLink && (
              <a
                className="pokemon-dex-link"
                href={pokemonLink}
                target="_blank"
                rel="noopener noreferrer"
                aria-label={`Open ${displayName(pokemon.speciesName)} in Pokédex (new tab)`}
                title="Open in Pokédex (new tab)"
              >
                <svg
                  viewBox="0 0 16 16"
                  fill="currentColor"
                  shapeRendering="crispEdges"
                  aria-hidden="true"
                >
                  <path d="M9 1h6v6h-2V5h-2v2H9v2H7V7h2V5h2V3H9zM1 4h5v2H3v7h7v-3h2v5H1z" />
                </svg>
              </a>
            )}
          </div>
          <div className="selected-tags">
            {pokemon.types?.map((type) => (
              <TypeBadge type={type.name} key={type.id} />
            ))}
            {pokemon.hp !== undefined && <Condition pokemon={pokemon} />}
            {active && <span className="in-battle">In battle</span>}
          </div>
          {pokemon.hp !== undefined && pokemon.maxHp !== undefined ? (
            <HpBar
              value={pokemon.hp}
              max={pokemon.maxHp}
              label={`HP for ${nameOf(pokemon)}`}
            />
          ) : (
            <span className="muted">Stored Pokémon · HP unavailable</span>
          )}
        </div>
      </div>
      <div
        className="pokemon-stat-grid"
        role="group"
        aria-label="Stats"
        data-tooltip-anchor
      >
        {stats.map(([key, label, name]) => {
          const stage =
            active && key !== "hp" ? pokemon.statStages?.[key] : undefined;
          const stageText =
            stage == null
              ? "Unavailable"
              : stage > 0
                ? `+${stage}`
                : stage < 0
                  ? String(stage)
                  : "Neutral";
          return (
            <InfoTip
              key={key}
              label={`Stat: ${name}, total ${totals?.[key] ?? "unavailable"}${stage ? `, stage ${stageText}` : ""}`}
              content={
                <>
                  <h3>{name}</h3>
                  <dl className="stat-details">
                    <dt>Base</dt>
                    <dd>{pokemon.stats.base?.[key] ?? "Unavailable"}</dd>
                    <dt>IV</dt>
                    <dd>{pokemon.stats.ivs[key]}</dd>
                    <dt>EV</dt>
                    <dd>{pokemon.stats.evs[key]}</dd>
                    <dt>Nature</dt>
                    <dd>{pokemon.nature?.name ?? "Unavailable"}</dd>
                    {active && (
                      <>
                        <dt>Before stages</dt>
                        <dd>{pokemon.stats.actual?.[key] ?? "Unavailable"}</dd>
                        {key !== "hp" && (
                          <>
                            <dt>Battle stage</dt>
                            <dd
                              className={
                                stage
                                  ? stage > 0
                                    ? "stage-up"
                                    : "stage-down"
                                  : undefined
                              }
                            >
                              {stageText}
                            </dd>
                          </>
                        )}
                      </>
                    )}
                    <dt>Total</dt>
                    <dd>
                      <strong>{totals?.[key] ?? "Unavailable"}</strong>
                    </dd>
                  </dl>
                  <small>
                    {active
                      ? "Totals include stages; other battle modifiers excluded."
                      : "Total shows calculated party stats."}
                  </small>
                </>
              }
            >
              <span className="stat-label">{label}</span>
              <span className="stat-value">
                <strong>{totals?.[key] ?? "—"}</strong>
                {!!stage && (
                  <span
                    className={stage > 0 ? "stage-up" : "stage-down"}
                    aria-hidden="true"
                  >
                    {stage > 0 ? "↑" : "↓"}
                  </span>
                )}
              </span>
            </InfoTip>
          );
        })}
      </div>
      <div className="fact-line ability-line" data-tooltip-anchor>
        <span>Ability</span>
        {pokemon.abilities === null ? (
          <strong>Unavailable</strong>
        ) : pokemon.abilities.length ? (
          <div className="ability-list">
            {pokemon.abilities.map((ability) => (
              <AbilityTip
                ability={ability}
                key={`${ability.slot}:${ability.kind}`}
              />
            ))}
          </div>
        ) : (
          <strong>None</strong>
        )}
      </div>
      <div className="fact-line item-line" data-tooltip-anchor>
        <span>Item</span>
        {pokemon.heldItem ? (
          <InfoTip
            label={`Item: ${pokemon.heldItem.name}`}
            content={
              <>
                <div className="item-heading">
                  {itemImage.src && (
                    <img
                      className="item-sprite"
                      src={itemImage.src}
                      alt=""
                      width={24}
                      height={24}
                    />
                  )}
                  <div>
                    <h3>{displayName(pokemon.heldItem.name)}</h3>
                    <p className="muted">Item</p>
                  </div>
                </div>
                <Description value={pokemon.heldItem} />
                {!itemImage.src && (
                  <small>
                    {itemImage.reason ??
                      "Item artwork unavailable for this profile."}
                  </small>
                )}
              </>
            }
          >
            {itemImage.src && (
              <img
                className="item-sprite"
                src={itemImage.src}
                alt=""
                width={24}
                height={24}
              />
            )}
            {pokemon.heldItem.name}
          </InfoTip>
        ) : (
          <strong>None</strong>
        )}
      </div>
      <table className="pokemon-move-table" aria-label="Moves">
        <colgroup>
          <col style={{ width: "22%" }} />
          <col style={{ width: "38%" }} />
          <col style={{ width: "6%" }} />
          <col style={{ width: "13%" }} />
          <col style={{ width: "21%" }} />
        </colgroup>
        <thead>
          <tr>
            <th colSpan={3}>Moves</th>
            <th title="Move priority">Pri</th>
            <th>PP</th>
          </tr>
        </thead>
        <tbody>
          {Array.from({ length: 4 }, (_, slot) => {
            const move = pokemon.moves[slot];
            return (
              <tr key={slot} data-tooltip-anchor>
                {move ? (
                  <>
                    <td>
                      <TypeBadge type={move.type.name} />
                    </td>
                    <th>
                      <InfoTip
                        label={`Move: ${displayName(move.name)}`}
                        content={
                          <>
                            <div className="tip-heading">
                              <h3>{displayName(move.name)}</h3>
                              <TypeBadge type={move.type.name} />
                            </div>
                            <div className="move-facts">
                              <span>
                                Power {move.power > 0 ? move.power : "—"}
                              </span>
                              <Category category={move.category} detail />
                            </div>
                            <Description value={move} />
                            <p className="tip-priority">
                              Priority{" "}
                              {priorityText(move.priority) ?? "Unavailable"}
                            </p>
                          </>
                        }
                      >
                        {displayName(move.name)}
                      </InfoTip>
                    </th>
                    <td>
                      <Category category={move.category} />
                    </td>
                    <td
                      title={
                        move.priority == null
                          ? "Priority unavailable"
                          : "Move priority"
                      }
                      className={
                        move.priority != null && move.priority > 0
                          ? "stage-up"
                          : undefined
                      }
                    >
                      {priorityText(move.priority) ?? "—"}
                    </td>
                    <td>
                      {move.pp ?? "—"} / {move.maxPp}
                    </td>
                  </>
                ) : (
                  <>
                    <td />
                    <td colSpan={3} className="muted">
                      —
                    </td>
                    <td>—</td>
                  </>
                )}
              </tr>
            );
          })}
        </tbody>
      </table>
      {pokemon.warnings.length > 0 && (
        <details className="data-warning">
          <summary>Some data is unavailable</summary>
          {pokemon.warnings.map((w) => (
            <p key={w}>{w}</p>
          ))}
        </details>
      )}
    </section>
  );
}

export function PokemonRoster({
  party,
  stored = [],
  artwork,
  linksFactory,
  opponent = false,
  label,
  message,
  activeSlots = [],
}: {
  party: readonly PokemonPartyMember[];
  stored?: readonly PokemonDetails[];
  artwork: Artwork;
  linksFactory?: PokemonLinksFactory;
  opponent?: boolean;
  label: string;
  message?: string;
  activeSlots?: readonly number[];
}) {
  const [query, setQuery] = useState("");
  const [selectedId, select] = useState<string>();
  const entries = (query ? [...party, ...stored] : party).map(
    (pokemon, index, all) => ({
      pokemon,
      id: `${identity(pokemon)}:${all.slice(0, index).filter((p) => identity(p) === identity(pokemon)).length}`,
    }),
  );
  const visible = entries.filter(
    ({ pokemon }) =>
      !query ||
      searchKey(`${pokemon.nickname} ${pokemon.speciesName}`).includes(
        searchKey(query),
      ),
  );
  const selected =
    visible.find(({ id }) => id === selectedId) ??
    visible.find(
      ({ pokemon }) =>
        "slot" in pokemon && activeSlots.includes(pokemon.slot as number),
    ) ??
    visible[0];
  return (
    <aside
      className={`pokemon-roster ${opponent ? "opponent-roster" : "player-roster"}`}
      aria-label={opponent ? "Opponent" : "Your Pokémon"}
    >
      <div className="roster-heading strip">
        <h2>{opponent ? "Opponent" : "Your Pokémon"}</h2>
        <span>{label}</span>
      </div>
      {!opponent && (
        <>
          <label className="team-search">
            <span aria-hidden="true">⌕</span>
            <input
              type="search"
              placeholder="Search team & PC…"
              aria-label="Search team and PC"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </label>
          <h2 className="strip team-label">
            {query ? "Search results" : "Current team"}
          </h2>
        </>
      )}
      <ol
        className="roster-grid"
        aria-label={opponent ? "Opponent team" : "Current team"}
        onKeyDown={(event) => {
          const index = visible.findIndex((entry) => entry.id === selected?.id);
          const offset =
            event.key === "ArrowRight"
              ? 1
              : event.key === "ArrowLeft"
                ? -1
                : event.key === "ArrowDown"
                  ? 2
                  : event.key === "ArrowUp"
                    ? -2
                    : 0;
          const next =
            event.key === "Home"
              ? 0
              : event.key === "End"
                ? visible.length - 1
                : offset
                  ? (index + offset + visible.length) % visible.length
                  : -1;
          if (next < 0 || !visible[next]) return;
          event.preventDefault();
          select(visible[next].id);
          event.currentTarget
            .querySelectorAll<HTMLButtonElement>("button")
            [next]?.focus();
        }}
      >
        {visible.map(({ pokemon, id }) => {
          const image = artwork(pokemon.artwork?.front);
          const live =
            "hp" in pokemon ? (pokemon as PokemonPartyMember) : undefined;
          return (
            <li key={id}>
              <button
                className={`roster-card ${selected?.id === id ? "selected" : ""}`}
                aria-pressed={selected?.id === id}
                onClick={() => select(id)}
              >
                {image.src ? (
                  <img src={image.src} alt="" />
                ) : (
                  <span className="sprite-missing">?</span>
                )}
                <div>
                  <div className="card-name">
                    <strong>{nameOf(pokemon)}</strong>
                    <span>Lv{pokemon.level}</span>
                  </div>
                  {live ? (
                    <HpBar
                      value={live.hp}
                      max={live.maxHp}
                      label={`HP for ${nameOf(pokemon)}`}
                    />
                  ) : (
                    <span className="muted">PC storage</span>
                  )}
                </div>
              </button>
            </li>
          );
        })}
        {!visible.length && (
          <li className="roster-empty">
            {message ??
              (query
                ? "No matching Pokémon"
                : opponent
                  ? "No active battle"
                  : "No Pokémon in your party")}
          </li>
        )}
      </ol>
      <PokemonInspector
        pokemon={selected?.pokemon}
        artwork={artwork}
        linksFactory={linksFactory}
        active={
          !!selected &&
          "slot" in selected.pokemon &&
          activeSlots.includes(selected.pokemon.slot as number)
        }
      />
    </aside>
  );
}

const trainerResultLimit = 50;

function TrainerPortrait({
  trainer,
  artwork,
}: {
  trainer: PokemonTrainerPreview;
  artwork: Artwork;
}) {
  const image = artwork(trainer.portrait ?? undefined);
  return image.src ? (
    <img src={image.src} alt="" />
  ) : (
    <span className="trainer-portrait-missing" title={image.reason}>
      ?
    </span>
  );
}

/** Search is connector-supplied and synchronous over its current coherent context. */
export function TrainerSearch({
  search,
  artwork,
  linksFactory,
}: {
  search: PokemonTrainerSearch;
  artwork: Artwork;
  linksFactory?: PokemonLinksFactory;
}) {
  const root = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState("");
  const [selection, setSelection] = useState<{
    id: string;
    searchName: string;
  }>();
  const result = useMemo(() => search(query), [query, search]);
  const visible = useMemo(
    () =>
      result.status === "ready" ? result.data.slice(0, trainerResultLimit) : [],
    [result],
  );
  const selectedResult = useMemo(
    () => (selection ? search(selection.searchName) : null),
    [search, selection],
  );
  const selected =
    selection && selectedResult?.status === "ready"
      ? selectedResult.data.find((trainer) => trainer.id === selection.id)
      : undefined;
  const resultMessage =
    result.status === "unavailable"
      ? result.reason
      : query
        ? "No trainers match this search."
        : "Search by trainer name or variant.";
  return (
    <div className="trainer-opponent-column">
      <section className="trainer-search-panel" aria-label="Trainer search">
        <div className="roster-heading strip">
          <h2>Trainer search</h2>
        </div>
        <Combobox.Root
          items={visible}
          filter={null}
          autoHighlight
          modal={false}
          value={selected ?? null}
          inputValue={query}
          itemToStringLabel={(trainer) => trainer.name}
          isItemEqualToValue={(a, b) => a.id === b.id}
          onInputValueChange={(value, details) => {
            // Selecting or dismissing a preview must not replace the search.
            if (
              details.reason === "input-change" ||
              details.reason === "input-clear"
            )
              setQuery(value);
          }}
          onValueChange={(trainer) => {
            if (trainer)
              setSelection({ id: trainer.id, searchName: trainer.name });
          }}
        >
          <div className="trainer-search-control" ref={root}>
            <label>
              <span aria-hidden="true">⌕</span>
              <Combobox.Input
                type="search"
                aria-label="Search trainers"
                placeholder="Search trainers…"
              />
            </label>
            <Combobox.Portal>
              <Combobox.Positioner
                anchor={root}
                positionMethod="fixed"
                align="start"
                sideOffset={5}
                collisionPadding={12}
                className="trainer-search-positioner"
              >
                <Combobox.Popup className="trainer-search-dropdown">
                  {visible.length ? (
                    <Combobox.List render={<ul />}>
                      {visible.map((trainer, index) => (
                        <li role="none" key={trainer.id}>
                          <Combobox.Item
                            value={trainer}
                            index={index}
                            nativeButton
                            render={<button />}
                          >
                            <TrainerPortrait
                              trainer={trainer}
                              artwork={artwork}
                            />
                            <span>
                              <strong>{trainer.name}</strong>
                              <small>
                                {[trainer.className, trainer.variant]
                                  .filter(Boolean)
                                  .join(" · ")}
                              </small>
                            </span>
                            <span>Avg Lv{trainer.averageLevel}</span>
                          </Combobox.Item>
                        </li>
                      ))}
                    </Combobox.List>
                  ) : (
                    <p className="trainer-search-message">{resultMessage}</p>
                  )}
                  {result.status === "ready" &&
                    result.data.length > visible.length && (
                      <p className="trainer-result-count">
                        Showing {visible.length} of {result.data.length}. Refine
                        your search to see the rest.
                      </p>
                    )}
                </Combobox.Popup>
              </Combobox.Positioner>
            </Combobox.Portal>
          </div>
        </Combobox.Root>
        {selected ? (
          <div className="selected-trainer-summary">
            <TrainerPortrait trainer={selected} artwork={artwork} />
            <div>
              <strong>{selected.name}</strong>
              <span>
                {[selected.className, selected.variant]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
              <small>
                {selected.team.length} Pokémon · Avg Lv{selected.averageLevel}
              </small>
            </div>
            {selected.warnings.length > 0 && (
              <details>
                <summary>Data notes</summary>
                {selected.warnings.map((warning) => (
                  <p key={warning}>{warning}</p>
                ))}
              </details>
            )}
          </div>
        ) : selection ? (
          <p className="trainer-search-message">
            Selected trainer is unavailable in the current game context.
          </p>
        ) : (
          <p className="trainer-search-message">
            {result.status === "unavailable"
              ? result.reason
              : "Find a trainer to preview their team and details."}
          </p>
        )}
      </section>
      {selected && (
        <PokemonRoster
          party={selected.team}
          artwork={artwork}
          linksFactory={linksFactory}
          opponent
          label={`Avg Lv${selected.averageLevel}`}
          message="This trainer has no available team data."
        />
      )}
    </div>
  );
}

/**
 * The phone player's line under the screen: the party's health, then the
 * active opponents in a battle or the arena's known fields outside one.
 */
function HandheldSummary({
  party,
  opponents,
  arena,
  artwork,
}: {
  party: readonly PokemonPartyMember[];
  opponents: readonly PokemonPartyMember[];
  arena: GameArenaStatus;
  artwork: Artwork;
}) {
  const icons = (members: readonly PokemonPartyMember[], label: string) => (
    <ol className="handheld-party" aria-label={label}>
      {members.map((member) => {
        const image = artwork(member.artwork?.front);
        return (
          <li key={member.slot} data-fainted={member.hp === 0 || undefined}>
            {image.src ? (
              <img src={image.src} alt="" />
            ) : (
              <span className="sprite-missing">?</span>
            )}
            <progress
              className={hpTone(member.hp, member.maxHp)}
              aria-label={`${nameOf(member)} HP ${member.hp} of ${member.maxHp}`}
              value={member.hp}
              max={member.maxHp}
            />
          </li>
        );
      })}
    </ol>
  );
  const known = arena.fields.flatMap((field) =>
    field.value ? [field.value] : [],
  );
  return (
    <div className="handheld-summary">
      {icons(party, "Your team")}
      {opponents.length ? (
        <>
          <span className="handheld-versus">VS</span>
          {icons(opponents, "Opponent")}
        </>
      ) : (
        known.length > 0 && (
          <span className="handheld-arena">{known.join(" · ")}</span>
        )
      )}
    </div>
  );
}

export interface PokemonDashboardProps {
  readonly gameName: string;
  readonly party: BridgeState<PokemonPartyView>;
  readonly storage: BridgeState<PokemonStorageView>;
  readonly battle: BridgeState<PokemonBattleView>;
  readonly context: BridgeState<{ readonly arena: GameArenaStatus }>;
  /** ROM-bound artwork and links; absent until the ROM is prepared. */
  readonly resources:
    | Pick<PokemonResources, "artwork" | "linksFactory">
    | undefined;
  readonly trainerSearch?: PokemonTrainerSearch;
  /** The loaded build's provenance; a located build is marked as unreviewed. */
  readonly build?: BuildInfo;
  readonly children: ReactNode;
  /** On phones: hand the HUD to the player instead of laying out the page. */
  readonly handheld?: GameLayoutProps["handheld"];
  /** On narrower pages: hand the panels to the player's dock instead. */
  readonly docked?: GameLayoutProps["docked"];
}

export function PokemonDashboard({
  gameName,
  party,
  storage,
  battle,
  context,
  resources,
  trainerSearch,
  build,
  children,
  handheld,
  docked,
}: PokemonDashboardProps) {
  const [mobilePanel, setMobilePanel] = useState<"team" | "info">("team");
  const panelId = useId();
  const artwork = usePokemonArtwork(resources);
  const combat =
    battle.status === "ready" && battle.data.active ? battle.data : undefined;
  const members =
    combat?.playerParty ?? (party.status === "ready" ? party.data.party : []);
  const battleParty = (
    side: "player" | "opponent",
    members: readonly PokemonPartyMember[],
  ) =>
    members.map((member) => {
      const live = combat?.battlers.find(
        (battler) => battler.side === side && battler.partySlot === member.slot,
      );
      return live ? { ...member, ...live, slot: member.slot } : member;
    });
  const stored =
    storage.status === "ready"
      ? storage.data.boxes.flatMap((box) => box.pokemon)
      : [];
  const timeline = "sample" in party ? party.sample?.timeline : party.status;
  const battleTimeline =
    "sample" in battle ? battle.sample?.timeline : battle.status;
  const message =
    party.status === "unavailable"
      ? party.reason
      : party.status === "error"
        ? party.message
        : party.status === "loading"
          ? "Reading your team…"
          : undefined;
  const battleLabel = combat
    ? `${combat.kind[0].toUpperCase()}${combat.kind.slice(1)} battle`
    : battle.status === "ready"
      ? "Exploring"
      : "Battle data unavailable";
  const opponentMessage =
    battle.status === "unavailable"
      ? battle.reason
      : battle.status === "error"
        ? battle.message
        : battle.status === "loading"
          ? "Reading battle…"
          : undefined;
  const arena: GameArenaStatus =
    context.status === "ready"
      ? context.data.arena
      : {
          title: gameName,
          fields: [
            {
              id: "context",
              label: "Arena",
              value: null,
              reason:
                context.status === "unavailable"
                  ? context.reason
                  : context.status === "error"
                    ? context.message
                    : "Waiting for game context",
            },
          ],
        };
  const playerSlots = combat?.battlers
    .filter((b) => b.side === "player")
    .map((b) => b.partySlot);
  const opponentSlots = combat?.battlers
    .filter((b) => b.side === "opponent")
    .map((b) => b.partySlot);
  const opponents = battleParty("opponent", combat?.opponentParty ?? []);
  const team = (
    <PokemonRoster
      key={`team:${timeline}`}
      party={battleParty("player", members)}
      stored={stored}
      artwork={artwork}
      linksFactory={resources?.linksFactory}
      label={
        storage.status === "ready"
          ? `${members.length + stored.length} owned`
          : `${members.length} in party`
      }
      message={message}
      activeSlots={playerSlots}
    />
  );
  const info =
    battle.status === "ready" && !battle.data.active && trainerSearch ? (
      <TrainerSearch
        key={`trainer:${battleTimeline}`}
        search={trainerSearch}
        artwork={artwork}
        linksFactory={resources?.linksFactory}
      />
    ) : (
      <PokemonRoster
        key={`opponent:${timeline}`}
        party={opponents}
        artwork={artwork}
        linksFactory={resources?.linksFactory}
        opponent
        label={
          combat
            ? battleLabel
            : opponentMessage
              ? "Unavailable"
              : "Not in battle"
        }
        message={opponentMessage}
        activeSlots={opponentSlots}
      />
    );
  if (handheld)
    return handheld({
      summary: (
        <HandheldSummary
          party={battleParty("player", members)}
          opponents={opponents.filter((member) =>
            opponentSlots?.includes(member.slot),
          )}
          arena={arena}
          artwork={artwork}
        />
      ),
      panels: [
        { id: "team", label: "Team", content: team },
        {
          id: "info",
          label: combat ? "Battle" : "Info",
          content: (
            <>
              <ArenaStatus build={build} arena={arena} />
              {info}
            </>
          ),
        },
      ],
    });
  // Beside the screen, the battle opens when one starts and the team when it
  // ends; the arena line stays above the screen.
  if (docked)
    return docked({
      heading: <ArenaStatus build={build} arena={arena} />,
      panels: [
        { id: "team", label: "Team", content: team },
        { id: "info", label: combat ? "Battle" : "Info", content: info },
      ],
      focus: combat ? "info" : "team",
    });
  return (
    <main
      className="adventure-grid"
      data-game-layout="pokemon"
      data-panel={mobilePanel}
    >
      <div className="dashboard-team" id={`${panelId}-team`}>
        {team}
      </div>
      <div className="adventure-center">
        <ArenaStatus build={build} arena={arena} />
        {children}
      </div>
      <nav className="dashboard-panels" aria-label="Game panels">
        <button
          aria-pressed={mobilePanel === "team"}
          aria-controls={`${panelId}-team`}
          onClick={() => setMobilePanel("team")}
        >
          Team
        </button>
        <button
          aria-pressed={mobilePanel === "info"}
          aria-controls={`${panelId}-info`}
          onClick={() => setMobilePanel("info")}
        >
          Info
        </button>
      </nav>
      <div className="dashboard-info" id={`${panelId}-info`}>
        {info}
      </div>
    </main>
  );
}

const idle = { status: "idle" } as const;

/**
 * The Pokémon overlay for any game built on the Gen III views: it reads every
 * view of `game` and renders the dashboard around the player (`children`).
 */
export function Gen3Overlay({
  game,
  gameName,
  build,
  children,
  handheld,
  docked,
}: {
  game: Gen3Game;
  gameName: string;
  build?: BuildInfo;
  children: ReactNode;
  handheld?: GameLayoutProps["handheld"];
  docked?: GameLayoutProps["docked"];
}) {
  const views = useViews(game);
  const media = views?.media.status === "ready" ? views.media.data : undefined;
  const resources = useMemo(
    () =>
      media && {
        artwork: media.artwork,
        linksFactory: media.links ?? undefined,
      },
    [media?.artwork, media?.links],
  );
  const trainers = views?.trainers;
  const trainerReason =
    trainers?.status === "unavailable"
      ? trainers.reason
      : trainers?.status === "error"
        ? trainers.message
        : "Waiting for trainer preview context";
  const trainerSearch = useMemo<PokemonTrainerSearch | undefined>(() => {
    if (!trainers) return undefined;
    if (trainers.status === "ready") return trainers.data;
    return () => ({ status: "unavailable", reason: trainerReason });
  }, [trainers?.status === "ready" ? trainers.data : trainerReason]);
  return (
    <PokemonDashboard
      gameName={gameName}
      party={views?.party ?? idle}
      storage={views?.storage ?? idle}
      battle={views?.battle ?? idle}
      context={views?.context ?? idle}
      resources={resources}
      trainerSearch={trainerSearch}
      build={build}
      handheld={handheld}
      docked={docked}
    >
      {children}
    </PokemonDashboard>
  );
}

export function HeartNSoulDashboard({
  build,
  children,
  handheld,
  docked,
}: GameLayoutProps) {
  return (
    <Gen3Overlay
      game={heartNSoul}
      gameName="Heart & Soul"
      build={build}
      handheld={handheld}
      docked={docked}
    >
      {children}
    </Gen3Overlay>
  );
}

export function EliteReduxDashboard({
  build,
  children,
  handheld,
  docked,
}: GameLayoutProps) {
  return (
    <Gen3Overlay
      game={eliteRedux}
      gameName="Elite Redux"
      build={build}
      handheld={handheld}
      docked={docked}
    >
      {children}
    </Gen3Overlay>
  );
}
