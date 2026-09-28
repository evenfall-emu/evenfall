import { describe, expect, it } from "vite-plus/test";
import {
  checkManifest,
  content,
  createGame,
  createMemoryReader,
  createOffsetMap,
  decodeGame,
  defineFamily,
  locateSources,
  locateWith,
  ram,
  ramArray,
  relocateManifest,
  codeSignatures,
  dataSignatures,
  detectFamily,
  follow,
  inferred,
  offset,
  pointerSignatures,
  references,
  accessSignatures,
  thumbAccess,
  thumbImmediate,
  valueSignatures,
  locateBuild,
  manifestFromLocated,
  thumbLiteral,
  u8,
  buildProvenance,
  locatable,
  locateGame,
  registerBuild,
  resolveGame,
  unregisterBuild,
  type ManifestOf,
  type MemoryLayout,
  type RomContext,
} from "@evenfall/gba/bridge";
import {
  noise,
  reviewedHash,
  runtimeFamily,
  runtimeGame,
  runtimeRom,
  runtimeTemplate,
  thumbLoader,
} from "./locate-fixture";

interface Views {
  readonly party: { readonly levels: readonly number[] };
  readonly lookup: (id: number) => string;
  readonly enemy?: { readonly count: number };
}
interface Rom {
  readonly names: number;
}
const level: MemoryLayout<number> = {
  byteLength: 4,
  decode: (memory, address) => memory.u8(address + 2),
};
const family = defineFamily<Views, Rom>()({
  id: "example/family",
  sources: { count: ram(u8), party: ramArray(level, 3) },
  resources: ({ rom, config }: RomContext<Rom>) => ({
    name: (id: number) => `name-${rom.u8(config.names + id)}`,
  }),
  views: {
    party: ({ memory, symbols }) => ({
      status: "ready",
      data: {
        levels: Array.from({ length: memory.read(symbols.count) }, (_, i) =>
          memory.read({
            address: symbols.party.address + i * 4,
            layout: level,
          }),
        ),
      },
    }),
    lookup: ({ resources }) => ({ status: "ready", data: resources.name }),
  },
});
const manifest: ManifestOf<typeof family> = {
  family: "example/family",
  romHash: "a".repeat(40),
  source: {
    repository: "synthetic",
    revision: "fixture",
    variant: "test",
    compiler: "none",
    options: [],
  },
  addresses: { count: 0x02000000, party: 0x02000004 },
  rom: { names: 0x08000010 },
  provenance: { method: "reviewed" },
};

describe("game families", () => {
  const game = createGame(family, [manifest]);
  const memory = createMemoryReader([
    {
      address: 0x02000000,
      bytes: new Uint8Array([2, 0, 0, 0, 0, 0, 7, 0, 0, 0, 9, 0]),
    },
  ]);
  const rom = createMemoryReader([
    { address: 0x08000000, bytes: new Uint8Array(0x20).fill(5) },
  ]);
  const sample = {
    romHash: manifest.romHash,
    timeline: "t",
    sequence: 1,
    cycle: 0n,
  };

  it("compiles to a game whose views are bridges at the manifest's addresses", () => {
    expect(Object.keys(game.bridges).sort()).toEqual(["lookup", "party"]);
    const resources = game.prepare!({ rom, romHash: manifest.romHash });
    const states = decodeGame(game, sample, memory, resources);
    expect(states.party).toMatchObject({
      status: "ready",
      data: { levels: [7, 9] },
    });
    expect(Object.isFrozen(states.party)).toBe(true);
  });

  it("passes functions in view data through by reference", () => {
    const resources = game.prepare!({ rom, romHash: manifest.romHash });
    const first = decodeGame(game, sample, memory, resources).lookup;
    const second = decodeGame(
      game,
      { ...sample, sequence: 2 },
      memory,
      resources,
    ).lookup;
    if (first.status !== "ready" || second.status !== "ready")
      throw new Error("not ready");
    expect(first.data(1)).toBe("name-5");
    expect(second.data).toBe(first.data);
  });

  it("rejects manifests that do not describe the family's sources", () => {
    expect(() =>
      checkManifest(family, { ...manifest, family: "other" }),
    ).toThrow("targets other");
    expect(() =>
      checkManifest(family, { ...manifest, romHash: "A".repeat(40) }),
    ).toThrow("lowercase SHA-1");
    const { party: _missing, ...partial } = manifest.addresses;
    expect(() =>
      checkManifest(family, {
        ...manifest,
        addresses: partial as typeof manifest.addresses,
      }),
    ).toThrow("missing address for party");
    expect(() =>
      checkManifest(family, {
        ...manifest,
        addresses: {
          ...manifest.addresses,
          typo: 1,
        } as typeof manifest.addresses,
      }),
    ).toThrow("unknown source typo");
    expect(() => game.prepare!({ rom, romHash: "b".repeat(40) })).toThrow(
      "No example/family manifest",
    );
  });

  it("adds locators to named sources only", () => {
    const locator = content("aa bb");
    const sources = locateWith(family.sources, { party: [locator] });
    expect(sources.party.locate).toEqual([locator]);
    expect(sources.count.locate).toBeUndefined();
  });
});

