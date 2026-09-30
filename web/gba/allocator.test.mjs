import { test } from "vite-plus/test";
import assert from "node:assert/strict";
import { wasm } from "./scripts/wasm-test.mjs";

test(
  "Rust wasm allocator grows by actual large allocation size and reuses released space",
  { skip: !wasm },
  async () => {
    const module = new WebAssembly.Module(wasm);
    assert.deepEqual(WebAssembly.Module.imports(module), []);
    const { exports: core } = new WebAssembly.Instance(module, {});
    const mib = 1024 * 1024;
    assert.equal(core.memory.buffer.byteLength, 16 * mib);
    for (const size of [mib - 1, 32 * mib + 1]) {
      assert.equal(core.ef_prepare(size), 0);
      const address = core.ef_input_ptr();
      const first = core.memory.buffer.byteLength;
      if (size > mib)
        assert.ok(
          first < 50 * mib,
          `a 32 MiB + 1 byte buffer should not occupy 64 MiB: ${first}`,
        );
      new Uint8Array(core.memory.buffer)[address + size - 1] = 0xa5;
      for (let i = 0; i < 6; i++) {
        core.ef_dispose();
        assert.equal(core.ef_prepare(size), 0);
        assert.equal(core.ef_input_ptr(), address);
        assert.equal(core.memory.buffer.byteLength, first);
      }
      core.ef_dispose();
    }
  },
);

test(
  "Rust ABI reuses freed large buffers, preserves live bytes across growth, and survives OOM",
  { skip: !wasm },
  () => {
    // Checks the allocator only through observable behavior, not through
    // its internal bitmap or resize details.
    const { exports: core } = new WebAssembly.Instance(
      new WebAssembly.Module(wasm),
      {},
    );
    const mib = 1024 * 1024;
    const error = () =>
      new TextDecoder().decode(
        new Uint8Array(
          core.memory.buffer,
          core.ef_error_ptr(),
          core.ef_error_len(),
        ),
      );
    assert.equal(core.ef_prepare(4), 0);
    new Uint8Array(core.memory.buffer, core.ef_input_ptr(), 4).set([
      0xfe, 0xff, 0xff, 0xea,
    ]);
    assert.equal(core.ef_load(4, 0, 0, 0, -1, -1, 0n, 48000), 0);
    const large = 32 * mib + 1;
    assert.equal(core.ef_prepare(large), 0);
    const first = core.ef_input_ptr();
    const bytes = new Uint8Array(core.memory.buffer);
    bytes[first] = 0x5a;
    bytes[first + 2 * mib + 17] = 0xa5;
    bytes[first + large - 1] = 0x3c;
    core.memory.grow(1); // Old typed-array views are detached by WASM growth.
    const grown = new Uint8Array(core.memory.buffer);
    assert.deepEqual(
      [grown[first], grown[first + 2 * mib + 17], grown[first + large - 1]],
      [0x5a, 0xa5, 0x3c],
    );
    core.ef_dispose();
    const high = core.memory.buffer.byteLength;
    assert.equal(core.ef_prepare(large), 0);
    assert.equal(core.memory.buffer.byteLength, high);
    core.ef_dispose();

    assert.equal(core.ef_prepare(4), 0);
    new Uint8Array(core.memory.buffer, core.ef_input_ptr(), 4).set([
      0xfe, 0xff, 0xff, 0xea,
    ]);
    assert.equal(core.ef_load(4, 0, 0, 0, -1, -1, 0n, 48000), 0);
    assert.equal(core.ef_prepare(large), 0); // Keep the large run live.

    // Exhaust the *actual* bounded linear memory without exceeding the ABI's
    // 68 MiB input limit: the next valid allocation must fail, not abort.
    const pages = (512 * mib - core.memory.buffer.byteLength) / 65536;
    core.memory.grow(pages);
    assert.equal(core.ef_prepare(large + 1), -1);
    assert.equal(error(), "OutOfMemory");
    assert.equal(core.ef_prepare(4), 0); // Session and freed space still work.
    assert.equal(core.ef_export_rom(), 0);
    assert.deepEqual(
      [
        ...new Uint8Array(
          core.memory.buffer,
          core.ef_output_ptr(),
          core.ef_output_len(),
        ),
      ],
      [0xfe, 0xff, 0xff, 0xea],
    );
    core.ef_dispose();
  },
);

test(
  "caller-sized read, state, and bundle outputs fail fallibly while the session remains usable",
  { skip: !wasm },
  () => {
    for (const [occupiedMiB, stateFails] of [
      [6.25, false],
      [7, true],
    ]) {
      const { exports: core } = new WebAssembly.Instance(
        new WebAssembly.Module(wasm),
        {},
      );
      const mib = 1024 * 1024;
      const error = () =>
        new TextDecoder().decode(
          new Uint8Array(
            core.memory.buffer,
            core.ef_error_ptr(),
            core.ef_error_len(),
          ),
        );
      assert.equal(core.ef_prepare(4), 0);
      new Uint8Array(core.memory.buffer, core.ef_input_ptr(), 4).set([
        0xfe, 0xff, 0xff, 0xea,
      ]);
      assert.equal(core.ef_load(4, 0, 0, 0, -1, -1, 0n, 48000), 0);
      assert.equal(core.ef_prepare(occupiedMiB * mib), 0);
      core.memory.grow((512 * mib - core.memory.buffer.byteLength) / 65536);

      assert.equal(core.ef_write_rewind_bundle(), -1);
      assert.equal(error(), "OutOfMemory");
      assert.equal(core.ef_write_state(), stateFails ? -1 : 0);
      if (stateFails) assert.equal(error(), "OutOfMemory");
      assert.equal(core.ef_read_memory(0x02000000, mib), -1);
      assert.equal(error(), "OutOfMemory");
      assert.equal(core.ef_cycle(), 0n);
      assert.equal(core.ef_export_rom(), 0);
      assert.deepEqual(
        [
          ...new Uint8Array(
            core.memory.buffer,
            core.ef_output_ptr(),
            core.ef_output_len(),
          ),
        ],
        [0xfe, 0xff, 0xff, 0xea],
      );
      assert.equal(core.ef_prepare(4), 0); // Releases the large live upload.
      assert.equal(core.ef_write_state(), 0);
      assert.ok(core.ef_output_len() > 100);
      assert.equal(core.ef_read_memory(0x02000000, 4), 0);
      core.ef_dispose();
    }
  },
);
