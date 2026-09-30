// Finding sources in ROM images: locators, and relocating a verified build's
// manifest onto a nearby build. Pure functions over raw cartridge bytes, for
// tools that write and check manifests; the player never runs them.
import {
  checkManifest,
  locatable,
  type Family,
  type Locator,
  type Manifest,
  type Sources,
} from "./family.js";
import { createMemoryReader } from "./memory.js";

const ROM_BASE = 0x08000000;
const ROM_END = 0x0a000000;
const isRom = (value: number) => value >= ROM_BASE && value < ROM_END;
/** EWRAM or IWRAM. */
export const isWorkRam = (value: number) =>
  (value >= 0x02000000 && value < 0x02040000) ||
  (value >= 0x03000000 && value < 0x03008000);

function word(bytes: Uint8Array, offset: number): number {
  return (
    (bytes[offset] |
      (bytes[offset + 1] << 8) |
      (bytes[offset + 2] << 16) |
      (bytes[offset + 3] << 24)) >>>
    0
  );
}

// Pattern bytes: 0-255 exact, ANY matches anything, POINTER_HIGH matches the
// top byte of a ROM pointer (0x08 or 0x09).
const ANY = -1;
const POINTER_HIGH = -2;

/**
 * Hex bytes with `??` wildcards and `ptr` for a 4-byte ROM pointer, e.g.
 * `"?? 48 00 68 ptr 01 00"`.
 */
function parsePattern(pattern: string): number[] {
  return pattern
    .trim()
    .split(/\s+/)
    .flatMap((token) => {
      if (token === "??") return [ANY];
      if (token === "ptr") return [ANY, ANY, ANY, POINTER_HIGH];
      if (!/^[0-9a-fA-F]{2}$/.test(token))
        throw new Error(`Invalid pattern byte ${token}`);
      return [parseInt(token, 16)];
    });
}
// How often each byte occurs, per ROM image, so a pattern is searched by its
// rarest fixed byte rather than by a common one such as 0x00 or 0xff.
const histograms = new WeakMap<Uint8Array, Uint32Array>();
function histogram(rom: Uint8Array): Uint32Array {
  let counts = histograms.get(rom);
  if (!counts) {
    counts = new Uint32Array(256);
    for (let i = 0; i < rom.length; i++) counts[rom[i]]++;
    histograms.set(rom, counts);
  }
  return counts;
}

function matches(rom: Uint8Array, pattern: number[], align = 1): number[] {
  const counts = histogram(rom);
  let anchor = -1;
  for (let i = 0; i < pattern.length; i++)
    if (
      pattern[i] >= 0 &&
      (anchor < 0 || counts[pattern[i]] < counts[pattern[anchor]])
    )
      anchor = i;
  if (anchor < 0) throw new Error("A pattern needs at least one fixed byte");
  const out: number[] = [];
  const value = pattern[anchor];
  for (
    let at = rom.indexOf(value, anchor);
    at >= 0;
    at = rom.indexOf(value, at + 1)
  ) {
    const start = at - anchor;
    if (start + pattern.length > rom.length) break;
    if (start % align) continue;
    if (
      pattern.every((expected, i) => {
        const byte = rom[start + i];
        return expected === ANY
          ? true
          : expected === POINTER_HIGH
            ? byte === 0x08 || byte === 0x09
            : byte === expected;
      })
    )
      out.push(start);
  }
  return out;
}

/**
 * The word a Thumb `ldr rN, [pc, #imm]` loads from its literal pool, where the
 * load is `ldr` bytes into `pattern`: an address (a RAM global, a ROM table)
 * or a constant such as a table's count.
 */
export function thumbLiteral(pattern: string, ldr: number): Locator {
  const bytes = parsePattern(pattern);
  return {
    id: `thumbLiteral(${JSON.stringify(pattern)}, ${ldr})`,
    find(rom) {
      const found = new Set<number>();
      for (const start of matches(rom, bytes, 2)) {
        const at = start + ldr;
        const instruction = rom[at] | (rom[at + 1] << 8);
        if ((instruction & 0xf800) !== 0x4800) continue;
        const literal = ((at + 4) & ~3) + (instruction & 0xff) * 4;
        if (literal + 4 > rom.length) continue;
        found.add(word(rom, literal));
      }
      return [...found];
    },
  };
}

/**
 * The 8-bit immediate of a Thumb `mov`, `cmp`, `add` or `sub` (`rN, #imm`),
 * where the instruction is `at` bytes into `pattern`: a constant such as a
 * table's count. A `movs rd, #a` followed by `lsls rd, rd, #n` (how compilers
 * build larger constants) gives `a << n`. Leave the immediate byte a wildcard
 * so a changed value still matches.
 */
export function thumbImmediate(pattern: string, at: number): Locator {
  const bytes = parsePattern(pattern);
  return {
    id: `thumbImmediate(${JSON.stringify(pattern)}, ${at})`,
    find(rom) {
      const found = new Set<number>();
      for (const start of matches(rom, bytes, 2)) {
        const high = rom[start + at + 1];
        if ((high & 0xe0) !== 0x20) continue;
        found.add(rom[start + at] << shiftAfter(rom, start + at));
      }
      return [...found];
    },
  };
}