describe("locators", () => {
  it("reads the literal a Thumb load names, and finds data by content", () => {
    const rom = new Uint8Array(64);
    rom.set(thumbLoader(0x02001234, 0x07), 16);
    rom.set([0xde, 0xad, 0xbe, 0xef], 41);
    expect(thumbLiteral("10 b5 ?? 21 ?? 4b 1b 78", 4).find(rom)).toEqual([
      0x02001234,
    ]);
    expect(content("de ad be ef").find(rom)).toEqual([0x08000029]);
  });

  it("resolves a source when every locator that finds something agrees", () => {
    const rom = new Uint8Array(64);
    rom.set(thumbLoader(0x02001234, 0x07), 0);
    const agree = thumbLiteral("10 b5 ?? 21 ?? 4b 1b 78", 4);
    const empty = thumbLiteral("10 b5 ff 21 ?? 4b 1b 78", 4);
    const [result] = locateSources(
      { count: ram(u8, { locate: [agree, empty] }) },
      rom,
    );
    expect(result).toMatchObject({ address: 0x02001234, support: 1 });
    rom.set(thumbLoader(0x02005678, 0x09), 32);
    const [split] = locateSources({ count: ram(u8, { locate: [agree] }) }, rom);
    expect(split.address).toBeNull();
  });

  it("generates signatures that are unique in their ROM and survive a moved call site", () => {
    const rom = noise(256, 7);
    rom.set(thumbLoader(0x02003000, 0x03), 64);
    const [locator] = codeSignatures(rom, 0x02003000, { before: 2, after: 2 });
    expect(locator.find(rom)).toEqual([0x02003000]);
    const moved = new Uint8Array(rom.length + 8);
    moved.set(rom.subarray(0, 32));
    moved.set(rom.subarray(32), 40); // code inserted before the function
    expect(locator.find(moved)).toEqual([0x02003000]);
  });
});

