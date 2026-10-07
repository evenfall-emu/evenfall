# @evenfall/gba-game-plugins

Game plugins for the [Evenfall player](../../docs/player.md). They read a
running game's memory and show what it holds beside the screen. Today the
package holds unofficial Pokémon panels: the current party, PC storage, battle
state, trainer previews and species details. It is also a complete example of
a game package built on the SDK's [game bridges](../../docs/game-bridges.md).

## Disclaimer

> **Evenfall and this package are not affiliated with, endorsed by, or
> sponsored by Nintendo, Creatures Inc., GAME FREAK inc. or The Pokémon
> Company.** Pokémon and Pokémon character names are trademarks of Nintendo,
> Creatures Inc. and GAME FREAK inc. The ROM hacks named below are
> third-party works by their own authors and are not part of Evenfall. This
> package contains no game code, data or artwork: you must supply your own
> legally obtained ROMs.

## What it does

When the player loads a supported ROM, this package decodes its data **at
runtime from the user's own ROM and emulated memory**:

- party, PC boxes and active battlers: species, nickname, level, HP,
  ailments, nature, held item, abilities, moves with PP, stats with IVs, EVs
  and battle stages;
- species, move, ability and item names and descriptions, read from the ROM's
  own tables (the "dex" data);
- Pokémon, item and trainer sprites, decompressed from the ROM's graphics;
- for Elite Redux, the difficulty-scaled teams of trainers, for previewing
  upcoming battles.

Nothing is bundled: no names, statistics, tables, sprites or other assets from
any game are in this repository or its builds. Everything is read from the
loaded ROM and discarded with the session. The one exception is generic
mechanics: the standard type-effectiveness chart used to rate moves in
battle ([`src/type-system.ts`](src/type-system.ts)), which a game can
override. Observation is read-only; the package never writes game memory.

## Supported builds

Reviewed support is per **exact build**, identified by the SHA-1 of the raw
ROM (the `.gba`, or the single ROM inside a ZIP). Title and header codes are
not enough.

| Game                            | Build                  | Raw ROM SHA-1                              | Support               |
| ------------------------------- | ---------------------- | ------------------------------------------ | --------------------- |
| Pokémon Heart & Soul (ROM hack) | 1.2.1 release          | `a4be81f8330d7f223a3ab847c3da822fe6f8a2e0` | reviewed manifest     |
| Pokémon Heart & Soul (ROM hack) | 1.2.0 release          | `b8a4b491ca67a76f079a8e7c19fd5512a2147dd2` | located in the player |
| Pokémon Elite Redux (ROM hack)  | 2.65 Beta2             | `f1323126bc043f3ce931bcc670957930559eb515` | reviewed manifest     |
| Pokémon Elite Redux (ROM hack)  | 2.65 Beta2 debug build | `299165fb56e19ac8afe5d66f83dd5bada823d58c` | located in the player |
| Pokémon Elite Redux (ROM hack)  | 2.65 Beta3 (2.65.3b)   | `5b38a681b6924ef3e0bb9f6cfbf22426c4c4e523` | reviewed manifest     |
| Pokémon Elite Redux (ROM hack)  | 2.65.4.1b              | `2f3e17e55c6afe23da97ec0f96a067eeb142ad15` | located in the player |

Both games carry shared locators seeded from their reviewed builds.
When the player loads another build, it runs them.
A build is accepted only if every address is found without disagreement and the ROM probes pass.
Its dashboard is marked as not reviewed.
Redux also locates its ability record size and suppression flags offset from code, so those values need not match the template.
Other record layouts still come from the template.
Other revisions, such as Elite Redux 2.65 Beta1, are recognized only if their locators and probes still match.
Other pokeemerald-expansion hacks and retail Pokémon games have no family, and
unknown ROMs keep the generic player.
[`docs/profiles.md`](docs/profiles.md) records the evidence behind every
address and layout, and what remains unavailable.

## In the player

The player's [game-plugin registry](../../docs/player.md#game-plugins)
installs this package through one import in
[`web/player/src/games.ts`](../player/src/games.ts):