/** For `movs rd, #a` at `at`, the `n` of a following `lsls rd, rd, #n`, else 0. */
function shiftAfter(rom: Uint8Array, at: number): number {
  const mov = rom[at] | (rom[at + 1] << 8);
  if ((mov & 0xf800) !== 0x2000 || at + 4 > rom.length) return 0;
  const rd = (mov >> 8) & 7;
  const next = rom[at + 2] | (rom[at + 3] << 8);
  const same = (next & 7) === rd && ((next >> 3) & 7) === rd;
  return (next & 0xf800) === 0x0000 && same ? (next >> 6) & 31 : 0;
}

/** A register's constant part, and whether an unknown term (an index) is added. */
interface Term {
  readonly c: number;
  readonly u: boolean;
}
const sum = (a: Term | null, b: Term | null): Term =>
  a && b
    ? { c: (a.c + b.c) >>> 0, u: a.u || b.u }
    : { c: (a ?? b)?.c ?? 0, u: true };

/**
 * Registers after running Thumb code from `from` to `to` symbolically: literal
 * loads and immediates are constants, an unknown value added in (an array
 * index) keeps the constant part. Anything not modelled forgets what it
 * writes, and control flow forgets everything.
 */
function evaluate(rom: Uint8Array, from: number, to: number): (Term | null)[] {
  const regs: (Term | null)[] = Array.from({ length: 16 }, () => null);
  for (let pc = from; pc < to; pc += 2) {
    const i = rom[pc] | (rom[pc + 1] << 8);
    const rd = i & 7,
      rs = (i >> 3) & 7,
      r8 = (i >> 8) & 7;
    if ((i & 0xf800) === 0x4800) {
      const literal = ((pc + 4) & ~3) + (i & 0xff) * 4;
      regs[r8] =
        literal + 4 <= rom.length ? { c: word(rom, literal), u: false } : null;
    } else if ((i & 0xf800) === 0x2000) regs[r8] = { c: i & 0xff, u: false };
    else if ((i & 0xf000) === 0x3000) {
      const t = regs[r8];
      const delta = i & 0x0800 ? -(i & 0xff) : i & 0xff;
      regs[r8] = t && { c: (t.c + delta) >>> 0, u: t.u };
    } else if ((i & 0xfc00) === 0x1c00) {
      const t = regs[rs];
      const delta = i & 0x0200 ? -((i >> 6) & 7) : (i >> 6) & 7;
      regs[rd] = t && { c: (t.c + delta) >>> 0, u: t.u };
    } else if ((i & 0xfe00) === 0x1800)
      regs[rd] = sum(regs[rs], regs[(i >> 6) & 7]);
    else if ((i & 0xf800) === 0x0000) {
      // lsls: a constant shifts; an unknown index stays an unknown term.
      const t = regs[rs];
      const k = (i >> 6) & 31;
      regs[rd] = t ? { c: (t.c << k) >>> 0, u: t.u } : { c: 0, u: true };
    } else if ((i & 0xff00) === 0x4400 || (i & 0xff00) === 0x4600) {
      const d = (i & 7) | ((i >> 4) & 8),
        m = (i >> 3) & 15;
      if (d >= 13 || m >= 13) regs[d] = null;
      else regs[d] = (i & 0xff00) === 0x4400 ? sum(regs[d], regs[m]) : regs[m];
    } else if (
      (i & 0xf000) === 0xd000 ||
      (i & 0xf800) === 0xe000 ||
      (i & 0xf000) === 0xf000 ||
      (i & 0xff00) === 0x4700 ||
      (i & 0xf600) === 0xb400 ||
      (i & 0xf000) === 0xc000
    )
      regs.fill(null); // branches, calls, push/pop and multiple transfers
    else if (
      (i & 0xf800) === 0x2800 || // cmp rN, #imm
      (i & 0xffc0) === 0x4200 || // tst
      (i & 0xff80) === 0x4280 || // cmp, cmn
      (i & 0xff00) === 0x4500 || // cmp (high)
      (i & 0xfc00) === 0x5000 || // str, strh (register offset)
      (i & 0xfe00) === 0x5400 || // strb (register offset)
      (i & 0xf800) === 0x6000 || // str
      (i & 0xf800) === 0x7000 || // strb
      (i & 0xf800) === 0x8000 || // strh
      (i & 0xf800) === 0x9000 || // str sp
      (i & 0xff00) === 0xb000 // add/sub sp
    )
      continue;
    else if ((i & 0xf000) === 0x9000 || (i & 0xf000) === 0xa000)
      regs[r8] = null;
    else regs[rd] = null;
  }
  return regs;
}

/** The constant part of the address a Thumb load or store at `at` uses, or null. */
function accessAt(rom: Uint8Array, from: number, at: number): number | null {
  const regs = evaluate(rom, from, at);
  const i = rom[at] | (rom[at + 1] << 8);
  const base = regs[(i >> 3) & 7];
  if (!base) return null;
  const scale =
    (i & 0xf000) === 0x6000
      ? 4
      : (i & 0xf000) === 0x7000
        ? 1
        : (i & 0xf000) === 0x8000
          ? 2
          : 0;
  if (scale) return (base.c + ((i >> 6) & 31) * scale) >>> 0;
  if ((i & 0xf000) === 0x5000) return sum(base, regs[(i >> 6) & 7]).c;
  return null;
}

/**
 * The address the Thumb load or store `at` bytes into `pattern` accesses,
 * worked out from the instructions before it in the pattern (literal loads,
 * immediates, adds, shifts). An unknown index added in drops out, so an
 * indexed access gives its array's start. This finds a field that code
 * reaches from a global plus an offset, or through an index, exactly.
 */