describe("relocation", () => {
  // A parent ROM, and a child with 0x40 bytes inserted in the middle.
  const parent = noise(0x4000, 99);
  const loaderAt = 0x3000;
  parent.set(thumbLoader(0x02000010, 0x05), loaderAt);
  const child = new Uint8Array(parent.length + 0x40);
  child.set(parent.subarray(0, 0x2000));
  child.set(parent.subarray(0x2000), 0x2040);
  // The child's code names a RAM address moved by 0x14.
  new DataView(child.buffer).setUint32(loaderAt + 0x40 + 8, 0x02000024, true);

  it("finds piecewise offsets from unique windows", () => {
    const map = createOffsetMap(parent, child);
    expect(map.anchors).toBeGreaterThan(100);
    expect(map.deltaAt(0x1000)).toBe(0);
    expect(map.deltaAt(0x3800)).toBe(0x40);
  });

  it("moves ROM data by the map and RAM sources by their literal pools", () => {
    const tableFamily = defineFamily<{ party: number }, { table: number }>()({
      id: "example/table",
      sources: { count: ram(u8) },
      resources: ({ config }: RomContext<{ table: number }>) => config,
      views: { party: () => ({ status: "ready", data: 0 }) },
    });
    const { manifest, evidence } = relocateManifest(
      tableFamily,
      {
        rom: parent,
        manifest: {
          family: "example/table",
          romHash: "a".repeat(40),
          source: {
            repository: "r",
            revision: "v",
            variant: "p",
            compiler: "c",
            options: ["reviewed"],
          },
          addresses: { count: 0x02000010 },
          rom: { table: 0x08003400 },
          provenance: { method: "reviewed" },
        },
      },
      { rom: child, romHash: "b".repeat(40) },
      { tableBytes: 256 },
    );
    expect(manifest.rom.table).toBe(0x08003440);
    expect(manifest.addresses.count).toBe(0x02000024);
    expect(manifest.provenance).toEqual({
      method: "relocated",
      parent: "a".repeat(40),
    });
    expect(manifest.source.options).toEqual([
      `Relocated from ${"a".repeat(40)}; not reviewed`,
    ]);
    expect(evidence.find((e) => e.path === "rom.table")).toMatchObject({
      method: "table",
      score: 1,
    });
    expect(evidence.find((e) => e.path === "addresses.count")).toMatchObject({
      method: "literal",
      score: 1,
    });
  });
});

