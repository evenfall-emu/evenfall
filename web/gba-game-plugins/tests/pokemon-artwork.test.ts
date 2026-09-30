import { describe, expect, it } from "vite-plus/test";
import { createMemoryReader } from "@evenfall/gba/bridge";
import {
  createPokemonArtwork,
  type PokemonArtworkConfig,
} from "../src/artwork";
import type { PokemonArtworkRef } from "../src/model";

const base = 0x08000000;
const address = (offset: number) => base + offset;
const config: PokemonArtworkConfig = {
  romHash: "fixture-rom",
  speciesCount: 32,
  eggSpecies: 5,
  unownSpecies: 2,
  unownFormsStart: 3,
  iconTable: address(0x0000),
  iconPaletteIndices: address(0x0100),
  iconPalettes: address(0x0200),
  frontTable: address(0x0300),
  palettes: address(0x0500),
  shinyPalettes: address(0x0600),
  modernShinyPalettes: address(0x0700),
  itemCount: 4,
  itemIconTable: address(0x0800),
  spindaSpecies: 6,
  deoxysSpecies: 7,
};

function lzLiterals(input: Uint8Array): Uint8Array {
  const bytes = [
    0x10,
    input.length & 255,
    (input.length >>> 8) & 255,
    input.length >>> 16,
  ];
  for (let offset = 0; offset < input.length; offset += 8) {
    bytes.push(0, ...input.subarray(offset, offset + 8));
  }
  return Uint8Array.from(bytes);
}

function fixture(): {
  bytes: Uint8Array;
  decode: ReturnType<typeof createPokemonArtwork>;
} {
  const bytes = new Uint8Array(0x40000);
  const view = new DataView(bytes.buffer);
  const palette = new Uint8Array(32);
  new DataView(palette.buffer).setUint16(2, 0x001f, true); // red
  new DataView(palette.buffer).setUint16(4, 0x7c00, true); // blue
  new DataView(palette.buffer).setUint16(6, 0x03e0, true); // green
  const palettes = [palette, recolor(0x03e0), recolor(0x7c00)];

  for (let species = 0; species < config.speciesCount; species++) {
    const iconOffset = 0x1000 + species * 0x200;
    view.setUint32(species * 4, address(iconOffset), true);
    bytes[iconOffset] = ((species === 1 ? 2 : species & 15) << 4) | 1;
    bytes[iconOffset + 32] = 3; // First pixel of the tile at x=8.
    bytes[0x0100 + species] = 0;

    const frontOffset = 0x10000 + species * 0x1000;
    const front = new Uint8Array(2048);
    front[0] = 1;
    write(bytes, frontOffset, lzLiterals(front));
    view.setUint32(0x0300 + species * 8, address(frontOffset), true);
    view.setUint16(0x0300 + species * 8 + 4, 2048, true);
  }

  palettes.forEach((colors, index) => {
    const offset = 0x5200 + index * 0x40;
    write(bytes, offset, colors);
    view.setUint32(0x0200 + index * 8, address(offset), true);
  });
  for (let item = 0; item < config.itemCount; item++) {
    const tileOffset = 0x30000 + item * 0x200;
    const paletteOffset = tileOffset + 0x180;
    const tiles = new Uint8Array(288);
    tiles[0] = item === 1 ? 3 : 1;
    tiles[32] = 1; // x=8, y=0.
    tiles[96] = 2; // x=0, y=8: item images have three tiles per row, not four.
    tiles[287] = 0x30; // x=23, y=23, high nibble of the last byte.
    write(bytes, tileOffset, lzLiterals(tiles));
    write(bytes, paletteOffset, lzLiterals(palette));
    view.setUint32(0x0800 + item * 8, address(tileOffset), true);
    view.setUint32(0x0800 + item * 8 + 4, address(paletteOffset), true);
  }
  [0x0500, 0x0600, 0x0700].forEach((tableOffset, tableIndex) => {
    for (let species = 0; species < config.speciesCount; species++) {
      const offset = 0x8000 + tableIndex * 0x1000 + species * 0x40;
      write(bytes, offset, lzLiterals(palettes[tableIndex]));
      view.setUint32(tableOffset + species * 8, address(offset), true);
    }
  });
  const decode = createPokemonArtwork(
    createMemoryReader([{ address: base, bytes }]),
    config,
  );
  return { bytes, decode };
}

