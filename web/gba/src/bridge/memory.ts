import type { MemoryLayout, MemoryReader, MemoryRef } from "./contract.js";

export interface MemoryRegion {
  readonly address: number;
  readonly bytes: Uint8Array;
}

/** Copies caller-owned regions once; neither reads nor decoders can change them. */
export function createMemoryReader(
  regions: readonly MemoryRegion[],
): MemoryReader {
  const copies = regions
    .map(({ address, bytes }) => {
      checkRange(address, bytes.length);
      return { address, bytes: new Uint8Array(bytes) };
    })
    .sort((a, b) => a.address - b.address);
  for (let i = 1; i < copies.length; i++) {
    if (copies[i].address < copies[i - 1].address + copies[i - 1].bytes.length)
      throw new RangeError("Overlapping captured memory regions");
  }
  function view(address: number, length: number): DataView {
    checkRange(address, length);
    if (length > 1024 * 1024) throw new RangeError("Memory read exceeds 1 MiB");
    const region = copies.find(
      (region) =>
        address >= region.address &&
        address + length <= region.address + region.bytes.length,
    );
    if (!region)
      throw new RangeError(
        `Uncaptured memory at 0x${address.toString(16)} (${length} bytes)`,
      );
    return new DataView(region.bytes.buffer, address - region.address, length);
  }
  const memory: MemoryReader = {
    u8: (address) => view(address, 1).getUint8(0),
    u16: (address) => view(address, 2).getUint16(0, true),
    u32: (address) => view(address, 4).getUint32(0, true),
    bytes(address, length) {
      const data = view(address, length);
      return new Uint8Array(data.buffer, data.byteOffset, length).slice();
    },
    read<T>({ address, layout }: MemoryRef<T>): T {
      view(address, layout.byteLength);
      return layout.decode(memory, address);
    },
  };
  return Object.freeze(memory);
}

function checkRange(address: number, length: number): void {
  if (
    !Number.isInteger(address) ||
    address < 0 ||
    address > 0xffffffff ||
    !Number.isInteger(length) ||
    length < 0 ||
    address + length > 0x100000000
  )
    throw new RangeError("Invalid guest memory range");
}

export function ref<T>(address: number, layout: MemoryLayout<T>): MemoryRef<T> {
  checkRange(address, layout.byteLength);
  return Object.freeze({ address, layout });
}

export const u8: MemoryLayout<number> = Object.freeze({
  byteLength: 1,
  decode: (memory: MemoryReader, address: number) => memory.u8(address),
});

/** Follow explicitly with memory.read; each hop remains in the same snapshot. */
export function pointer<T>(
  layout: MemoryLayout<T>,
): MemoryLayout<MemoryRef<T> | null> {
  return {
    byteLength: 4,
    decode(memory, address) {
      const target = memory.u32(address);
      if (!target) return null;
      if (target % 4)
        throw new RangeError(`Unaligned pointer at 0x${address.toString(16)}`);
      return ref(target, layout);
    },
  };
}

export interface MemoryArrayRef<T> extends MemoryRef<readonly T[]> {
  readonly element: MemoryLayout<T>;
  readonly length: number;
}

/** A bounded array root. Use at() to avoid decoding unused slots. */
export function array<T>(
  address: number,
  element: MemoryLayout<T>,
  length: number,
): MemoryArrayRef<T> {
  if (
    !Number.isInteger(element.byteLength) ||
    element.byteLength <= 0 ||
    !Number.isInteger(length) ||
    length < 0 ||
    length * element.byteLength > 1024 * 1024
  )
    throw new RangeError("Invalid memory array size");
  const reference: MemoryArrayRef<T> = Object.freeze({
    ...ref(address, {
      byteLength: element.byteLength * length,
      decode: (memory: MemoryReader) =>
        Array.from({ length }, (_, index) => memory.read(at(reference, index))),
    }),
    element,
    length,
  });
  return reference;
}

export function at<T>(
  reference: MemoryArrayRef<T>,
  index: number,
): MemoryRef<T> {
  if (!Number.isInteger(index) || index < 0 || index >= reference.length)
    throw new RangeError("Memory array index out of bounds");
  return ref(
    reference.address + index * reference.element.byteLength,
    reference.element,
  );
}