export function thumbAccess(pattern: string, at: number): Locator {
  const bytes = parsePattern(pattern);
  return {
    id: `thumbAccess(${JSON.stringify(pattern)}, ${at})`,
    find(rom) {
      const found = new Set<number>();
      for (const start of matches(rom, bytes, 2)) {
        const address = accessAt(rom, start, start + at);
        if (address !== null && (isWorkRam(address) || isRom(address)))
          found.add(address);
      }
      return [...found];
    },
  };
}

/**
 * Data at a ROM address, such as a table's first entries or a string: exact
 * bytes, `??` wildcards and `ptr` for words that are ROM pointers. A pattern
 * with `ptr` matches word-aligned data only.
 */
export function content(pattern: string): Locator {
  const bytes = parsePattern(pattern);
  const align = /(^|\s)ptr(\s|$)/.test(pattern) ? 4 : 1;
  return {
    id: `content(${JSON.stringify(pattern)})`,
    find: (rom) => matches(rom, bytes, align).map((at) => ROM_BASE + at),
  };
}

/** Each of `locator`'s results plus `delta`: a table's start from an entry. */
export function offset(locator: Locator, delta: number): Locator {
  return {
    ...(locator.inferred ? { inferred: true } : {}),
    id: `offset(${locator.id}, ${delta})`,
    find: (rom) => locator.find(rom).map((value) => (value + delta) >>> 0),
  };
}

/**
 * Where a pointer to any of `locator`'s results is stored: the word-aligned
 * ROM addresses holding it. `references(pokemonText("BULBASAUR"))` finds the
 * name table entry that points at the name.
 */
export function references(locator: Locator): Locator {
  return {
    ...(locator.inferred ? { inferred: true } : {}),
    id: `references(${locator.id})`,
    find(rom) {
      const targets = new Set(locator.find(rom).filter(isRom));
      if (!targets.size) return [];
      const found: number[] = [];
      for (let at = 0; at + 4 <= rom.length; at += 4)
        if (rom[at + 3] === 0x08 || rom[at + 3] === 0x09)
          if (targets.has(word(rom, at))) found.push(ROM_BASE + at);
      return found;
    },
  };
}

/** The pointer stored at each of `locator`'s results: the inverse of `references`. */
export function follow(locator: Locator): Locator {
  return {
    ...(locator.inferred ? { inferred: true } : {}),
    id: `follow(${locator.id})`,
    find: (rom) =>
      locator
        .find(rom)
        .filter(
          (address) => isRom(address) && address - ROM_BASE + 4 <= rom.length,
        )
        .map((address) => word(rom, address - ROM_BASE)),
  };
}

/**
 * Marks `locator` as resting on an assumed layout (a field at a fixed offset
 * from a global, a count one past a bounds check) rather than finding the
 * value itself; results that only inferred locators support need review.
 */
export function inferred(locator: Locator): Locator {
  return { ...locator, inferred: true, id: `inferred(${locator.id})` };
}

export interface LocateResult {
  /** A source name, or a path into the manifest's ROM data (`catalog.species`). */
  readonly source: string;
  /** Agreed address, or null when locators disagree or find nothing. */
  readonly address: number | null;
  /** How many locators found the agreed address, out of all of them. */
  readonly support: number;
  /** Every agreeing locator is `inferred`: review the value by hand. */
  readonly inferred: boolean;
  readonly candidates: readonly {
    readonly locator: string;
    readonly found: readonly number[];
  }[];
}
/**
 * A value resolves when every locator that finds something finds the same
 * single address; code or data that changed in a new build may leave some
 * locators empty, which lowers `support` rather than giving a wrong address.
 */
function resolve(
  source: string,
  locators: readonly Locator[],
  rom: Uint8Array,
): LocateResult {
  const candidates = locators.map((locator) => ({
    locator: locator.id,
    inferred: locator.inferred ?? false,
    found: locator.find(rom),
  }));
  const answering = candidates.filter(({ found }) => found.length);
  const all = new Set(answering.flatMap(({ found }) => found));
  const agreed =
    all.size === 1 && answering.every(({ found }) => found.length === 1)
      ? [...all][0]
      : null;
  return {
    source,
    address: agreed,
    support: agreed === null ? 0 : answering.length,
    inferred: agreed !== null && answering.every(({ inferred }) => inferred),
    candidates,
  };
}

/** Runs every source's locators against a ROM image. */
export function locateSources(
  sources: Sources,
  rom: Uint8Array,
): LocateResult[] {
  return Object.entries(sources).flatMap(([source, spec]) =>
    spec.locate?.length ? [resolve(source, spec.locate, rom)] : [],
  );
}

/** One unit of locating work per value: a source name or a ROM data path. */
function locateSteps(
  family: Family<any, any, any, any>,
  rom: Uint8Array,
): { kind: "sources" | "rom"; run: () => LocateResult }[] {
  return [
    ...Object.entries(family.sources as Sources).flatMap(([name, spec]) =>
      spec.locate?.length
        ? [
            {
              kind: "sources" as const,
              run: () => resolve(name, spec.locate!, rom),
            },
          ]
        : [],
    ),
    ...Object.entries(
      (family.romLocators ?? {}) as Record<string, readonly Locator[]>,
    ).flatMap(([path, locators]) =>
      locators.length
        ? [{ kind: "rom" as const, run: () => resolve(path, locators, rom) }]
        : [],
    ),
  ];
}

