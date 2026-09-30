import type { MemoryReader } from "@evenfall/gba/bridge";
import type {
  PokemonArtworkRef,
  PokemonImage,
  PokemonItemArtworkRef,
  PokemonTrainerArtworkRef,
} from "./model";

const MAX_DECODED_BYTES = 64 * 1024;
const CACHE_LIMIT = 64;

/** ROM-specific addresses and species IDs needed by the shared decoder. */
export interface PokemonArtworkConfig {
  readonly romHash: string;
  readonly speciesCount: number;
  readonly eggSpecies: number;
  readonly unownSpecies: number;
  readonly unownFormsStart: number;
  readonly iconTable: number;
  readonly iconPaletteIndices: number;
  readonly iconPalettes: number;
  readonly iconPaletteCount?: number;
  readonly frontTable: number;
  readonly palettes: number;
  readonly shinyPalettes: number;
  readonly modernShinyPalettes: number;
  readonly itemCount: number;
  /** Pairs of LZ77 tile and palette pointers, indexed by item ID. */
  readonly itemIconTable: number;
  readonly trainerCount?: number;
  readonly trainerFrontTable?: number;
  readonly trainerPalettes?: number;
  /** Front artwork for these species needs unverified runtime processing. */
  readonly deoxysSpecies?: number;
  readonly spindaSpecies?: number;
}

/**
 * Prepare an identity-bound, synchronous artwork decoder. Returned pixel
 * arrays are always fresh copies and may safely be mutated by the caller.
 */
export function createPokemonArtwork(
  rom: MemoryReader,
  config: PokemonArtworkConfig,
): (
  ref: PokemonArtworkRef | PokemonItemArtworkRef | PokemonTrainerArtworkRef,
) => PokemonImage {
  validateConfig(config);
  const cache = new Map<string, PokemonImage>();

  return (ref) => {
    if (ref.kind === "item") validateItemRef(ref, config);
    else if (ref.kind === "trainer") {
      if (
        config.trainerCount === undefined ||
        config.trainerFrontTable === undefined ||
        config.trainerPalettes === undefined
      )
        throw new Error("Trainer portraits are not verified for this profile");
      if (
        !Number.isInteger(ref.trainerPic) ||
        ref.trainerPic < 0 ||
        ref.trainerPic >= config.trainerCount
      )
        throw new RangeError("Invalid trainer portrait ID");
    } else validateRef(ref);
    if (ref.romHash !== config.romHash)
      throw new Error(
        `Pokémon artwork ROM hash mismatch: expected ${config.romHash}, received ${ref.romHash}`,
      );
    const species =
      ref.kind === "trainer"
        ? ref.trainerPic
        : ref.kind === "item"
          ? ref.item
          : resolveSpecies(ref, config);
    const key =
      ref.kind === "trainer"
        ? `trainer:${ref.trainerPic}`
        : ref.kind === "item"
          ? `item:${ref.item}`
          : `pokemon:${ref.kind}:${ref.palette}:${species}`;
    const cached = cache.get(key);
    if (cached) {
      cache.delete(key);
      cache.set(key, cached);
      return copyImage(cached);
    }

    const image =
      ref.kind === "trainer"
        ? decodeTrainer(rom, config, ref.trainerPic)
        : ref.kind === "item"
          ? decodeItem(rom, config, ref.item)
          : ref.kind === "icon"
            ? decodeIcon(rom, config, species)
            : decodeFront(rom, config, species, ref.palette);
    cache.set(key, image);
    if (cache.size > CACHE_LIMIT) cache.delete(cache.keys().next().value!);
    return copyImage(image);
  };
}

function decodeTrainer(
  rom: MemoryReader,
  config: PokemonArtworkConfig,
  trainerPic: number,
): PokemonImage {
  const record = entry(config.trainerFrontTable!, trainerPic, 8);
  const size = rom.u16(add(record, 4));
  if (size !== 2048 && size !== 4096)
    throw new RangeError("Invalid trainer portrait size");
  const data = decodeLz77(rom, rom.u32(record));
  if (data.length !== size)
    throw new RangeError("Trainer portrait sheet size mismatch");
  const palette = decodeLz77(
    rom,
    rom.u32(entry(config.trainerPalettes!, trainerPic, 8)),
  );
  if (palette.length !== 32)
    throw new RangeError("Invalid trainer portrait palette");
  return rgbaImage(data.subarray(0, 2048), palette, 64, 64);
}

