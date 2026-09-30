import { CYCLES_PER_FRAME, WasmCore } from "@evenfall/gba/core";

const snapshots = new Map<string, { data: string; cycle: string }>();

/** Generate immutable test inputs once per worker, without booting another UI. */
export async function stateAt(rom: Uint8Array, cycle: bigint) {
  const key = `${Buffer.from(rom).toString("hex")}:${cycle}`;
  let state = snapshots.get(key);
  if (!state) {
    const core = await WasmCore.create();
    try {
      core.load(rom, { rtcEpoch: 123n });
      while (core.currentCycle() < cycle) {
        const next = core.currentCycle() + CYCLES_PER_FRAME;
        const result = core.runUntil(next < cycle ? next : cycle);
        if (result.reason === "faulted" || result.reason === "power_stopped")
          throw new Error(`Fixture execution stopped: ${result.reason}`);
        core.drainAudio();
      }
      state = {
        // One string avoids Playwright serializing every byte as an object.
        data: JSON.stringify(Array.from(core.saveState())),
        cycle: core.currentCycle().toString(),
      };
      snapshots.set(key, state);
    } finally {
      core.dispose();
    }
  }
  // Strings are immutable; each test owns its wrapper.
  return { ...state };
}
