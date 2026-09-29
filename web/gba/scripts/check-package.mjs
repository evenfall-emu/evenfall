// prepack guard: refuse to pack an SDK without its WASM, with a debug
// (DWARF) build, or with an ABI this JavaScript does not speak.
import { readFile } from "node:fs/promises";

const fail = (message) => {
  console.error(`@evenfall/gba: ${message}`);
  process.exit(1);
};
const wasm = await readFile(
  new URL("../evenfall-gba.wasm", import.meta.url),
).catch(() =>
  fail("evenfall-gba.wasm is missing; build the Rust WASM core first"),
);
if (Buffer.from(wasm).includes(".debug_info"))
  fail("evenfall-gba.wasm carries DWARF; rebuild a stripped WASM binary");
const { instance } = await WebAssembly.instantiate(wasm, {});
if (instance.exports.zb_version() !== 1)
  fail("evenfall-gba.wasm ABI does not match core.js");
await readFile(new URL("../bridge/index.js", import.meta.url)).catch(() =>
  fail("bridge/ is not built"),
);
console.log(`@evenfall/gba: evenfall-gba.wasm ${wasm.length} bytes, ABI 1`);