function decodeItem(
  rom: MemoryReader,
  config: PokemonArtworkConfig,
  item: number,
): PokemonImage {
  const record = entry(config.itemIconTable, item, 8);
  const data = decodeLz77(rom, rom.u32(record));
  if (data.length !== 288)
    throw new RangeError(`Invalid Pokémon item artwork size ${data.length}`);
  const decodedPalette = decodeLz77(rom, rom.u32(add(record, 4)));
  if (
    decodedPalette.length < 2 ||
    decodedPalette.length > 32 ||
    decodedPalette.length % 2
  )
    throw new RangeError(
      `Invalid Pokémon item palette size ${decodedPalette.length}`,
    );
  // Item palettes omit unused trailing colors; GBA decompression targets
  // zeroed palette buffers, so materialize those entries explicitly.
  const palette = new Uint8Array(32);
  palette.set(decodedPalette);
  return rgbaImage(data, palette, 24, 24);
}

function decodeIcon(
  rom: MemoryReader,
  config: PokemonArtworkConfig,
  species: number,
): PokemonImage {
  const data = rom.bytes(rom.u32(entry(config.iconTable, species, 4)), 512);
  const paletteIndex = rom.u8(entry(config.iconPaletteIndices, species, 1));
  if (paletteIndex >= (config.iconPaletteCount ?? 3))
    throw new RangeError(
      `Unsupported Pokémon icon palette index ${paletteIndex}`,
    );
  const paletteRecord = entry(config.iconPalettes, paletteIndex, 8);
  const palette = rom.bytes(rom.u32(paletteRecord), 32);
  return rgbaImage(data, palette, 32, 32);
}

function decodeFront(
  rom: MemoryReader,
  config: PokemonArtworkConfig,
  species: number,
  paletteKind: PokemonArtworkRef["palette"],
): PokemonImage {
  if (species === config.spindaSpecies)
    throw new Error(
      "Spinda front artwork requires runtime spot rendering, which is unsupported",
    );
  if (species === config.deoxysSpecies)
    throw new Error(
      "Deoxys front artwork requires runtime tile processing, which is unsupported",
    );

  const pictureRecord = entry(config.frontTable, species, 8);
  const frameSize = rom.u16(add(pictureRecord, 4));
  if (frameSize < 2048 || frameSize > MAX_DECODED_BYTES)
    throw new RangeError(`Invalid Pokémon front frame size ${frameSize}`);
  const data = decodeLz77(rom, rom.u32(pictureRecord));
  if (data.length < 2048)
    throw new RangeError(`Pokémon front artwork is only ${data.length} bytes`);

  const table =
    paletteKind === "normal"
      ? config.palettes
      : paletteKind === "shiny"
        ? config.shinyPalettes
        : config.modernShinyPalettes;
  const palette = decodeLz77(rom, rom.u32(entry(table, species, 8)));
  return rgbaImage(data.subarray(0, 2048), palette, 64, 64);
}

function decodeLz77(rom: MemoryReader, address: number): Uint8Array {
  if (rom.u8(address) !== 0x10) throw new Error("Invalid GBA LZ77 header");
  const length =
    rom.u8(add(address, 1)) |
    (rom.u8(add(address, 2)) << 8) |
    (rom.u8(add(address, 3)) << 16);
  if (length === 0 || length > MAX_DECODED_BYTES)
    throw new RangeError(`Invalid GBA LZ77 decoded length ${length}`);

  const output = new Uint8Array(length);
  let source = add(address, 4);
  let destination = 0;
  while (destination < length) {
    const flags = rom.u8(source);
    source = add(source, 1);
    for (let bit = 7; bit >= 0 && destination < length; bit--) {
      if ((flags & (1 << bit)) === 0) {
        output[destination++] = rom.u8(source);
        source = add(source, 1);
        continue;
      }
      const first = rom.u8(source);
      const second = rom.u8(add(source, 1));
      source = add(source, 2);
      const count = (first >>> 4) + 3;
      const displacement = ((first & 0x0f) << 8) | second;
      const copyFrom = destination - displacement - 1;
      if (copyFrom < 0)
        throw new RangeError(
          "Invalid GBA LZ77 back-reference before output start",
        );
      if (destination + count > length)
        throw new RangeError("GBA LZ77 back-reference exceeds decoded length");
      for (let i = 0; i < count; i++)
        output[destination++] = output[copyFrom + i];
    }
  }
  return output;
}

