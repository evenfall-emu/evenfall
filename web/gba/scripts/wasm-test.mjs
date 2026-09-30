import { readFile } from "node:fs/promises";
import { test } from "vite-plus/test";

export const wasm = await readFile(
  new URL("../evenfall-gba.wasm", import.meta.url),
).catch((error) => {
  if (error.code === "ENOENT") return null;
  throw error;
});

export const scalarWasm = await readFile(
  new URL("../evenfall-gba-scalar.wasm", import.meta.url),
).catch((error) => {
  if (error.code === "ENOENT") return null;
  throw error;
});

const missing = "Rust WASM not built: web/gba/evenfall-gba.wasm";

export const wasmTest = wasm
  ? test
  : (name) => test(name, (context) => context.skip(missing));