/** Runs a family's RAM and ROM locators against a ROM image. */
export function locateBuild(
  family: Family<any, any, any, any>,
  rom: Uint8Array,
): { readonly sources: LocateResult[]; readonly rom: LocateResult[] } {
  const out = { sources: [] as LocateResult[], rom: [] as LocateResult[] };
  for (const step of locateSteps(family, rom)) out[step.kind].push(step.run());
  return out;
}

/** A build `locateGame` recognized: its manifest, and what to review in it. */
export interface LocatedBuild {
  readonly manifest: Manifest<any, any>;
  /** The game's locator version when it was located (see `locatable`). */
  readonly version: string;
  /** Values found only by inferred locators, or kept from the template. */
  readonly review: readonly string[];
}
export type LocateOutcome =
  | { readonly status: "located"; readonly build: LocatedBuild }
  | { readonly status: "not-this-game"; readonly detection: Detection | null }
  | {
      readonly status: "rejected";
      readonly reason: string;
      readonly detection: Detection | null;
    };

const macrotask = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

/**
 * Tries to recognize `rom` as a build of `game` from its locators alone, for
 * a player meeting a ROM with no reviewed manifest. It checks the family's
 * detector first (unless `known`), runs each locator with a pause in between,
 * fills what locators cannot find from the reviewed template, and accepts the
 * build only if no locators disagree, every address is found, the manifest
 * checks pass and every probe passes. Register an accepted build with
 * `registerBuild`.
 */
export async function locateGame(
  game: object,
  rom: Uint8Array,
  options: {
    readonly signal?: AbortSignal;
    /** Awaited between locators; defaults to a macrotask. */
    readonly yieldControl?: () => Promise<void>;
    /** The caller already knows the game (a catalog version): skip detection. */
    readonly known?: boolean;
    /** The ROM's SHA-1, when the caller has it. */
    readonly romHash?: string;
    /** Called once the ROM looks like the game, before the slower locating. */
    readonly onDetected?: () => void;
  } = {},
): Promise<LocateOutcome> {
  const target = locatable(game);
  if (!target) return { status: "not-this-game", detection: null };
  const { family, template, version } = target;
  const pause = options.yieldControl ?? macrotask;
  let detection: Detection | null = null;
  if (!options.known) {
    detection = detectFamily(family, rom);
    if (detection.score < 0.75) return { status: "not-this-game", detection };
  }
  options.onDetected?.();
  const reject = (reason: string): LocateOutcome => ({
    status: "rejected",
    reason,
    detection,
  });
  const located = { sources: [] as LocateResult[], rom: [] as LocateResult[] };
  for (const step of locateSteps(family, rom)) {
    await pause();
    options.signal?.throwIfAborted();
    located[step.kind].push(step.run());
  }
  const disagreed = [...located.sources, ...located.rom].filter(
    (r) => r.address === null && r.candidates.some((c) => c.found.length),
  );
  if (disagreed.length)
    return reject(
      `Locators disagree on ${disagreed.map((r) => r.source).join(", ")}`,
    );
  const romHash = options.romHash ?? (await sha1(rom));
  const { manifest, unlocated } = manifestFromLocated(
    template,
    located,
    romHash,
  );
  // Every address must be found; counts and labels may come from the template.
  const missing = unlocated.filter((path) => {
    if (path.startsWith("addresses.")) return true;
    const value = path
      .slice("rom.".length)
      .split(".")
      .reduce<unknown>(
        (node, key) => (node as Record<string, unknown> | undefined)?.[key],
        template.rom,
      );
    return typeof value === "number" && isRom(value);
  });
  if (missing.length) return reject(`Not located: ${missing.join(", ")}`);
  try {
    checkManifest(family, manifest);
  } catch (error) {
    return reject(error instanceof Error ? error.message : String(error));
  }
  await pause();
  options.signal?.throwIfAborted();
  const resources = family.resources({
    rom: createMemoryReader([{ address: ROM_BASE, bytes: rom }]),
    romHash,
    config: manifest.rom,
  });
  for (const [name, probe] of Object.entries(family.probes ?? {})) {
    try {
      probe(resources, romHash);
    } catch (error) {
      return reject(
        `Probe ${name} failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }
  const review = [
    ...located.sources
      .filter((r) => r.inferred)
      .map((r) => `addresses.${r.source}`),
    ...located.rom.filter((r) => r.inferred).map((r) => `rom.${r.source}`),
    ...unlocated,
  ];
  return { status: "located", build: { manifest, version, review } };
}

async function sha1(bytes: Uint8Array): Promise<string> {
  const digest = await globalThis.crypto.subtle.digest(
    "SHA-1",
    bytes.slice().buffer,
  );
  return Array.from(new Uint8Array(digest), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
}

/**
 * A manifest for `romHash` from located values: every resolved source and ROM
 * path takes the located address, and everything else (counts, labels,
 * unlocated values) comes from `template`, a verified build's manifest.
 */
export function manifestFromLocated<S extends Sources, Rom>(
  template: Manifest<S, Rom>,
  located: ReturnType<typeof locateBuild>,
  romHash: string,
): { manifest: Manifest<S, Rom>; unlocated: string[] } {
  const unlocated: string[] = [];
  const addresses = { ...(template.addresses as Record<string, number>) };
  for (const name of Object.keys(addresses)) {
    const result = located.sources.find((r) => r.source === name);
    if (result?.address != null) addresses[name] = result.address;
    else unlocated.push(`addresses.${name}`);
  }
  const rom = structuredClone(template.rom) as Rom;
  const results = new Map(located.rom.map((r) => [r.source, r]));
  // Addresses must be located; other numbers (counts, labels) are replaced
  // when they have locators and kept from the template otherwise.
  const walk = (value: unknown, path: string, set: (v: number) => void) => {
    if (typeof value === "number") {
      const result = results.get(path);
      if (result?.address != null) set(result.address);
      else if (result || isRom(value)) unlocated.push(`rom.${path}`);
    } else if (value && typeof value === "object")
      for (const [key, item] of Object.entries(value))
        walk(item, path ? `${path}.${key}` : key, (v) => {
          (value as Record<string, unknown>)[key] = v;
        });
  };
  walk(rom, "", () => {});
  return {
    manifest: {
      family: template.family,
      romHash,
      source: {
        ...template.source,
        options: [`Located from ${template.romHash}'s locators; not reviewed`],
      },
      addresses: addresses as Manifest<S, Rom>["addresses"],
      rom,
      provenance: { method: "located", parent: template.romHash },
    },
    unlocated,
  };
}

