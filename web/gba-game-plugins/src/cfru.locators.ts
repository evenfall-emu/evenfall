// Locators for Complete Fire Red Upgrade (CFRU) hacks, from the reviewed
// Pokémon Unbound 2.1.1.1 ROM (b4776b82a4c7915d0fadeaa27e013523f99dfd94).
//
// CFRU is inserted into a FireRed 1.0 (BPRE) ROM rather than rebuilt from a
// decompilation: FireRed's own code stays where it was, and CFRU repoints the
// tables it expands by rewriting the literal pools and the Game Freak ROM
// header that name them. So every pattern below is FireRed's own code, which
// any CFRU hack keeps, generated with `codeSignatures` and `accessSignatures`
// over the first 0x1e0000 bytes, kept only when unique in the whole ROM and
// free of ROM pointers a hack could repoint. The header locators read the
// pointer fields CFRU and its Pokémon expansion keep up to date.
import {
  content,
  follow,
  offset,
  thumbAccess,
  thumbImmediate,
  thumbLiteral,
  type Locator,
} from "@evenfall/gba/bridge";

/** The reviewed build these locators come from: the template for located builds. */
export const locatedFrom = "b4776b82a4c7915d0fadeaa27e013523f99dfd94";

/** FireRed's Game Freak header names itself "pokemon red version" at 0x08000108. */
const gfHeader = content(
  "70 6f 6b 65 6d 6f 6e 20 72 65 64 20 76 65 72 73 69 6f 6e 00",
);
/** The table pointer at `field` (a ROM address) of the Game Freak header. */
const headerTable = (field: number): Locator => ({
  ...follow(offset(gfHeader, field - 0x08000108)),
  id: `headerTable(0x${field.toString(16)})`,
});

export const locators = {
  gPlayerPartyCount: [
    thumbLiteral("?? 49 09 68 09 7c ?? 4a 12 78 01 3a 12 06 12 0e ?? 4b", 6),
    thumbLiteral("64 22 ?? ?? ?? ?? ?? 49 68 1c 08 70 00 20 ?? ?? 00 00", 6),
    thumbLiteral("00 26 ?? ?? ?? ?? ?? 49 08 78 01 28 ?? ?? 00 25 09 78", 6),
  ],
  gPlayerParty: [
    thumbLiteral("08 40 00 28 ?? ?? ?? 48 3c 18 20 1c 39 21 ?? ?? ?? ??", 6),
    thumbLiteral("02 22 11 43 01 70 ?? 4c fa 20 40 00 25 18 20 1c 03 21", 6),
    thumbLiteral("64 20 4c 46 44 43 ?? 48 24 18 20 1c 41 21 ?? ?? ?? ??", 6),
  ],
  gEnemyParty: [
    thumbLiteral("02 02 84 42 02 02 ?? 48 3c 18 20 1c 39 21 ?? ?? ?? ??", 6),
    thumbLiteral("00 00 84 42 02 02 ?? 48 3c 18 20 1c 39 21 ?? ?? ?? ??", 6),
    thumbLiteral("?? ?? 03 2a ?? ?? ?? 48 21 1c c8 22 ?? ?? ?? ?? ?? ??", 6),
  ],
  gSaveBlock1Ptr: [
    thumbLiteral("?? 48 ?? 49 01 60 ?? 4a ?? 48 10 60 f2 20 00 01 09 18", 6),
    thumbLiteral("01 28 ?? ?? 00 22 ?? 4f c8 21 49 00 6d 18 3e 1c ?? 4b", 6),
    thumbLiteral("08 70 00 22 2b 1c ?? 4e ?? 4d 98 18 31 68 49 19 89 18", 6),
  ],
  gSaveBlock2Ptr: [
    thumbLiteral("?? 48 ?? ?? ?? ?? ?? 48 ?? 49 01 60 ?? 4a ?? 48 10 60", 6),
    thumbLiteral("50 86 13 85 93 85 ?? 48 00 68 c0 7c 02 28 ?? ?? d1 8d", 6),
    thumbLiteral("00 2a ?? ?? ?? 4c ?? 4d 28 68 00 7d 41 07 48 0f 00 19", 6),
  ],
  gPokemonStoragePtr: [
    thumbLiteral("?? 48 08 18 20 60 ?? 4a ?? 48 09 18 11 60 ?? ?? ?? ??", 6),
    thumbLiteral("3a 1c ?? ?? ?? ?? ?? 4e 31 68 ?? 48 80 46 ?? 48 42 46", 6),
    thumbLiteral("00 00 00 b5 01 1c ?? 48 00 68 ?? 4a ?? ?? ?? ?? 01 bc", 6),
  ],
  battleActive: [
    thumbAccess(
      "?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 4d ?? 48 2d 18 28 78 c4 07 e4 0f",
      16,
    ),
    thumbAccess(
      "?? 4d ?? 48 2d 18 28 78 c4 07 e4 0f 01 21 08 43 28 70 ?? ?? ?? ??",
      16,
    ),
    thumbAccess(
      "09 88 ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 ?? 49 40 18 01 78 02 22 11 43",
      16,
    ),
  ],
  gBattleTypeFlags: [
    thumbLiteral("?? ?? 93 42 ?? ?? ?? 4a 10 68 ?? 49 08 40 10 60 ?? 48", 6),
    thumbLiteral("01 bc 00 47 00 b5 ?? 48 00 68 02 21 08 40 00 28 ?? ??", 6),
    thumbLiteral("dc 3d 02 02 30 b5 ?? 48 00 68 02 21 08 40 00 28 ?? ??", 6),
  ],
  gBattlersCount: [
    thumbLiteral("00 28 ?? ?? 00 24 ?? 48 01 78 8c 42 ?? ?? 05 1c 20 06", 6),
    thumbLiteral("03 70 51 60 44 70 ?? 49 02 20 ?? ?? 4c 2b 02 02 ?? 4f", 6),
    thumbLiteral("50 60 01 20 48 70 ?? 49 02 20 ?? ?? ?? 4f 00 03 ?? ??", 6),
  ],
  gBattlerPartyIndexes: [
    thumbLiteral("8c 46 ?? 4f 80 46 ?? 4e 00 24 03 22 91 46 61 46 60 18", 6),
    thumbLiteral("10 bc 01 bc 00 47 ?? 4f 00 03 ?? ?? 02 08 d6 3b 02 02", 6),
    thumbLiteral("?? ?? 00 28 ?? ?? ?? 48 30 18 05 80 ?? ?? 2c 40 02 02", 6),
  ],
  gBattleMons: [
    thumbLiteral("?? 48 80 46 10 32 ?? 48 04 1c 20 34 d1 18 20 78 08 70", 6),
    thumbLiteral("b8 46 4c 46 10 34 ?? 48 02 1c 20 32 e1 18 10 78 08 70", 6),
    thumbLiteral("00 25 64 46 58 23 ?? 4a 18 32 06 21 20 78 58 43 28 18", 6),
  ],
  gAbsentBattlerFlags: [
    thumbLiteral("c1 5f 20 68 09 18 ?? 48 00 78 88 71 19 78 88 00 40 18", 6),
    thumbLiteral("40 18 c0 78 10 70 ?? 4a 26 23 e8 5e 40 18 80 79 10 70", 6),
    thumbLiteral("41 76 ?? 4a 11 70 ?? 48 01 70 ?? 4f 38 68 6c 30 01 70", 6),
  ],
  gMapHeader: [
    thumbLiteral("00 20 81 46 80 46 ?? 49 48 68 00 78 81 45 ?? ?? ?? ??", 6),
    thumbLiteral("?? ?? ?? 4a 92 46 ?? 4b 58 68 41 68 4c 46 60 00 48 44", 6),
    thumbLiteral("56 46 31 68 c9 18 ?? 4a 50 68 40 68 38 18 00 78 8e 25", 6),
  ],
};