```ts
import { pokemonPlugin } from "@evenfall/gba-game-plugins/ui";
const installed: readonly GamePlugin[] = [pokemonPlugin];
```

`pokemonPlugin` lists the headless game definitions and lazily loads their
dashboards with their styles ([`ui/dashboards.ts`](ui/dashboards.ts),
[`ui/dashboard.css`](ui/dashboard.css)). Removing the import, the array entry
and the `@evenfall/gba-game-plugins` dependency in `web/player/package.json` builds a
player without any Pokémon code.

Opening a supported ROM selects the three-column dashboard automatically:
team and PC roster on the left, the player in the center, and the selected
Pokémon's inspector on the right. On landscape pages narrower than 17:10 the
player's [dock](../../docs/player.md#narrower-screens) holds the Team and
Info (Battle) panels as tabs beside the screen, with the arena line above
it; a battle opens the Battle tab and its end the Team tab. On phones the
player's [phone layout](../../docs/player.md#phones) shows the party's
icons and HP under the screen, with the active opponents in a battle or the
arena's known fields outside one, and opens the Team and Info (Battle)
panels in a sheet. Team cards support click and Arrow/Home/End;
selection follows identity through reordering and resets on a new timeline,
and search includes PC storage, ignoring case, accents and punctuation. While
the game runs, observations refresh up
to 20 times a second; pausing keeps the last reading, and loading a state
refreshes it.

- The inspector shows names, ROM portraits, types, abilities (one or many,
  including locked innates, persistent suppression and extra Hell-mode
  abilities), nature, held item with its ROM sprite, bounded descriptions,
  stats with IVs/EVs, move category, power, priority and PP, friendship,
  experience, HP and ailments. Hover, focus or tap an ability, move or item
  for its details; Escape or an outside tap dismisses the tooltip.
- In battle, active battlers supply live HP, stats, moves, items, types and
  abilities, and the stat totals use the bridge's `stageAdjustedStats`, with
  an arrow on boosted or lowered stats. These are stage-adjusted, not a full
  damage calculation: ability, item, weather and move modifiers are excluded.
- In battle, each damaging move is rated by type against the opposing side's
  active battlers, for your Pokémon and the opponent's alike, including
  benched ones: a black row for no effect, strong red for ×¼, light red for
  ×½, light green for ×2 and strong green for ×4, with the multiplier beside
  the name. In a double battle a move shows its best result among the
  standing foes, and its tooltip names that foe. Status moves are not rated.
  This is the type chart alone: abilities such as Levitate, items, weather
  and type-changing moves are excluded.
- Condition colors are interface conventions, not ROM data: burn is orange,
  paralysis yellow, freeze ice-blue, bleed red, sleep white, poison and
  unknown ailments purple.
- Elite Redux adds trainer previews for inactive battles and Pokédex links
  for the selected Pokémon. Its trainer search matches names, classes and
  encounter labels without case, accents or punctuation.

Move-category icons are original inline SVG drawn for this package. Values the
bridges cannot verify stay visibly unavailable with a reason: randomized
party types and abilities, turn count, live battle weather, most map names
and some rare shiny palettes. Spinda and Deoxys portraits and Japanese text
are unsupported. PC records do not keep current PP, so the
inspector shows a dash.

## As a library

`@evenfall/gba-game-plugins` exports the headless game definitions and decoders
([`src/`](src/)); `@evenfall/gba-game-plugins/ui` exports the plugin and its React
panels ([`ui/`](ui/)). Both are TypeScript sources consumed through npm
workspaces; the package is private and not published to npm.

```ts
import { BridgeHost } from "@evenfall/gba/bridge";
import { heartNSoul } from "@evenfall/gba-game-plugins";

const store = new BridgeHost(emulator).observe(heartNSoul);
store.subscribe(() => {
  const party = store.getSnapshot().party;
  if (party.status === "ready") console.log(party.data.party.length);
}, "party");
```