// ---------------------------------------------------------------------------
// Relocation: map a verified build's manifest onto a nearby build.

/** Piecewise offsets from `parent` to `child` bytes, from unique 16-byte windows. */
export interface OffsetMap {
  /** The most common offset among anchors around a parent ROM offset. */
  deltaAt(offset: number): number;
  /** Distinct offsets near `offset`, most common first. */
  candidatesAt(offset: number): readonly number[];
  readonly anchors: number;
}
const WINDOW = 16;
function windowKey(bytes: Uint8Array, at: number): number | null {
  let distinct = 0,
    seen = 0;
  let h1 = 0x811c9dc5,
    h2 = 0x01000193;
  for (let i = 0; i < WINDOW; i++) {
    const x = bytes[at + i];
    if (!(seen & (1 << (x & 31)))) {
      seen |= 1 << (x & 31);
      distinct++;
    }
    h1 = Math.imul(h1 ^ x, 16777619);
    h2 = Math.imul(h2 + x, 2654435761) ^ (h2 >>> 13);
  }
  if (distinct < 4) return null; // padding and fills are not anchors
  return (h1 >>> 0) * 1048576 + ((h2 >>> 0) & 0xfffff);
}
export function createOffsetMap(
  parent: Uint8Array,
  child: Uint8Array,
  step = 64,
): OffsetMap {
  const index = new Map<number, number>();
  for (let at = 0; at + WINDOW <= child.length; at += 4) {
    const key = windowKey(child, at);
    if (key !== null) index.set(key, index.has(key) ? -1 : at);
  }
  const parentCount = new Map<number, number>();
  for (let at = 0; at + WINDOW <= parent.length; at += 4) {
    const key = windowKey(parent, at);
    if (key !== null) parentCount.set(key, (parentCount.get(key) ?? 0) + 1);
  }
  const offsets: number[] = [];
  const deltas: number[] = [];
  for (let at = 0; at + WINDOW <= parent.length; at += step) {
    const key = windowKey(parent, at);
    if (key === null || parentCount.get(key) !== 1) continue;
    const target = index.get(key);
    if (target === undefined || target < 0) continue;
    let same = true;
    for (let i = 0; i < WINDOW && same; i++)
      same = parent[at + i] === child[target + i];
    if (!same) continue;
    offsets.push(at);
    deltas.push(target - at);
  }
  const candidatesAt = (offset: number) => {
    let lo = 0,
      hi = offsets.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (offsets[mid] < offset) lo = mid + 1;
      else hi = mid;
    }
    const votes = new Map<number, number>();
    for (let i = Math.max(0, lo - 9); i < Math.min(offsets.length, lo + 9); i++)
      votes.set(deltas[i], (votes.get(deltas[i]) ?? 0) + 1);
    return [...votes].sort((a, b) => b[1] - a[1]).map(([delta]) => delta);
  };
  return {
    anchors: offsets.length,
    candidatesAt,
    deltaAt: (offset) => candidatesAt(offset)[0] ?? 0,
  };
}

/**
 * Pointer-aware comparison of `length` bytes: equal words match, and a word
 * that is a ROM pointer matches when it points to the relocated target.
 */
function compare(
  parent: Uint8Array,
  child: Uint8Array,
  map: OffsetMap,
  from: number,
  to: number,
  length: number,
): number {
  let words = 0,
    equal = 0;
  for (let i = 0; i + 4 <= length; i += 4) {
    if (from + i + 4 > parent.length || to + i + 4 > child.length) break;
    const a = word(parent, from + i),
      b = word(child, to + i);
    words++;
    if (a === b) equal++;
    else if (isRom(a) && b === a + map.deltaAt(a - ROM_BASE)) equal++;
  }
  return words ? equal / words : 0;
}

export interface RelocationEvidence {
  /** Manifest path, such as `addresses.party` or `rom.catalog.species`. */
  readonly path: string;
  readonly from: number;
  readonly to: number | null;
  /** How the new value was found, and how strongly. */
  readonly method:
    | "table"
    | "literal"
    | "neighbour"
    | "unchanged"
    | "unresolved";
  /** Table match ratio, or the share of literal pools that agree. */
  readonly score: number;
  readonly detail: string;
}