function rgbaImage(
  tiles: Uint8Array,
  paletteBytes: Uint8Array,
  width: number,
  height: number,
): PokemonImage {
  const required = (width * height) / 2;
  if (tiles.length < required)
    throw new RangeError(
      `4bpp artwork needs ${required} bytes, received ${tiles.length}`,
    );
  if (paletteBytes.length < 2 || paletteBytes.length % 2 !== 0)
    throw new RangeError(`Invalid Pokémon palette size ${paletteBytes.length}`);

  const colorCount = Math.min(paletteBytes.length / 2, 16);
  const palette = new Uint8Array(colorCount * 4);
  for (let i = 0; i < colorCount; i++) {
    const color = paletteBytes[i * 2] | (paletteBytes[i * 2 + 1] << 8);
    palette[i * 4] = expand5(color & 31);
    palette[i * 4 + 1] = expand5((color >>> 5) & 31);
    palette[i * 4 + 2] = expand5((color >>> 10) & 31);
    palette[i * 4 + 3] = i === 0 ? 0 : 255;
  }

  const pixels = new Uint8ClampedArray(width * height * 4);
  const tilesAcross = width / 8;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const tile = Math.floor(y / 8) * tilesAcross + Math.floor(x / 8);
      const inTile = (y & 7) * 8 + (x & 7);
      const packed = tiles[tile * 32 + (inTile >>> 1)];
      const index = (inTile & 1) === 0 ? packed & 15 : packed >>> 4;
      if (index >= colorCount)
        throw new RangeError(`Pokémon palette is missing color index ${index}`);
      pixels.set(
        palette.subarray(index * 4, index * 4 + 4),
        (y * width + x) * 4,
      );
    }
  }
  return { width, height, pixels };
}

function resolveSpecies(
  ref: PokemonArtworkRef,
  config: PokemonArtworkConfig,
): number {
  if (
    !Number.isInteger(ref.species) ||
    ref.species < 0 ||
    ref.species >= config.speciesCount
  )
    throw new RangeError(`Invalid Pokémon artwork species ${ref.species}`);
  let species = ref.isEgg ? config.eggSpecies : ref.species;
  if (species === config.unownSpecies) {
    if (ref.personality === null)
      throw new Error("Unown artwork requires a known personality");
    const personality = ref.personality >>> 0;
    const form =
      (((personality & 0x03000000) >>> 18) |
        ((personality & 0x00030000) >>> 12) |
        ((personality & 0x00000300) >>> 6) |
        (personality & 3)) %
      28;
    if (form !== 0) species = config.unownFormsStart + form - 1;
  }
  if (species < 0 || species >= config.speciesCount)
    throw new RangeError(
      `Resolved Pokémon artwork species ${species} is out of range`,
    );
  return species;
}

function validateRef(ref: PokemonArtworkRef): void {
  if (
    ref.personality !== null &&
    (!Number.isInteger(ref.personality) ||
      ref.personality < 0 ||
      ref.personality > 0xffffffff)
  )
    throw new RangeError(
      `Invalid Pokémon artwork personality ${ref.personality}`,
    );
  if (typeof ref.isEgg !== "boolean")
    throw new TypeError("Pokémon artwork isEgg must be boolean");
  if (ref.kind !== "icon" && ref.kind !== "front")
    throw new TypeError(`Invalid Pokémon artwork kind ${String(ref.kind)}`);
  if (
    ref.palette !== "normal" &&
    ref.palette !== "shiny" &&
    ref.palette !== "modern-shiny"
  )
    throw new TypeError(
      `Invalid Pokémon artwork palette ${String(ref.palette)}`,
    );
}

function validateItemRef(
  ref: PokemonItemArtworkRef,
  config: PokemonArtworkConfig,
): void {
  if (
    !Number.isInteger(ref.item) ||
    ref.item < 0 ||
    ref.item >= config.itemCount
  )
    throw new RangeError(`Invalid Pokémon item artwork ID ${ref.item}`);
}

function validateConfig(config: PokemonArtworkConfig): void {
  if (!config.romHash)
    throw new Error("Pokémon artwork config requires a ROM hash");
  for (const [name, value] of Object.entries(config)) {
    if (name === "romHash" || value === undefined) continue;
    if (!Number.isInteger(value) || value < 0 || value > 0xffffffff)
      throw new RangeError(`Invalid Pokémon artwork config value ${name}`);
  }
  if (config.speciesCount < 1)
    throw new RangeError("speciesCount must be positive");
  if (config.itemCount < 1) throw new RangeError("itemCount must be positive");
  for (const species of [
    config.eggSpecies,
    config.unownSpecies,
    config.unownFormsStart,
    config.deoxysSpecies,
    config.spindaSpecies,
  ])
    if (species !== undefined && species >= config.speciesCount)
      throw new RangeError("Pokémon artwork config species is out of range");
  if (config.unownFormsStart + 26 >= config.speciesCount)
    throw new RangeError(
      "Pokémon artwork config does not contain all Unown forms",
    );
}

function entry(table: number, index: number, stride: number): number {
  return add(table, index * stride);
}

function add(address: number, offset: number): number {
  const result = address + offset;
  if (!Number.isSafeInteger(result) || result < 0 || result > 0xffffffff)
    throw new RangeError("Pokémon artwork guest address overflow");
  return result;
}

function expand5(value: number): number {
  return (value << 3) | (value >>> 2);
}

function copyImage(image: PokemonImage): PokemonImage {
  return {
    width: image.width,
    height: image.height,
    pixels: image.pixels.slice(),
  };
}
