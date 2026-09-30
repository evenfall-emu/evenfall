/**
 * ROM patch formats: BPS, UPS and IPS. Each applier checks what its format
 * can check (sizes, and CRC32s for BPS and UPS); the caller still verifies
 * the output's SHA-1, which IPS cannot.
 */
export type PatchFormat = "bps" | "ups" | "ips";

/** Largest cartridge ROM the emulator accepts. */
export const MAX_ROM_SIZE = 32 * 1024 * 1024;

export class PatchError extends Error {}

const crcTable = Array.from({ length: 256 }, (_, index) => {
  let crc = index;
  for (let bit = 0; bit < 8; bit++)
    crc = crc & 1 ? 0xedb88320 ^ (crc >>> 1) : crc >>> 1;
  return crc >>> 0;
});

export function crc32(bytes: Uint8Array) {
  let crc = 0xffffffff;
  for (const byte of bytes) crc = crcTable[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

/** A forward-only cursor over patch bytes that fails instead of overrunning. */
class Reader {
  offset = 0;
  constructor(
    readonly bytes: Uint8Array,
    readonly end = bytes.length,
  ) {}
  byte() {
    if (this.offset >= this.end)
      throw new PatchError("The patch is truncated.");
    return this.bytes[this.offset++];
  }
  take(length: number) {
    if (length > this.end - this.offset)
      throw new PatchError("The patch is truncated.");
    return this.bytes.subarray(this.offset, (this.offset += length));
  }
  /** BPS and UPS variable-length number: 7 bits per byte, high bit ends. */
  number() {
    let value = 0;
    let shift = 1;
    for (;;) {
      const byte = this.byte();
      value += (byte & 0x7f) * shift;
      if (byte & 0x80) break;
      shift *= 128;
      value += shift;
      if (value > Number.MAX_SAFE_INTEGER / 256)
        throw new PatchError("The patch has an out-of-range number.");
    }
    return value;
  }
  /** Big-endian unsigned integer of `length` bytes. */
  big(length: number) {
    let value = 0;
    for (const byte of this.take(length)) value = value * 256 + byte;
    return value;
  }
}

const u32le = (bytes: Uint8Array, offset: number) =>
  (bytes[offset] |
    (bytes[offset + 1] << 8) |
    (bytes[offset + 2] << 16) |
    (bytes[offset + 3] << 24)) >>>
  0;

function checkMagic(patch: Uint8Array, magic: string, format: string) {
  if (
    patch.length < magic.length ||
    [...magic].some((char, index) => patch[index] !== char.charCodeAt(0))
  )
    throw new PatchError(`This is not a ${format} patch.`);
}

function checkTargetSize(size: number) {
  if (size === 0 || size > MAX_ROM_SIZE)
    throw new PatchError("The patch produces a ROM of an unsupported size.");
}

/**
 * BPS and UPS end with the source, target and patch CRC32s. The patch CRC
 * covers every byte before it.
 */
function footer(patch: Uint8Array, format: string) {
  if (patch.length < 4 + 12) throw new PatchError("The patch is truncated.");
  const end = patch.length - 12;
  if (crc32(patch.subarray(0, patch.length - 4)) !== u32le(patch, end + 8))
    throw new PatchError(`The ${format} patch is damaged.`);
  return { end, source: u32le(patch, end), target: u32le(patch, end + 4) };
}

function checkSource(source: Uint8Array, size: number, crc: number) {
  if (source.length !== size || crc32(source) !== crc)
    throw new PatchError("This patch is for a different base ROM.");
}

function checkTarget(target: Uint8Array, crc: number) {
  if (crc32(target) !== crc)
    throw new PatchError("The patched ROM failed its checksum.");
}

export function applyBps(source: Uint8Array, patch: Uint8Array) {
  checkMagic(patch, "BPS1", "BPS");
  const crcs = footer(patch, "BPS");
  const read = new Reader(patch, crcs.end);
  read.offset = 4;
  const sourceSize = read.number();
  const targetSize = read.number();
  read.take(read.number()); // Metadata, unused.
  checkSource(source, sourceSize, crcs.source);
  checkTargetSize(targetSize);
  const target = new Uint8Array(targetSize);
  let output = 0;
  let sourceOffset = 0;
  let targetOffset = 0;
  while (read.offset < read.end) {
    const action = read.number();
    const length = Math.floor(action / 4) + 1;
    if (length > targetSize - output)
      throw new PatchError("The patch writes past the end of the ROM.");
    switch (action % 4) {
      case 0: // SourceRead: the source's bytes at the same offset.
        if (output + length > source.length)
          throw new PatchError("The patch reads past the base ROM.");
        target.set(source.subarray(output, output + length), output);
        break;
      case 1: // TargetRead: bytes stored in the patch.
        target.set(read.take(length), output);
        break;
      case 2: {
        // SourceCopy: source bytes at a relative, signed offset.
        const delta = read.number();
        sourceOffset += (delta % 2 ? -1 : 1) * Math.floor(delta / 2);
        if (sourceOffset < 0 || sourceOffset + length > source.length)
          throw new PatchError("The patch reads past the base ROM.");
        target.set(
          source.subarray(sourceOffset, sourceOffset + length),
          output,
        );
        sourceOffset += length;
        break;
      }
      default: {
        // TargetCopy: earlier output, byte by byte, so runs may overlap.
        const delta = read.number();
        targetOffset += (delta % 2 ? -1 : 1) * Math.floor(delta / 2);
        if (targetOffset < 0 || targetOffset >= output)
          throw new PatchError("The patch copies output it has not written.");
        for (let index = 0; index < length; index++)
          target[output + index] = target[targetOffset++];
      }
    }
    output += length;
  }
  if (output !== targetSize)
    throw new PatchError("The patch does not fill the ROM.");
  checkTarget(target, crcs.target);
  return target;
}

export function applyUps(source: Uint8Array, patch: Uint8Array) {
  checkMagic(patch, "UPS1", "UPS");
  const crcs = footer(patch, "UPS");
  const read = new Reader(patch, crcs.end);
  read.offset = 4;
  const sourceSize = read.number();
  const targetSize = read.number();
  checkSource(source, sourceSize, crcs.source);
  checkTargetSize(targetSize);
  // Unchanged bytes are the source's; past its end they are zero.
  const target = new Uint8Array(targetSize);
  target.set(source.subarray(0, targetSize));
  let offset = 0;
  while (read.offset < read.end) {
    offset += read.number();
    // XOR bytes run until a zero, which also covers one unchanged byte.
    for (;;) {
      const xor = read.byte();
      if (xor === 0) break;
      if (offset >= targetSize)
        throw new PatchError("The patch writes past the end of the ROM.");
      target[offset] = (source[offset] ?? 0) ^ xor;
      offset++;
    }
    offset++;
  }
  checkTarget(target, crcs.target);
  return target;
}

export function applyIps(source: Uint8Array, patch: Uint8Array) {
  checkMagic(patch, "PATCH", "IPS");
  const read = new Reader(patch);
  read.offset = 5;
  const records: { offset: number; bytes: Uint8Array | [number, number] }[] =
    [];
  let size = source.length;
  for (;;) {
    const offset = read.big(3);
    if (offset === 0x454f46) break; // "EOF"
    const length = read.big(2);
    // A zero length is a run: a 16-bit count of one repeated byte.
    const bytes: Uint8Array | [number, number] = length
      ? read.take(length)
      : [read.big(2), read.byte()];
    const extent = offset + (bytes instanceof Uint8Array ? length : bytes[0]);
    size = Math.max(size, extent);
    records.push({ offset, bytes });
  }
  // An optional 24-bit size after EOF truncates the output.
  if (read.end - read.offset === 3) size = read.big(3);
  else if (read.offset !== read.end)
    throw new PatchError("The IPS patch has trailing data.");
  checkTargetSize(size);
  const target = new Uint8Array(size);
  target.set(source.subarray(0, size));
  for (const { offset, bytes } of records) {
    if (offset >= size) continue;
    if (bytes instanceof Uint8Array)
      target.set(bytes.subarray(0, size - offset), offset);
    else target.fill(bytes[1], offset, Math.min(size, offset + bytes[0]));
  }
  return target;
}

export function applyPatch(
  format: PatchFormat,
  source: Uint8Array,
  patch: Uint8Array,
) {
  return { bps: applyBps, ups: applyUps, ips: applyIps }[format](source, patch);
}