/**
 * Relocates a verified manifest onto a nearby build: ROM data by the offset
 * map (checked pointer-aware), RAM sources by pairing the literal pools that
 * load them. Returns the candidate manifest and per-value evidence; review
 * weak or unresolved values before registering the build.
 */
export function relocateManifest<S extends Sources, Rom>(
  family: Family<any, S, any, Rom>,
  parent: { readonly rom: Uint8Array; readonly manifest: Manifest<S, Rom> },
  child: { readonly rom: Uint8Array; readonly romHash: string },
  options: { readonly tableBytes?: number } = {},
): { manifest: Manifest<S, Rom>; evidence: RelocationEvidence[] } {
  const map = createOffsetMap(parent.rom, child.rom);
  const evidence: RelocationEvidence[] = [];
  const tableBytes = options.tableBytes ?? 1024;

  const relocateRom = (value: number, path: string): number => {
    const offset = value - ROM_BASE;
    const tried = map
      .candidatesAt(offset)
      .map((delta) => ({
        delta,
        score: compare(
          parent.rom,
          child.rom,
          map,
          offset,
          offset + delta,
          tableBytes,
        ),
      }))
      .sort((a, b) => b.score - a.score);
    const best = tried[0];
    if (!best) {
      evidence.push({
        path,
        from: value,
        to: null,
        method: "unresolved",
        score: 0,
        detail: "no anchors",
      });
      return value;
    }
    evidence.push({
      path,
      from: value,
      to: value + best.delta,
      method: "table",
      score: best.score,
      detail: `delta ${best.delta >= 0 ? "+" : "-"}0x${Math.abs(best.delta).toString(16)}`,
    });
    return value + best.delta;
  };
  const walk = (value: unknown, path: string): unknown => {
    if (typeof value === "number" && isRom(value))
      return relocateRom(value, path);
    if (Array.isArray(value))
      return value.map((item, i) => walk(item, `${path}[${i}]`));
    if (value && typeof value === "object")
      return Object.fromEntries(
        Object.entries(value).map(([key, item]) => [
          key,
          walk(item, `${path}.${key}`),
        ]),
      );
    return value;
  };

  // RAM: every literal-pool word in the parent that names work RAM, paired
  // with the word at the relocated position in the child.
  const pairs = new Map<number, Map<number, number>>();
  for (let at = 0; at + 4 <= parent.rom.length; at += 4) {
    const a = word(parent.rom, at);
    if (!isWorkRam(a)) continue;
    const to = at + map.deltaAt(at);
    if (to < 0 || to + 4 > child.rom.length) continue;
    const b = word(child.rom, to);
    if (!isWorkRam(b)) continue;
    const votes = pairs.get(a) ?? new Map<number, number>();
    votes.set(b, (votes.get(b) ?? 0) + 1);
    pairs.set(a, votes);
  }
  const literal = (address: number) => {
    const votes = pairs.get(address);
    if (!votes) return null;
    const total = [...votes.values()].reduce((sum, n) => sum + n, 0);
    const [to, n] = [...votes].sort((a, b) => b[1] - a[1])[0];
    return { to, n, total };
  };
  const known = [...pairs.keys()].sort((a, b) => a - b);
  const addresses = Object.fromEntries(
    Object.entries(parent.manifest.addresses as Record<string, number>).map(
      ([name, from]) => {
        const path = `addresses.${name}`;
        if (isRom(from)) return [name, relocateRom(from, path)];
        if (!isWorkRam(from)) {
          evidence.push({
            path,
            from,
            to: from,
            method: "unchanged",
            score: 1,
            detail: "not in ROM or work RAM",
          });
          return [name, from];
        }
        const exact = literal(from);
        if (exact) {
          evidence.push({
            path,
            from,
            to: exact.to,
            method: "literal",
            score: exact.n / exact.total,
            detail: `${exact.n}/${exact.total} literal pools agree`,
          });
          return [name, exact.to];
        }
        // No literal names it directly: use the neighbouring literals when
        // both moved by the same amount.
        const below = known.filter((a) => a < from).at(-1);
        const above = known.find((a) => a > from);
        const shift = (a: number | undefined) =>
          a === undefined ? null : literal(a)!.to - a;
        const d1 = shift(below),
          d2 = shift(above);
        if (d1 !== null && d1 === d2) {
          evidence.push({
            path,
            from,
            to: from + d1,
            method: "neighbour",
            score: 0.5,
            detail: `between literals 0x${below!.toString(16)} and 0x${above!.toString(16)}`,
          });
          return [name, from + d1];
        }
        evidence.push({
          path,
          from,
          to: null,
          method: "unresolved",
          score: 0,
          detail: "neighbours disagree",
        });
        return [name, from];
      },
    ),
  ) as Manifest<S, Rom>["addresses"];

  const manifest: Manifest<S, Rom> = {
    family: parent.manifest.family,
    romHash: child.romHash,
    // The parent's review claims do not carry over to a relocated build.
    source: {
      ...parent.manifest.source,
      options: [`Relocated from ${parent.manifest.romHash}; not reviewed`],
    },
    addresses,
    rom: walk(parent.manifest.rom, "rom") as Rom,
    provenance: { method: "relocated", parent: parent.manifest.romHash },
  };
  void family;
  return { manifest, evidence };
}

// ---------------------------------------------------------------------------
// Signatures: turn a verified build's code into locators for other builds.