export const romLocators = {
  "catalog.speciesNames.address": [
    headerTable(0x08000144),
    thumbLiteral("81 88 0b 20 48 43 ?? 49 40 18 00 78 15 1c ff 28 ?? ??", 6),
    thumbLiteral("81 88 0b 20 48 43 ?? 49 40 18 00 78 90 46 ff 28 ?? ??", 6),
  ],
  "catalog.species.address": [
    headerTable(0x080001bc),
    thumbLiteral("?? ?? 3c 68 7c 34 ?? 49 88 46 c1 00 09 1a 89 00 41 44", 6),
    thumbLiteral("d0 00 80 1a 80 00 ?? 4a 80 18 80 79 21 31 08 70 38 78", 6),
  ],
  "catalog.moveNames.address": [
    headerTable(0x08000148),
    thumbLiteral("21 88 0d 20 41 43 ?? 48 09 18 30 1c ?? ?? ?? ?? e9 1c", 6),
    thumbLiteral("01 1c 0d 20 41 43 ?? 48 09 18 30 1c ?? ?? ?? ?? 30 1c", 6),
  ],
  "catalog.moves.address": [
    headerTable(0x080001cc),
    thumbLiteral("?? 48 00 78 98 72 ?? 4b 21 88 48 00 40 18 80 00 c0 18", 6),
    thumbLiteral("?? 49 40 18 84 89 ?? 4a 60 00 00 19 80 00 84 18 ?? 48", 6),
  ],
  "catalog.abilities.address": [headerTable(0x080001c0)],
  "catalog.abilityDescriptions.address": [headerTable(0x080001c4)],
  "catalog.moveDescriptions.address": [
    thumbLiteral("02 23 ?? ?? ?? ?? ?? 49 68 1e 80 00 40 18 01 68 00 94", 6),
    thumbLiteral("01 91 02 96 03 95 ?? 4b 44 46 21 78 49 00 ?? 4c 12 19", 6),
  ],
  "catalog.types.address": [
    thumbLiteral("92 08 d1 00 89 1a ?? 4a 89 18 ?? ?? ?? ?? ?? 49 ?? ??", 6),
    thumbLiteral("92 08 d1 00 89 1a ?? 4a 89 18 ?? ?? ?? ?? ?? 49 2e 20", 6),
  ],
  "catalog.natures.address": [
    thumbLiteral("00 06 6c 46 16 34 ?? 49 80 0d 40 18 01 68 20 1c ?? ??", 6),
    thumbLiteral("?? ?? 00 06 06 0e ?? 49 b0 00 40 18 01 68 00 20 ?? ??", 6),
  ],
  "catalog.items.address": [
    headerTable(0x080001c8),
    thumbLiteral("10 b5 00 04 00 0c ?? 4c ?? ?? ?? ?? 00 04 00 0c 2c 21", 6),
  ],
  "catalog.experience.address": [
    thumbLiteral("?? ?? ?? ?? 01 90 ?? 4b 01 34 a4 00 ?? 4a e9 00 49 1b", 6),
    thumbLiteral("?? ?? ?? ?? 03 1c ?? 4e a1 00 ?? 4a e8 00 40 1b 80 00", 6),
    thumbLiteral("?? ?? 00 04 00 0c ?? 4b 01 34 a4 00 ?? 4a c1 00 09 1a", 6),
  ],
  "artwork.frontTable": [
    headerTable(0x08000128),
    thumbLiteral("40 00 85 42 ?? ?? ?? 48 00 68 ?? ?? ?? ?? ?? ?? 00 00", 6),
    thumbLiteral("04 1c 0d 1c d0 00 ?? 49 40 18 00 21 84 42 ?? ?? 01 21", 6),
  ],
  "artwork.palettes": [headerTable(0x08000130)],
  "artwork.shinyPalettes": [headerTable(0x08000134)],
  "artwork.modernShinyPalettes": [headerTable(0x08000134)],
  "artwork.iconTable": [
    headerTable(0x08000138),
    thumbLiteral("0b 1c 00 04 00 0c ?? 4a 81 00 89 18 0a 68 c0 46 c0 46", 6),
  ],
  "artwork.iconPaletteIndices": [
    headerTable(0x0800013c),
    thumbLiteral("82 46 56 46 6a 46 ?? 49 71 18 ?? 4b 18 1c 09 78 40 18", 6),
    thumbLiteral("?? 48 09 90 0a 96 ?? 48 20 18 ?? 49 0d 1c 00 78 29 18", 6),
  ],
  "artwork.iconPalettes": [
    headerTable(0x08000140),
    thumbLiteral("00 00 30 b5 00 24 ?? 4d e0 00 40 19 ?? ?? ?? ?? 60 1c", 6),
    thumbLiteral("?? 48 08 18 00 78 ?? 49 c0 00 44 18 a0 88 ?? ?? ?? ??", 6),
  ],
  "artwork.itemIconTable": [
    thumbLiteral("83 42 ?? ?? c0 46 ?? 4a 88 00 d9 00 40 18 80 18 00 68", 6),
  ],
  // CFRU's expanded PC: tables of box and box-name pointers into fixed RAM.
  "storage.boxes": [content("18 93 02 02 e4 99 02 02 b0 a0 02 02 7c a7 02 02")],
  "storage.names": [content("58 16 03 02 61 16 03 02 6a 16 03 02 73 16 03 02")],
  // GetMapName: `idx = mapsec - KANTO_MAPSEC_START`, then `cmp idx, #count - 1`.
  "mapNames.address": [
    thumbLiteral("40 18 01 21 01 80 ?? 48 a4 00 24 18 24 68 20 1c ?? ??", 6),
    thumbLiteral("?? ?? ?? ?? 3e 08 ?? 48 a9 00 09 18 09 68 30 1c ?? ??", 6),
  ],
  "mapNames.count": [
    offset(
      thumbImmediate(
        "70 b5 06 1c 09 04 12 04 14 0c 08 0c ?? 4a 89 18 0d 0c ?? 2d ?? d8",
        18,
      ),
      1,
    ),
  ],
  // IsShinyOtIdPersonality: `cmp shinyValue, #threshold - 1`, then `bhi`.
  shinyThreshold: [
    offset(
      thumbImmediate(
        "02 0c ?? 4b 18 40 42 40 08 0c 42 40 19 40 4a 40 ?? 2a ?? d8",
        16,
      ),
      1,
    ),
  ],
};

/**
 * Bytes only CFRU writes: its removal of FireRed's record encryption (the
 * substructure shuffle and the XOR) and its expanded PC's box table.
 */
export const markers = [
  content("14 0e 00 26 00 20 c0 46 c0 46 17 28"),
  content("10 68 19 68 c0 46 10 60 59 68 c0 46"),
  content("18 93 02 02 e4 99 02 02 b0 a0 02 02 7c a7 02 02"),
];
