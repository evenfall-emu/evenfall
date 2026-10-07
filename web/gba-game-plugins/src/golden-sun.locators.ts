// Generated with the SDK's codeSignatures and accessSignatures: masked Thumb
// code that loads or computes each RAM root, from two verified ROMs. The two
// games compile the same routines differently, so each list holds both games'
// signatures, and each finds nothing in the other game. Regenerate rather than
// editing by hand.
import { thumbAccess, thumbLiteral, type Locator } from "@evenfall/gba/bridge";

/** The reviewed build these locators come from: the template for located builds. */
export const locatedFrom = "1864e0f1ae44e07f10c2d0ac826e6104c69ca5e9";

// From Golden Age 1 (1864e0f1ae44e07f10c2d0ac826e6104c69ca5e9), a patch of
// Golden Sun (USA, Europe).
const goldenSun = {
  // getUnit(id) at 0x08077394 (ids 0-7) and the new-game initializer.
  party: [
    thumbLiteral("00 02 00 b5 73 46 ?? 4a 07 28 ?? ?? a6 23 5b 00 43 43", 6),
    thumbLiteral("28 1c 2c 60 ?? 4b ?? 49 ?? 4a 07 c3 0c 3b 00 94 ?? ??", 6),
  ],
  partyFlags: [
    thumbLiteral("?? 4b f2 21 1b 68 ?? 4a 00 25 89 00 fb 86 ?? ?? 08 ca", 6),
    thumbLiteral("2c 60 ?? 4b 28 1c ?? 49 ?? 4a 07 c3 0c 3b ?? 4b 82 21", 6),
    thumbLiteral("9a 40 03 05 d8 0d ?? 4b 1b 5c 13 40 58 42 18 43 c0 0f", 6),
  ],
  partyOrder: [
    thumbLiteral("5b 46 40 46 03 80 ?? 48 90 44 03 88 41 46 0b 80 43 88", 6),
    thumbAccess(
      "6a 46 ?? 4b fc 24 28 32 64 00 04 92 11 1c 1a 19 13 78 0b 80 01 20",
      16,
    ),
    thumbAccess(
      "?? ?? be 42 ?? ?? ?? 4b fc 21 49 00 5a 18 3d 1c 10 78 01 32 00 92",
      16,
    ),
  ],
  // No literal names the coins: code adds 0x10 to the 0x02000240 block.
  coins: [
    thumbAccess(
      "?? ?? ?? ?? 8d 21 02 20 ?? ?? ?? ?? ?? 4a 96 21 13 69 49 00 5b 18",
      16,
    ),
    thumbAccess(
      "02 20 ?? ?? ?? ?? ?? 4a 96 21 13 69 49 00 5b 18 13 61 60 bc 01 bc",
      16,
    ),
  ],
  // getUnit(id) for ids 0x80-0x85, and a battle setup routine.
  enemies: [
    thumbLiteral("80 3b 05 2b ?? ?? ?? 4b 1a 68 00 2a ?? ?? a6 23 5b 00", 6),
    thumbLiteral("9b 0f 50 32 13 70 ?? 4b ?? 4c 1a 68 ?? 49 00 20 0b 78", 6),
    thumbAccess(
      "43 43 98 18 ?? ?? 03 1c 80 3b 05 2b ?? ?? ?? 4b 1a 68 00 2a ?? ??",
      16,
    ),
  ],
} satisfies Record<string, Locator[]>;

// From Golden Sun: The Lost Age (USA, Europe),
// b500663220cb9bf56b9f8e8c0c544f1d6fa3a824.
const theLostAge = {
  // getUnit(id) at 0x08016CA4 and the new-game initializer.
  party: [
    thumbLiteral("?? ?? a6 23 5b 00 ?? 4a 43 43 98 18 ?? ?? 03 1c 80 3b", 6),
    thumbLiteral("91 46 d4 33 28 1c ?? 49 ?? 4a 07 c3 0c 3b ?? ?? ?? ??", 6),
  ],
  partyFlags: [
    thumbLiteral("07 22 02 40 d8 0d ?? 4b 18 5c 01 23 10 41 18 40 70 47", 6),
    thumbLiteral("00 02 07 23 03 40 ?? 49 01 22 9a 40 03 05 d8 0d 0b 5c", 6),
  ],
  partyOrder: [
    thumbLiteral("08 80 40 46 03 80 ?? 48 90 44 03 88 41 46 0b 80 42 46", 6),
    thumbAccess(
      "?? 4b 6a 46 86 24 24 32 a4 00 03 92 11 1c 1a 19 13 78 01 37 0b 80",
      16,
    ),
  ],
  coins: [
    thumbAccess(
      "02 21 ?? ?? ?? ?? ?? 49 a3 23 9b 00 ca 18 12 68 0b 69 04 b0 9b 18",
      16,
    ),
    thumbAccess(
      "?? 49 a3 23 9b 00 ca 18 12 68 0b 69 04 b0 9b 18 0b 61 28 bc 98 46",
      16,
    ),
  ],
  // getUnit builds 0x030000D8 from immediates rather than a literal.
  enemies: [
    thumbAccess(
      "?? ?? 03 1c 80 3b 05 2b ?? ?? c0 23 9b 04 d8 33 1a 68 00 2a ?? ??",
      16,
    ),
    thumbAccess(
      "50 32 9b 06 9b 0f 13 70 c0 23 9b 04 d8 33 d9 24 1a 68 ?? 49 e4 00",
      16,
    ),
  ],
} satisfies Record<string, Locator[]>;

export const locators = {
  party: [...goldenSun.party, ...theLostAge.party],
  partyFlags: [...goldenSun.partyFlags, ...theLostAge.partyFlags],
  partyOrder: [...goldenSun.partyOrder, ...theLostAge.partyOrder],
  coins: [...goldenSun.coins, ...theLostAge.coins],
  enemies: [...goldenSun.enemies, ...theLostAge.enemies],
};

/** Finds what either locator finds: one detection marker for both games. */
function either(a: Locator, b: Locator): Locator {
  return {
    id: `either(${a.id}, ${b.id})`,
    find: (rom) => [...new Set([...a.find(rom), ...b.find(rom)])],
  };
}

/** Both games' `getUnit`: the party base and the enemy pointer. */
export const markers = [
  either(goldenSun.party[0], theLostAge.party[0]),
  either(goldenSun.enemies[0], theLostAge.enemies[0]),
];