/**
 * Finds Thumb code in `rom` that loads `address` (a RAM global or a ROM
 * table) from a literal pool, and returns locators whose masked pattern is
 * unique in this ROM. Literal offsets and branch targets are wildcards, so a
 * signature survives code moving around it.
 */
export function codeSignatures(
  rom: Uint8Array,
  address: number,
  options: {
    readonly before?: number;
    readonly after?: number;
    readonly limit?: number;
    /**
     * When no code loads `address` itself, also use code that loads a base up
     * to this many bytes below it (a field reached from a global); the
     * locator adds the offset back. Exact loads always come first.
     */
    readonly maxOffset?: number;
  } = {},
): Locator[] {
  const before = options.before ?? 6;
  const limit = options.limit ?? 3;
  const after = options.after ?? 10;
  const maxOffset = options.maxOffset ?? 0;
  // Literal-pool words naming `address` or a base just below it, closest first.
  const sites: { literal: number; distance: number }[] = [];
  for (let literal = 0; literal + 4 <= rom.length; literal += 4) {
    const distance = address - word(rom, literal);
    if (distance >= 0 && distance <= maxOffset)
      sites.push({ literal, distance });
  }
  sites.sort((a, b) => a.distance - b.distance || a.literal - b.literal);
  const found: Locator[] = [];
  for (const { literal, distance } of sites) {
    // A Thumb `ldr rN, [pc, #imm]` within 1 KiB before the literal.
    for (let at = Math.max(0, literal - 1020) & ~1; at < literal; at += 2) {
      const instruction = rom[at] | (rom[at + 1] << 8);
      if ((instruction & 0xf800) !== 0x4800) continue;
      if (((at + 4) & ~3) + (instruction & 0xff) * 4 !== literal) continue;
      const start = at - before;
      if (start < 0 || at + 2 + after > literal) continue;
      const tokens: string[] = [];
      for (let i = start; i < at + 2 + after; i += 2) {
        const low = rom[i],
          high = rom[i + 1];
        const value = low | (high << 8);
        const isLdrLiteral = (value & 0xf800) === 0x4800;
        const isBranch =
          (value & 0xf000) === 0xf000 ||
          (value & 0xf000) === 0xd000 ||
          (value & 0xf800) === 0xe000;
        if (isLdrLiteral) tokens.push("??", high.toString(16).padStart(2, "0"));
        else if (isBranch) tokens.push("??", "??");
        else
          tokens.push(
            low.toString(16).padStart(2, "0"),
            high.toString(16).padStart(2, "0"),
          );
      }
      const load = thumbLiteral(tokens.join(" "), at - start);
      const locator = distance ? inferred(offset(load, distance)) : load;
      const hits = locator.find(rom);
      if (hits.length === 1 && hits[0] === address) found.push(locator);
      if (found.length === limit) return found;
    }
  }
  return found;
}

/**
 * A locator for the data at `address` itself: its first bytes, with words
 * that are ROM pointers written as `ptr`, grown until the pattern is unique in
 * this ROM. Tables made only of pointers have too few fixed bytes and get none;
 * `codeSignatures` still finds them.
 */
export function dataSignatures(
  rom: Uint8Array,
  address: number,
  options: {
    readonly minBytes?: number;
    readonly maxBytes?: number;
    readonly minFixed?: number;
  } = {},
): Locator[] {
  const offset = address - ROM_BASE;
  if (offset < 0 || offset >= rom.length) return [];
  const aligned = offset % 4 === 0;
  const tokens: string[] = [];
  let fixed = 0;
  for (
    let length = 0;
    length < (options.maxBytes ?? 64) && offset + length + 4 <= rom.length;
    length += 4
  ) {
    const value = word(rom, offset + length);
    if (aligned && isRom(value)) tokens.push("ptr");
    else {
      for (let i = 0; i < 4; i++)
        tokens.push(rom[offset + length + i].toString(16).padStart(2, "0"));
      fixed += 4;
    }
    if (
      length + 4 < (options.minBytes ?? 16) ||
      fixed < (options.minFixed ?? 8)
    )
      continue;
    const locator = content(tokens.join(" "));
    const hits = locator.find(rom);
    if (hits.length === 1 && hits[0] === address) return [locator];
  }
  return [];
}

/** Masked Thumb code from `start` to `end`: literal offsets, branch targets and `wild` are `??`. */
function codeTokens(
  rom: Uint8Array,
  start: number,
  end: number,
  wild = -1,
): string {
  const tokens: string[] = [];
  for (let i = start; i < end; i += 2) {
    const low = rom[i],
      high = rom[i + 1];
    const value = low | (high << 8);
    const hex = (b: number) => b.toString(16).padStart(2, "0");
    if (i === wild) tokens.push("??", hex(high));
    else if ((value & 0xf800) === 0x4800) tokens.push("??", hex(high));
    else if (
      (value & 0xf000) === 0xf000 ||
      (value & 0xf000) === 0xd000 ||
      (value & 0xf800) === 0xe000
    )
      tokens.push("??", "??");
    else tokens.push(hex(low), hex(high));
  }
  return tokens.join(" ");
}

/**
 * Locators for a constant such as a table's count, from code in `rom` that
 * uses it: literal loads, and Thumb `mov`/`cmp` immediates (with a following
 * `lsls` for larger values) whose immediate is a wildcard, so a changed count
 * is read rather than missed. Code that uses the value minus one (a bounds
 * check against the last ID) gives inferred locators, ranked after exact ones.
 */