function recolor(color: number): Uint8Array {
  const result = new Uint8Array(32);
  new DataView(result.buffer).setUint16(2, color, true);
  return result;
}

function write(target: Uint8Array, offset: number, source: Uint8Array): void {
  target.set(source, offset);
}

function ref(overrides: Partial<PokemonArtworkRef> = {}): PokemonArtworkRef {
  return {
    romHash: config.romHash,
    species: 1,
    personality: 0,
    isEgg: false,
    palette: "normal",
    kind: "icon",
    ...overrides,
  };
}

function pixel(
  image: ReturnType<ReturnType<typeof createPokemonArtwork>>,
  x: number,
  y: number,
) {
  return [
    ...image.pixels.slice(
      (y * image.width + x) * 4,
      (y * image.width + x) * 4 + 4,
    ),
  ];
}

describe("Pokémon artwork", () => {
  it("decodes a trainer's first 64px frame and palette without conflating species or item IDs", () => {
    const { bytes } = fixture();
    const view = new DataView(bytes.buffer);
    const tiles = new Uint8Array(4096);
    tiles[0] = 1;
    tiles[2047] = 0x10;
    tiles.fill(2, 2048); // Animation frame two must not enter the portrait.
    write(bytes, 0x38000, lzLiterals(tiles));
    write(bytes, 0x3a000, lzLiterals(recolor(0x03e0)));
    view.setUint32(0x908, address(0x38000), true);
    view.setUint16(0x90c, 4096, true);
    view.setUint32(0xa08, address(0x3a000), true);
    const decode = createPokemonArtwork(
      createMemoryReader([{ address: base, bytes }]),
      {
        ...config,
        trainerCount: 2,
        trainerFrontTable: address(0x900),
        trainerPalettes: address(0xa00),
      },
    );
    const trainer = {
      kind: "trainer",
      romHash: config.romHash,
      trainerPic: 1,
    } as const;
    const image = decode(trainer);
    expect([image.width, image.height]).toEqual([64, 64]);
    expect(pixel(image, 0, 0)).toEqual([0, 255, 0, 255]);
    expect(pixel(image, 63, 63)).toEqual([0, 255, 0, 255]);
    expect(pixel(image, 1, 0)[3]).toBe(0);
    image.pixels.fill(0);
    expect(pixel(decode(trainer), 0, 0)[3]).toBe(255);
    expect(
      pixel(decode(ref({ kind: "front", personality: null })), 0, 0),
    ).toEqual([255, 0, 0, 255]);
    expect(() => decode(ref({ species: 2, personality: null }))).toThrow(
      /personality/,
    );
    expect(() => decode({ ...trainer, trainerPic: 2 })).toThrow(/trainer/);
    expect(() => decode({ ...trainer, romHash: "other" })).toThrow(
      /hash mismatch/,
    );
  });

  it("maps tiled low/high nibbles and BGR555 red/blue channels", () => {
    const { decode } = fixture();
    const image = decode(ref());
    expect(pixel(image, 0, 0)).toEqual([255, 0, 0, 255]);
    expect(pixel(image, 1, 0)).toEqual([0, 0, 255, 255]);
    expect(pixel(image, 8, 0)).toEqual([0, 255, 0, 255]);
    expect(pixel(image, 2, 0)[3]).toBe(0);
  });

  it("uses species table records and resolves eggs and Unown forms", () => {
    const { decode } = fixture();
    expect(pixel(decode(ref({ species: 9 })), 1, 0)[3]).toBe(255);
    expect(pixel(decode(ref({ species: 9, isEgg: true })), 1, 0)[3]).toBe(255);
    // Personality low bits 2 select form C => formsStart + 1 (species 4).
    expect(pixel(decode(ref({ species: 2, personality: 2 })), 1, 0)[3]).toBe(
      255,
    );
    expect(pixel(decode(ref({ species: 2, personality: 2 })), 1, 0)).toEqual(
      pixel(decode(ref({ species: 4 })), 1, 0),
    );
  });

  it("selects normal, shiny and modern portrait palettes", () => {
    const { decode } = fixture();
    expect(pixel(decode(ref({ kind: "front" })), 0, 0)).toEqual([
      255, 0, 0, 255,
    ]);
    expect(
      pixel(decode(ref({ kind: "front", palette: "shiny" })), 0, 0),
    ).toEqual([0, 255, 0, 255]);
    expect(
      pixel(decode(ref({ kind: "front", palette: "modern-shiny" })), 0, 0),
    ).toEqual([0, 0, 255, 255]);
  });

  it("accepts short front palettes only when every pixel has a color, for each palette kind", () => {
    for (const [palette, offset] of [
      ["normal", 0x8040],
      ["shiny", 0x9040],
      ["modern-shiny", 0xa040],
    ] as const) {
      const { bytes } = fixture();
      const colors = new Uint8Array(28); // Indices 0–13 exist, 14–15 do not.
      new DataView(colors.buffer).setUint16(2, 0x001f, true); // Red at 1.
      new DataView(colors.buffer).setUint16(26, 0x7c00, true); // Blue at 13.
      write(bytes, offset, lzLiterals(colors));
      const front = new Uint8Array(2048);
      front[0] = 0xd1; // Both first pixels use colors that exist.
      front[2047] = 0xe0; // Last pixel uses missing index 14.
      write(bytes, 0x11000, lzLiterals(front));
      const decode = createPokemonArtwork(
        createMemoryReader([{ address: base, bytes }]),
        config,
      );
      const artwork = ref({ kind: "front", palette });
      expect(() => decode(artwork)).toThrow(/palette.*index 14/i);

      front[2047] = 0;
      write(bytes, 0x11000, lzLiterals(front));
      const valid = createPokemonArtwork(
        createMemoryReader([{ address: base, bytes }]),
        config,
      );
      const image = valid(artwork);
      expect(pixel(image, 0, 0)).toEqual([255, 0, 0, 255]);
      expect(pixel(image, 1, 0)).toEqual([0, 0, 255, 255]);
      expect(pixel(image, 63, 63)[3]).toBe(0);
    }
  });

  it("returns owned pixels when serving cached artwork", () => {
    const { decode } = fixture();
    const first = decode(ref());
    first.pixels.fill(99);
    const second = decode(ref());
    expect(second.pixels[0]).toBe(255);
    expect(second.pixels).not.toBe(first.pixels);
  });

  it("decodes item icons with isolated cache keys and owned pixels", () => {
    const { decode } = fixture();
    const creature = decode(ref({ species: 1 }));
    const item = decode({ kind: "item", romHash: config.romHash, item: 1 });
    expect(item).toMatchObject({ width: 24, height: 24 });
    expect(pixel(item, 0, 0)).toEqual([0, 255, 0, 255]);
    expect(pixel(item, 8, 0)).toEqual([255, 0, 0, 255]);
    expect(pixel(item, 0, 8)).toEqual([0, 0, 255, 255]);
    expect(pixel(item, 23, 23)).toEqual([0, 255, 0, 255]);
    expect(pixel(item, 22, 23)[3]).toBe(0);
    expect(pixel(creature, 0, 0)).toEqual([255, 0, 0, 255]);
    item.pixels.fill(99);
    expect(
      pixel(decode({ kind: "item", romHash: config.romHash, item: 1 }), 0, 0),
    ).toEqual([0, 255, 0, 255]);
  });

  it("accepts shortened item palettes but rejects malformed item dimensions and palettes", () => {
    const item = { kind: "item", romHash: config.romHash, item: 1 } as const;
    const { bytes } = fixture();
    write(bytes, 0x30380, lzLiterals(Uint8Array.from([0, 0, 0x1f, 0])));
    const decode = createPokemonArtwork(
      createMemoryReader([{ address: base, bytes }]),
      config,
    );
    const image = decode(item);
    expect(pixel(image, 8, 0)).toEqual([255, 0, 0, 255]);
    expect(pixel(image, 0, 0)).toEqual([0, 0, 0, 255]);
    for (const length of [287, 289]) {
      const { bytes } = fixture();
      write(bytes, 0x30200, lzLiterals(new Uint8Array(length)));
      const decode = createPokemonArtwork(
        createMemoryReader([{ address: base, bytes }]),
        config,
      );
      expect(() => decode(item)).toThrow(/item artwork size/);
    }
    for (const length of [1, 3, 34]) {
      const { bytes } = fixture();
      write(bytes, 0x30380, lzLiterals(new Uint8Array(length)));
      const decode = createPokemonArtwork(
        createMemoryReader([{ address: base, bytes }]),
        config,
      );
      expect(() => decode(item)).toThrow(/item palette size/);
    }
  });

  it("rejects identity, species, invalid icon palettes and unsupported portraits", () => {
    const { bytes, decode } = fixture();
    expect(() => decode(ref({ romHash: "wrong" }))).toThrow(/hash mismatch/);
    expect(() => decode({ kind: "item", romHash: "wrong", item: 1 })).toThrow(
      /hash mismatch/,
    );
    expect(() =>
      decode({ kind: "item", romHash: config.romHash, item: 4 }),
    ).toThrow(/item artwork ID/);
    expect(() => decode(ref({ species: 32 }))).toThrow(/species/);
    expect(() => decode(ref({ kind: "front", species: 6 }))).toThrow(/Spinda/);
    expect(() => decode(ref({ kind: "front", species: 7 }))).toThrow(/Deoxys/);

    bytes[0x0101] = 3;
    const invalid = createPokemonArtwork(
      createMemoryReader([{ address: base, bytes }]),
      config,
    );
    expect(() => invalid(ref())).toThrow(/palette index 3/);
  });

  it("decodes overlapping LZ77 back-references", () => {
    const { bytes } = fixture();
    // Palette: literal bytes 1f 00, then copy those two bytes repeatedly.
    const overlap = Uint8Array.from([
      0x10, 32, 0, 0, 0x30, 0x1f, 0, 0xf0, 0x01, 0x90, 0x01,
    ]);
    write(bytes, 0x8040, overlap);
    const decode = createPokemonArtwork(
      createMemoryReader([{ address: base, bytes }]),
      config,
    );
    expect(pixel(decode(ref({ kind: "front" })), 0, 0)).toEqual([
      255, 0, 0, 255,
    ]);
  });

  it("rejects malicious LZ lengths, back-references and truncated input", () => {
    const cases = [
      Uint8Array.from([0x10, 1, 0, 1]), // More than the 64 KiB cap.
      Uint8Array.from([0x10, 32, 0, 0, 0x80, 0, 0]), // Copy before output.
      Uint8Array.from([0x10, 32, 0, 0, 0]), // Missing literals.
    ];
    for (const malformed of cases) {
      const { bytes } = fixture();
      const offset = bytes.length - malformed.length;
      new DataView(bytes.buffer).setUint32(0x0508, address(offset), true);
      write(bytes, offset, malformed);
      const decode = createPokemonArtwork(
        createMemoryReader([{ address: base, bytes }]),
        config,
      );
      expect(() => decode(ref({ kind: "front" }))).toThrow(
        /LZ77|Uncaptured memory/,
      );
    }
  });
});
