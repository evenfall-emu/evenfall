import { test } from "node:test";
import assert from "node:assert/strict";
import { wasm } from "./scripts/wasm-test.mjs";

test(
  "Rust wasm allocator grows by actual large allocation size and reuses released space",
  { skip: !wasm && "Rust WASM not built: web/gba/evenfall-gba.wasm" },
  async () => {
    const module = new WebAssembly.Module(wasm);
    assert.deepEqual(WebAssembly.Module.imports(module), []);
    const { exports: core } = new WebAssembly.Instance(module, {});
    const mib = 1024 * 1024;
    assert.equal(core.memory.buffer.byteLength, 16 * mib);
    for (const size of [mib - 1, 32 * mib + 1]) {
      assert.equal(core.zb_prepare(size), 0);
      const address = core.zb_input_ptr();
      const first = core.memory.buffer.byteLength;
      if (size > mib)
        assert.ok(
          first < 50 * mib,
          `a 32 MiB + 1 byte buffer should not occupy 64 MiB: ${first}`,
        );
      new Uint8Array(core.memory.buffer)[address + size - 1] = 0xa5;
      for (let i = 0; i < 6; i++) {
        core.zb_dispose();
        assert.equal(core.zb_prepare(size), 0);
        assert.equal(core.zb_input_ptr(), address);
        assert.equal(core.memory.buffer.byteLength, first);
      }
      core.zb_dispose();
    }
  },
);