export function valueSignatures(
  rom: Uint8Array,
  value: number,
  options: {
    readonly before?: number;
    readonly after?: number;
    readonly limit?: number;
  } = {},
): Locator[] {
  const before = options.before ?? 6;
  const after = options.after ?? 10;
  const limit = options.limit ?? 3;
  const exact: Locator[] = [];
  const lastId: Locator[] = [];
  const add = (list: Locator[], locator: Locator) => {
    if (list.length < limit && !list.some((l) => l.id === locator.id))
      list.push(locator);
  };
  for (const load of codeSignatures(rom, value, { before, after, limit }))
    add(exact, load);
  if (value > 0)
    for (const load of codeSignatures(rom, value - 1, { before, after, limit }))
      add(lastId, inferred(offset(load, 1)));
  let attempts = 0;
  for (
    let at = before;
    at + 2 + after <= rom.length && exact.length < limit && attempts < 400;
    at += 2
  ) {
    const high = rom[at + 1];
    // Format 3, `mov` (op 0) or `cmp` (op 1) with an 8-bit immediate.
    if ((high & 0xe0) !== 0x20 || (high & 0x18) > 0x08) continue;
    const built = rom[at] << shiftAfter(rom, at);
    if (built !== value && built !== value - 1) continue;
    attempts++;
    const pattern = codeTokens(rom, at - before, at + 2 + after, at);
    // The immediate is a wildcard, so the code around it must be unique.
    if (matches(rom, parsePattern(pattern), 2).length !== 1) continue;
    const locator = thumbImmediate(pattern, before);
    if (built === value) add(exact, locator);
    else add(lastId, inferred(offset(locator, 1)));
  }
  return [...exact, ...lastId].slice(0, limit);
}

/**
 * Locators for an address that code reaches from a global plus an offset or
 * through an index, which no literal names: `thumbAccess` over the code that
 * computes it, unique in `rom`. They find the address itself, so they are not
 * inferred.
 */
export function accessSignatures(
  rom: Uint8Array,
  address: number,
  options: {
    readonly window?: number;
    readonly after?: number;
    readonly limit?: number;
  } = {},
): Locator[] {
  const window = (options.window ?? 8) * 2;
  const after = options.after ?? 4;
  const limit = options.limit ?? 3;
  const found: Locator[] = [];
  let attempts = 0;
  for (
    let at = window;
    at + 2 + after <= rom.length && found.length < limit && attempts < 200;
    at += 2
  ) {
    const high = rom[at + 1];
    const access = high >= 0x50 && high < 0x90; // register- or immediate-offset load/store
    if (!access || accessAt(rom, at - window, at) !== address) continue;
    attempts++;
    const locator = thumbAccess(
      codeTokens(rom, at - window, at + 2 + after),
      window,
    );
    const hits = locator.find(rom);
    if (hits.length === 1 && hits[0] === address) found.push(locator);
  }
  return found;
}

/**
 * Locators for a table of pointers, which has no fixed bytes of its own:
 * through an entry's target, `offset(references(content(target)), -4 * entry)`.
 */
export function pointerSignatures(
  rom: Uint8Array,
  address: number,
  options: { readonly entries?: number } = {},
): Locator[] {
  const table = address - ROM_BASE;
  if (table < 0 || table % 4) return [];
  for (let entry = 0; entry < (options.entries ?? 8); entry++) {
    if (table + entry * 4 + 4 > rom.length) break;
    const target = word(rom, table + entry * 4);
    if (!isRom(target)) continue;
    const [data] = dataSignatures(rom, target);
    if (!data) continue;
    const locator = offset(references(data), -4 * entry);
    const hits = locator.find(rom);
    if (hits.length === 1 && hits[0] === address) return [locator];
  }
  return [];
}

/** Whether a ROM looks like one of a family's builds, before locating it fully. */
export interface Detection {
  readonly family: string;
  /** Share of marker locators that find something, 0 when the header rules it out. */
  readonly score: number;
  /** The header's game code matched the family's, or null when it lists none. */
  readonly header: boolean | null;
  readonly markers: readonly {
    readonly locator: string;
    readonly found: boolean;
  }[];
}
/**
 * A cheap check: the cartridge header's game code, then a few of the
 * family's own locators (its `detect.markers`, or the first locator of its
 * first `markers` sources) must find something.
 */
export function detectFamily(
  family: Family<any, any, any, any>,
  rom: Uint8Array,
  options: { readonly markers?: number } = {},
): Detection {
  const code = String.fromCharCode(...rom.subarray(0xac, 0xb0));
  const codes = family.detect?.gameCodes;
  const header = codes ? codes.includes(code) : null;
  if (header === false)
    return { family: family.id, score: 0, header, markers: [] };
  const chosen =
    family.detect?.markers ??
    Object.values(family.sources as Sources)
      .flatMap((source) =>
        (source.locate ?? [])
          .filter((locator) => !locator.inferred)
          .slice(0, 1),
      )
      .slice(0, options.markers ?? 6);
  const markers = chosen.map((locator) => ({
    locator: locator.id,
    found: locator.find(rom).length > 0,
  }));
  const score = markers.length
    ? markers.filter(({ found }) => found).length / markers.length
    : header
      ? 1
      : 0;
  return { family: family.id, score, header, markers };
}