describe("ROM locators", () => {
  // A ROM with code loading a table's address, and the table itself: two
  // pointers and a run of data.
  function romWithTable(shift: number): Uint8Array {
    const rom = noise(0x1000 + shift, 5);
    const table = 0x800 + shift;
    rom.set(thumbLoader(0x08000000 + table, 0x04), 0x100);
    const view = new DataView(rom.buffer);
    view.setUint32(table, 0x08000400 + shift, true);
    view.setUint32(table + 4, 0x08000600 + shift, true);
    rom.set(
      [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc],
      table + 8,
    );
    return rom;
  }

  it("matches ROM pointers with ptr and signs data with its pointers masked", () => {
    const parent = romWithTable(0);
    const [data] = dataSignatures(parent, 0x08000800);
    expect(data.id).toMatch(/^content\("ptr ptr 11 22 33 44/);
    // The table moved and its pointers changed; its data still identifies it.
    expect(data.find(romWithTable(0x40))).toEqual([0x08000840]);
    expect(
      content("ptr 11 22").find(new Uint8Array([0, 0, 0, 0x0a, 0x11, 0x22])),
    ).toEqual([]);
  });

  it("gives pointer-only tables no data signature", () => {
    const rom = noise(0x1000, 3);
    const view = new DataView(rom.buffer);
    for (let i = 0; i < 16; i++)
      view.setUint32(0x800 + i * 4, 0x08000100 + i * 16, true);
    expect(dataSignatures(rom, 0x08000800)).toEqual([]);
  });

  it("falls back to a base-plus-offset load, and marks it inferred", () => {
    const rom = noise(512, 11);
    rom.set(thumbLoader(0x02004000, 0x02), 64); // code loads a global next to the field
    const [locator] = codeSignatures(rom, 0x02004006, {
      maxOffset: 0x10,
      before: 2,
      after: 2,
    });
    expect(locator.inferred).toBe(true);
    expect(locator.find(rom)).toEqual([0x02004006]);
    expect(codeSignatures(rom, 0x02004006, { before: 2, after: 2 })).toEqual(
      [],
    );
    const [result] = locateSources(
      { field: ram(u8, { locate: [locator] }) },
      rom,
    );
    expect(result).toMatchObject({ address: 0x02004006, inferred: true });
  });

  it("locates a whole build and writes a manifest from a template", () => {
    const parent = romWithTable(0);
    const romFamily = defineFamily<
      { party: number },
      { table: { base: number; count: number } }
    >()({
      id: "example/rom",
      sources: { count: ram(u8) },
      resources: ({
        config,
      }: RomContext<{ table: { base: number; count: number } }>) => config,
      views: { party: () => ({ status: "ready", data: 0 }) },
      romLocators: {
        "table.base": [
          ...codeSignatures(parent, 0x08000800, { before: 2, after: 2 }),
          ...dataSignatures(parent, 0x08000800),
        ],
      },
    });
    const template: ManifestOf<typeof romFamily> = {
      family: "example/rom",
      romHash: "a".repeat(40),
      source: {
        repository: "r",
        revision: "v",
        variant: "p",
        compiler: "c",
        options: ["reviewed"],
      },
      addresses: { count: 0x02000000 },
      rom: { table: { base: 0x08000800, count: 12 } },
      provenance: { method: "reviewed" },
    };
    const child = romWithTable(0x40);
    const located = locateBuild(romFamily, child);
    expect(located.rom).toMatchObject([
      {
        source: "table.base",
        address: 0x08000840,
        support: 2,
        inferred: false,
      },
    ]);
    const { manifest, unlocated } = manifestFromLocated(
      template,
      located,
      "b".repeat(40),
    );
    expect(manifest.rom).toEqual({ table: { base: 0x08000840, count: 12 } });
    expect(manifest.provenance).toEqual({
      method: "located",
      parent: "a".repeat(40),
    });
    expect(unlocated).toEqual(["addresses.count"]);
    // A locator path the manifest's ROM data no longer has is caught at startup.
    const stale = {
      ...romFamily,
      romLocators: { "table.gone": [] },
    } as unknown as typeof romFamily;
    expect(() => checkManifest(stale, template)).toThrow(
      "ROM locator path table.gone names no number",
    );
  });
});

describe("composed locators", () => {
  // Contiguous strings and a 20-entry table of pointers to them, moved
  // together by `shift` as a patch would move data.
  function romWithNames(shift: number): Uint8Array {
    const rom = noise(0x1000, 21);
    const view = new DataView(rom.buffer);
    const strings = [
      [0xa1, 0xa2, 0xa3, 0xff],
      [0xbb, 0xbc, 0xbd, 0xbe, 0xbf, 0xc0, 0xff], // the one we know
      [0xd5, 0xd6, 0xff],
      [0xc1, 0xc2, 0xc3, 0xc4, 0xff],
      [0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xff],
    ];
    let at = 0xa00 + shift;
    const starts = strings.map((bytes) => {
      rom.set(bytes, at);
      at += bytes.length;
      return at - bytes.length;
    });
    // Each entry points somewhere different, as in a real table of names.
    for (let entry = 0; entry < 20; entry++)
      view.setUint32(
        0x800 + shift + entry * 4,
        0x08000000 + (starts[entry] ?? 0xc00 + shift + entry * 8),
        true,
      );
    return rom;
  }
  const name = content("bb bc bd be bf c0 ff");

  it("finds a table of pointers through one of its targets, and follows it back", () => {
    const table = offset(references(name), -4);
    expect(table.find(romWithNames(0))).toEqual([0x08000800]);
    expect(table.find(romWithNames(0x40))).toEqual([0x08000840]);
    expect(follow(offset(table, 4)).find(romWithNames(0x40))).toEqual(
      name.find(romWithNames(0x40)),
    );
    expect(table.id).toBe(
      'offset(references(content("bb bc bd be bf c0 ff")), -4)',
    );
  });

  it("generates the composition for a table with no fixed bytes", () => {
    const rom = romWithNames(0);
    expect(dataSignatures(rom, 0x08000800)).toEqual([]);
    const [locator] = pointerSignatures(rom, 0x08000800);
    expect(locator.find(romWithNames(0x40))).toEqual([0x08000840]);
  });

  it("keeps inferred through composition", () => {
    const base = inferred(content("bb bc"));
    expect(offset(base, 1).inferred).toBe(true);
    expect(references(base).inferred).toBe(true);
    expect(offset(content("bb bc"), 1).inferred).toBeUndefined();
  });
});

describe("value locators", () => {
  // `cmp r0, #count` in a small routine, then `bx lr`.
  function romWithCount(count: number, literal?: number): Uint8Array {
    const rom = noise(512, 13);
    rom.set([0x03, 0x1c, 0x80, 0x00, count, 0x28, 0x70, 0x47], 0x80); // movs r3,r0; lsls r0; cmp r0,#count; bx lr
    if (literal !== undefined) rom.set(thumbLoader(literal, 0x06), 0x100);
    return rom;
  }

  it("reads an instruction's immediate, which may change between builds", () => {
    const locator = thumbImmediate("03 1c 80 00 ?? 28 70 47", 4);
    expect(locator.find(romWithCount(24))).toEqual([24]);
    expect(locator.find(romWithCount(30))).toEqual([30]);
  });

  it("signs a count from code that uses it, or its last ID as inferred", () => {
    const [exact] = valueSignatures(romWithCount(24), 24, {
      before: 4,
      after: 2,
    });
    expect(exact.inferred).toBeUndefined();
    expect(exact.find(romWithCount(40))).toEqual([40]);
    // Exact uses rank first; the bounds check against 24 is the inferred one.
    const lastId = valueSignatures(romWithCount(24), 25, {
      before: 4,
      after: 2,
    }).find((locator) => locator.inferred)!;
    expect(lastId.find(romWithCount(40))).toEqual([41]);
    const [pooled] = valueSignatures(romWithCount(0, 2902), 2902, {
      before: 2,
      after: 2,
    });
    expect(pooled.find(romWithCount(0, 3000))).toEqual([3000]);
  });

  it("replaces a template's count when it is located", () => {
    const rom = romWithCount(30);
    const countFamily = defineFamily<
      { party: number },
      { speciesCount: number }
    >()({
      id: "example/count",
      sources: { count: ram(u8) },
      resources: ({ config }: RomContext<{ speciesCount: number }>) => config,
      views: { party: () => ({ status: "ready", data: 0 }) },
      romLocators: {
        speciesCount: [thumbImmediate("03 1c 80 00 ?? 28 70 47", 4)],
      },
    });
    const template: ManifestOf<typeof countFamily> = {
      family: "example/count",
      romHash: "a".repeat(40),
      source: {
        repository: "r",
        revision: "v",
        variant: "p",
        compiler: "c",
        options: [],
      },
      addresses: { count: 0x02000000 },
      rom: { speciesCount: 24 },
      provenance: { method: "reviewed" },
    };
    const { manifest } = manifestFromLocated(
      template,
      locateBuild(countFamily, rom),
      "b".repeat(40),
    );
    expect(manifest.rom.speciesCount).toBe(30);
  });
});

describe("family detection", () => {
  const rom = noise(0x400, 17);
  rom.set(
    Array.from("BPEE", (c) => c.charCodeAt(0)),
    0xac,
  );
  rom.set(thumbLoader(0x02001234, 0x07), 0x200);
  const marker = thumbLiteral("10 b5 ?? 21 ?? 4b", 4);
  const detected = defineFamily<{ party: number }>()({
    id: "example/detect",
    sources: { count: ram(u8, { locate: [marker] }) },
    resources: () => ({}),
    views: { party: () => ({ status: "ready", data: 0 }) },
    detect: { gameCodes: ["BPEE"] },
  });

  it("checks the header, then the family's own locators", () => {
    expect(detectFamily(detected, rom)).toMatchObject({
      header: true,
      score: 1,
    });
    const other = rom.slice();
    other.set(
      Array.from("AXVE", (c) => c.charCodeAt(0)),
      0xac,
    );
    expect(detectFamily(detected, other)).toMatchObject({
      header: false,
      score: 0,
    });
    const changed = rom.slice();
    changed.fill(0, 0x200, 0x210);
    expect(detectFamily(detected, changed)).toMatchObject({
      header: true,
      score: 0,
    });
  });
});

describe("computed addresses", () => {
  // Thumb code that reaches a field and an array element from literals:
  //   ldr r0, =0x03001574 ; ldr r1, =0x439 ; adds r0, r0, r1 ; ldrb r0, [r0]
  //   ldr r3, =0x02001424 ; lsls r2, r5, #1 ; ldrh r3, [r2, r3]
  function romWithAccesses(field = 0x439, array = 0x02001424): Uint8Array {
    const rom = noise(0x400, 29);
    const view = new DataView(rom.buffer);
    rom.set(
      [0x02, 0x48, 0x03, 0x49, 0x40, 0x18, 0x00, 0x78, 0x70, 0x47, 0xc0, 0x46],
      0x100,
    );
    view.setUint32(0x10c, 0x03001574, true);
    view.setUint32(0x110, field, true);
    rom.set([0x01, 0x4b, 0x6a, 0x00, 0xd3, 0x5a, 0x70, 0x47], 0x200);
    view.setUint32(0x208, array, true);
    return rom;
  }

  it("works out the address a load uses, dropping an unknown index", () => {
    const field = thumbAccess("?? 48 ?? 49 40 18 00 78", 6);
    expect(field.find(romWithAccesses())).toEqual([0x030019ad]);
    expect(field.find(romWithAccesses(0x43a))).toEqual([0x030019ae]);
    const element = thumbAccess("?? 4b 6a 00 d3 5a", 4);
    expect(element.find(romWithAccesses())).toEqual([0x02001424]);
    expect(element.inferred).toBeUndefined();
  });

  it("signs a field from the code that computes it, not a neighbour of it", () => {
    const rom = romWithAccesses();
    const [locator] = accessSignatures(rom, 0x030019ad, {
      window: 4,
      after: 2,
    });
    expect(locator.id).toMatch(/^thumbAccess\(/);
    expect(locator.find(romWithAccesses(0x43a))).toEqual([0x030019ae]);
    expect(accessSignatures(rom, 0x030019ac, { window: 4, after: 2 })).toEqual(
      [],
    );
  });

  it("reads a constant built as movs then lsls, and ranks exact values first", () => {
    const rom = noise(256, 31);
    rom.set([0xe7, 0x21, 0x49, 0x00, 0x88, 0x42, 0x70, 0x47], 0x40); // movs r1,#231; lsls r1,r1,#1; cmp r0,r1; bx lr
    expect(thumbImmediate("?? 21 49 00 88 42", 0).find(rom)).toEqual([462]);
    const [exact] = valueSignatures(rom, 462, { before: 2, after: 4 });
    expect(exact.inferred).toBeUndefined();
    expect(exact.find(rom)).toEqual([462]);
  });
});

describe("runtime locating", () => {
  const childHash = "b".repeat(40);
  const child = runtimeRom(0x02000100, 0x40);
  const fast = { yieldControl: async () => {} };

  it("locates a build from the family's locators and the reviewed template", async () => {
    const game = runtimeGame();
    let detected = 0;
    const outcome = await locateGame(game, child, {
      ...fast,
      romHash: childHash,
      onDetected: () => detected++,
    });
    expect(detected).toBe(1);
    expect(outcome).toMatchObject({
      status: "located",
      build: {
        version: locatable(game)!.version,
        review: [],
        manifest: {
          romHash: childHash,
          addresses: { count: 0x02000100 },
          rom: { names: 0x08000440 },
          provenance: { method: "located", parent: reviewedHash },
        },
      },
    });
  });

  it("registers a located build so the game resolves it, until it is removed", async () => {
    const game = runtimeGame();
    const outcome = await locateGame(game, child, {
      ...fast,
      romHash: childHash,
    });
    if (outcome.status !== "located") throw new Error(outcome.status);
    expect(resolveGame([game], childHash)).toBeNull();
    expect(buildProvenance(game, childHash)).toBeNull();
    registerBuild(game, outcome.build.manifest);
    expect(resolveGame([game], childHash)).toBe(game);
    expect(buildProvenance(game, childHash)).toBe("located");
    expect(buildProvenance(game, reviewedHash)).toBe("reviewed");
    unregisterBuild(game, childHash);
    unregisterBuild(game, reviewedHash);
    expect(resolveGame([game], childHash)).toBeNull();
    expect(resolveGame([game], reviewedHash)).toBe(game);
    expect(() =>
      registerBuild(game, { ...outcome.build.manifest, family: "other" }),
    ).toThrow("targets other");
  });

  it("takes its template from locatedFrom, which must be a reviewed build", () => {
    expect(locatable(runtimeGame())?.template).toBe(runtimeTemplate);
    const { locatedFrom: _, ...unlocated } = runtimeFamily;
    expect(locatable(createGame(unlocated, [runtimeTemplate]))).toBeNull();
    expect(() =>
      createGame({ ...runtimeFamily, locatedFrom: "c".repeat(40) }, [
        runtimeTemplate,
      ]),
    ).toThrow("which is not one of its reviewed builds");
  });

  it("changes version when a locator or the template changes", () => {
    const version = locatable(runtimeGame())!.version;
    expect(locatable(runtimeGame())!.version).toBe(version);
    const changed = runtimeGame({
      ...runtimeFamily,
      romLocators: { names: [content("de ad be")] },
    });
    expect(locatable(changed)!.version).not.toBe(version);
  });

  it("leaves other games alone, unless the caller already knows the game", async () => {
    const other = runtimeRom(0x02000100, 0x40, "AXVE");
    let detected = 0;
    const game = runtimeGame();
    expect(
      await locateGame(game, other, {
        ...fast,
        romHash: childHash,
        onDetected: () => detected++,
      }),
    ).toMatchObject({ status: "not-this-game", detection: { header: false } });
    expect(detected).toBe(0);
    expect(
      await locateGame(game, other, {
        ...fast,
        romHash: childHash,
        known: true,
      }),
    ).toMatchObject({ status: "located" });
  });

  it("rejects a build with disagreeing locators, a missing address or a failing probe", async () => {
    const game = runtimeGame();
    const twice = child.slice();
    twice.set(thumbLoader(0x02000200, 0x07), 0x600);
    expect(
      await locateGame(game, twice, { ...fast, romHash: childHash }),
    ).toMatchObject({
      status: "rejected",
      reason: "Locators disagree on count",
    });
    const nameless = child.slice();
    nameless.fill(0, 0x440, 0x444);
    expect(
      await locateGame(game, nameless, { ...fast, romHash: childHash }),
    ).toMatchObject({ status: "rejected", reason: "Not located: rom.names" });
    const probed = runtimeGame({
      ...runtimeFamily,
      probes: {
        names: () => {
          throw new Error("bad table");
        },
      },
    });
    expect(
      await locateGame(probed, child, { ...fast, romHash: childHash }),
    ).toMatchObject({
      status: "rejected",
      reason: "Probe names failed: bad table",
    });
  });

  it("stops when aborted", async () => {
    const controller = new AbortController();
    controller.abort();
    await expect(
      locateGame(runtimeGame(), child, { signal: controller.signal }),
    ).rejects.toMatchObject({ name: "AbortError" });
  });

  it("hashes the ROM when the caller does not give its SHA-1", async () => {
    const outcome = await locateGame(runtimeGame(), child, fast);
    const digest = new Uint8Array(
      await crypto.subtle.digest("SHA-1", child.slice().buffer),
    );
    expect(outcome).toMatchObject({
      status: "located",
      build: {
        manifest: {
          romHash: Array.from(digest, (b) =>
            b.toString(16).padStart(2, "0"),
          ).join(""),
        },
      },
    });
  });
});
