import { describe, expect, it } from "vitest";
import {
  applyBps,
  applyIps,
  applyPatch,
  applyUps,
  crc32,
  PatchError,
} from "../src/patch";

const text = (value: string) => new TextEncoder().encode(value);
const show = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

/** BPS and UPS variable-length number. */
function number(value: number) {
  const out: number[] = [];
  for (;;) {
    const low = value % 128;
    value = Math.floor(value / 128);
    if (value === 0) {
      out.push(0x80 | low);
      return out;
    }
    out.push(low);
    value--;
  }
}

const u32 = (value: number) => [0, 8, 16, 24].map((s) => (value >>> s) & 0xff);

/** Appends source, target and patch CRC32s. */
function seal(body: number[], source: Uint8Array, target: Uint8Array) {
  const head = [...body, ...u32(crc32(source)), ...u32(crc32(target))];
  return new Uint8Array([...head, ...u32(crc32(new Uint8Array(head)))]);
}

const source = text("ABCDEFGH");

describe("crc32", () => {
  it("matches the standard check value", () => {
    expect(crc32(text("123456789"))).toBe(0xcbf43926);
  });
});

describe("BPS", () => {
  const target = text("ABxyzFGHxyzFFFF");
  const action = (command: number, length: number) =>
    number(((length - 1) << 2) | command);
  const offset = (delta: number) =>
    number((Math.abs(delta) << 1) | (delta < 0 ? 1 : 0));
  const body = [
    ...text("BPS1"),
    ...number(source.length),
    ...number(target.length),
    ...number(3),
    ...text("m=1"),
    ...action(0, 2), // SourceRead "AB"
    ...action(1, 3),
    ...text("xyz"), // TargetRead
    ...action(2, 3),
    ...offset(5), // SourceCopy "FGH"
    ...action(3, 4),
    ...offset(2), // TargetCopy "xyzF"
    ...action(3, 3),
    ...offset(5), // TargetCopy from 11, overlapping: "FFF"
  ];
  const patch = seal(body, source, target);

  it("applies every action, including overlapping target copies", () => {
    expect(show(applyBps(source, patch))).toBe(show(target));
    expect(show(applyPatch("bps", source, patch))).toBe(show(target));
  });

  it("refuses another base, a damaged patch and a wrong result", () => {
    expect(() => applyBps(text("ABCDEFGX"), patch)).toThrow(
      "different base ROM",
    );
    const damaged = patch.slice();
    damaged[20] ^= 1;
    expect(() => applyBps(source, damaged)).toThrow("damaged");
    expect(() =>
      applyBps(source, seal(body, source, text("ABxyzFGHxyzFFFx"))),
    ).toThrow("checksum");
    expect(() => applyUps(source, patch)).toThrow(PatchError);
  });

  it("refuses reads past the base ROM", () => {
    const outside = [
      ...text("BPS1"),
      ...number(source.length),
      ...number(4),
      ...number(0),
      ...action(2, 4),
      ...offset(6),
    ];
    expect(() => applyBps(source, seal(outside, source, text("GH??")))).toThrow(
      "past the base ROM",
    );
  });
});

describe("UPS", () => {
  it("XORs changed runs and zero-fills a longer target", () => {
    const target = text("ABcDEFGHij");
    const patch = seal(
      [
        ...text("UPS1"),
        ...number(source.length),
        ...number(target.length),
        ...number(2),
        0x43 ^ 0x63,
        0,
        ...number(4),
        ...text("ij"),
        0,
      ],
      source,
      target,
    );
    expect(show(applyUps(source, patch))).toBe("ABcDEFGHij");
    expect(() => applyUps(text("abcdefgh"), patch)).toThrow(
      "different base ROM",
    );
  });
});

describe("IPS", () => {
  const records = [
    ...text("PATCH"),
    ...[0, 0, 1, 0, 2],
    ...text("xy"),
    ...[0, 0, 8, 0, 0, 0, 3],
    ...text("z"),
    ...text("EOF"),
  ];

  it("writes records and runs, growing the ROM", () => {
    expect(show(applyIps(source, new Uint8Array(records)))).toBe("AxyDEFGHzzz");
  });

  it("truncates to a size after EOF", () => {
    expect(show(applyIps(source, new Uint8Array([...records, 0, 0, 4])))).toBe(
      "AxyD",
    );
  });

  it("refuses a truncated patch", () => {
    expect(() =>
      applyIps(source, new Uint8Array(records.slice(0, 12))),
    ).toThrow("truncated");
  });
});