Games are families over the shared Gen III views
([`src/gen3.ts`](src/gen3.ts)): `pokemonGen3(format)` supplies the standard
sources, views and resources, Heart & Soul spreads it with its layout and map
context, and Elite Redux defines its own sources and views for the same
overlay. A build is a manifest; `createGame(family, manifests)` registers
further verified builds.

Types are keys such as `fire` or `electric`, parsed from the ROM's type
names ignoring case and punctuation. Without a pattern of its own, a name
matches a key it abbreviates: the same first letter, then some of the key's
letters in order, at least three in all. Truncations such as `ELECTR` and
`FIGHT` and contractions such as `PSYCHC` or `GRND` therefore parse without
configuration, so ROM hacks that keep the standard types work unchanged; a
name that could abbreviate two types, such as `FIR`, stays unmapped. Badge
colors and move ratings both follow the game's `typeSystem` resource, which
defaults to the standard eighteen types and the Generation VI chart. A game
whose types differ supplies its own:

```ts
import {
  extendTypeSystem,
  standardTypeDefinition,
} from "@evenfall/gba-game-plugins";

const typeSystem = extendTypeSystem(standardTypeDefinition, {
  types: {
    sound: { color: "#7fc8d8" }, // a type the stylesheet lacks
    fire: { pattern: /^(fire|flame)$/ }, // a name that is not an abbreviation
  },
  ids: { 9: null }, // type ID 9 (`???`) has no matchups
  chart: { sound: { ghost: 0 }, fire: { fairy: 1 } }, // rows merge per attacker
});
```

Types the system does not map show a plain badge and are not rated; as a
defender they count as neutral. See the
[Elite Redux profile](docs/profiles.md#elite-redux-265-beta2).

## Tests

```sh
vp check                                  # TypeScript, including bridge.typecheck.tsx
vp test --project gba-game-plugins             # decoders, catalogs, artwork, panels
npm run build -w @evenfall/player         # then, against the built player:
npm run test:browser -w @evenfall/gba-game-plugins
```

Unit tests use synthetic records. Tests that need real games skip unless you
point these variables at files you supply; they verify the documented hashes
before use:

| Variable                                             | File                                                                                              |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `EVENFALL_HNS_ROM`                                   | Heart & Soul 1.2.1 raw ROM                                                                        |
| `EVENFALL_HNS_120_ROM`                               | Heart & Soul 1.2.0 raw ROM, with `EVENFALL_HNS_ROM`                                               |
| `EVENFALL_HNS_STATE`, `EVENFALL_HNS_BATTLE`          | `game2.state` and `battle.state` ([hashes](docs/profiles.md#test-fixtures))                       |
| `EVENFALL_REDUX_ROM`                                 | Elite Redux 2.65 Beta2 raw ROM                                                                    |
| `EVENFALL_REDUX_DEBUG_ROM`                           | Elite Redux 2.65 Beta2 debug build raw ROM, with `EVENFALL_REDUX_ROM`                             |
| `EVENFALL_REDUX_STATE`, `EVENFALL_REDUX_STAGE_STATE` | `redux.state` and `redux-stage-buff.state` ([hashes](docs/profiles.md#test-fixtures))             |
| `EVENFALL_REDUX_BETA3_ROM`                           | Elite Redux 2.65 Beta3 raw ROM                                                                    |
| `EVENFALL_REDUX_BETA3_STATE`                         | a Beta3 overworld state, with `EVENFALL_REDUX_BETA3_ROM` ([hash](docs/profiles.md#test-fixtures)) |
| `EVENFALL_REDUX_26541_ROM`                           | Elite Redux 2.65.4.1b raw ROM                                                                     |

No ROM, save or state is committed, and none is needed for the default suites.
The state fixtures above were captured with the player's previous core and
start with `ZIGBAST1`, which the current core cannot load: the tests that load
them need the same situations captured again as Evenfall states, with their
new hashes recorded in the profiles.

## License

AGPL-3.0-or-later, like the rest of Evenfall. See the
[disclaimer](#disclaimer) above and the repository's
[third-party notices](../../THIRD_PARTY_NOTICES.md).
