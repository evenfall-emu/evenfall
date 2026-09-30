// prepack guard: refuse to pack an SDK without both WASM modules, with a debug
// (DWARF) build, or with an ABI this JavaScript does not speak.
import { readFile } from "node:fs/promises";

const fail = (message) => {
  console.error(`@evenfall/gba: ${message}`);
  process.exit(1);
};
for (const name of ["evenfall-gba.wasm", "evenfall-gba-scalar.wasm"]) {
  const wasm = await readFile(new URL(`../${name}`, import.meta.url)).catch(
    () => fail(`${name} is missing; build the Rust WASM core first`),
  );
  if (Buffer.from(wasm).includes(".debug_info"))
    fail(`${name} carries DWARF; rebuild a stripped WASM binary`);
  const { instance } = await WebAssembly.instantiate(wasm, {});
  if (instance.exports.ef_version() !== 1)
    fail(`${name} ABI does not match core.js`);
  console.log(`@evenfall/gba: ${name} ${wasm.length} bytes, ABI 1`);
}
await readFile(new URL("../bridge/index.js", import.meta.url)).catch(() =>
  fail("bridge/ is not built"),
);
