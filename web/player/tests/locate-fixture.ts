// Synthetic ROMs and a locatable family shared by the locator and registry tests.
import {
  content,
  createGame,
  defineFamily,
  ram,
  thumbLiteral,
  u8,
  type ManifestOf,
  type RomContext,
} from "@evenfall/gba/bridge";

/** Deterministic noise, so every 16-byte window of a fixture ROM is unique. */
export function noise(length: number, seed: number): Uint8Array {
  const bytes = new Uint8Array(length);
  let state = seed >>> 0 || 1;
  for (let i = 0; i < length; i++) {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    bytes[i] = state & 0xff;
  }
  return bytes;
}

/** A small Thumb function that loads `address` with `ldr r3, [pc, #0]`. */
export function thumbLoader(address: number, filler: number): Uint8Array {
  const bytes = new Uint8Array(16);
  // push {r4, lr}; movs r1, #filler; ldr r3, [pc, #0]; ldrb r3, [r3]
  bytes.set([0x10, 0xb5, filler, 0x21, 0x00, 0x4b, 0x1b, 0x78]);
  new DataView(bytes.buffer).setUint32(8, address, true); // the literal pool
  bytes.set([0x10, 0xbd, 0xc0, 0x46], 12); // pop {r4, pc}; nop
  return bytes;
}

/**
 * A build of the runtime family: its game code, a function loading the
 * `count` address, and the names table, both moved by `shift` bytes.
 */
export function runtimeRom(
  count: number,
  shift: number,
  gameCode = "BPEE",
): Uint8Array {
  const rom = noise(0x800, 23);
  rom.set(
    Array.from(gameCode, (c) => c.charCodeAt(0)),
    0xac,
  );
  rom.set(thumbLoader(count, 0x07), 0x200 + shift);
  rom.set([0xde, 0xad, 0xbe, 0xef, 1, 2, 3, 4], 0x400 + shift);
  return rom;
}

export const reviewedHash = "e".repeat(40);

interface RuntimeRom {
  readonly names: number;
}

export const runtimeFamily = defineFamily<{ count: number }, RuntimeRom>()({
  id: "example/runtime",
  sources: {
    count: ram(u8, { locate: [thumbLiteral("10 b5 ?? 21 ?? 4b 1b 78", 4)] }),
  },
  resources: ({ rom, config }: RomContext<RuntimeRom>) => ({
    firstName: () => rom.u8(config.names),
  }),
  views: {
    count: ({ memory, symbols }) => ({
      status: "ready",
      data: memory.read(symbols.count),
    }),
  },
  probes: {
    names: ({ firstName }) => {
      if (firstName() !== 0xde) throw new Error("names table not found");
    },
  },
  romLocators: { names: [content("de ad be ef")] },
  detect: { gameCodes: ["BPEE"] },
  locatedFrom: reviewedHash,
});

export const runtimeTemplate: ManifestOf<typeof runtimeFamily> = {
  family: "example/runtime",
  romHash: reviewedHash,
  source: {
    repository: "synthetic",
    revision: "fixture",
    variant: "test",
    compiler: "none",
    options: [],
  },
  addresses: { count: 0x02000000 },
  rom: { names: 0x08000400 },
  provenance: { method: "reviewed" },
};

/** A fresh game each call: located builds register on the game object. */
export const runtimeGame = (family = runtimeFamily) =>
  createGame(family, [runtimeTemplate]);
