import { wasmTest as test } from "../scripts/wasm-test.mjs";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

test("CLI keeps exit codes distinct, never overwrites evidence, and replays failures against their identity", async () => {
  const directory = await mkdtemp(join(tmpdir(), "evenfall-devtools-cli-"));
  const run = (args) =>
    spawnSync(process.execPath, args, { encoding: "utf8", timeout: 30000 });
  try {
    const fixture = join(directory, "fixture");
    execFileSync(
      process.execPath,
      ["scripts/run-scenario.mjs", "--example", fixture],
      { timeout: 30000 },
    );
    const manifestPath = join(fixture, "scenario.json");
    const success = run([
      "scripts/run-scenario.mjs",
      manifestPath,
      join(directory, "passed"),
    ]);
    assert.equal(success.status, 0, success.stderr);
    const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
    manifest.checkpoints[0].expect.cpu.registers[0] = 43;
    await writeFile(manifestPath, JSON.stringify(manifest));
    const output = join(directory, "failed");
    const failure = run(["scripts/run-scenario.mjs", manifestPath, output]);
    assert.equal(failure.status, 1, failure.stderr);
    const resultPath = join(output, "result.json");
    const report = JSON.parse(await readFile(resultPath, "utf8"));
    assert.equal(report.status, "failed");
    assert.equal(report.capture.history.replayable, true);
    assert.equal(
      (await readFile(join(output, "frame.bgr555le"))).length,
      76800,
    );
    const replay = run([
      "scripts/replay-capture.mjs",
      resultPath,
      manifestPath,
    ]);
    assert.equal(replay.status, 0, replay.stderr);
    const reproduced = JSON.parse(replay.stdout);
    assert.equal(reproduced.status, "reproduced");
    assert.equal(reproduced.inspection.registers[0], 42);
    const previous = await readFile(resultPath, "utf8");
    assert.equal(
      run(["scripts/run-scenario.mjs", manifestPath, output]).status,
      2,
    );
    assert.equal(await readFile(resultPath, "utf8"), previous);

    report.capture.identity.build.wasmSha256 = "0".repeat(64);
    const tampered = join(directory, "wrong-build.json");
    await writeFile(tampered, JSON.stringify(report));
    const mismatch = run([
      "scripts/replay-capture.mjs",
      tampered,
      manifestPath,
    ]);
    assert.equal(mismatch.status, 2);
    assert.match(mismatch.stderr, /identity mismatch/);

    const invalid = join(fixture, "invalid.json");
    await writeFile(
      invalid,
      JSON.stringify({
        ...manifest,
        inputs: [{ ...manifest.inputs[0], at: 100 }],
      }),
    );
    const rejected = run([
      "scripts/run-scenario.mjs",
      invalid,
      join(directory, "invalid"),
    ]);
    assert.equal(rejected.status, 2);
    assert.match(rejected.stderr, /expected decimal u64 string/);

    await writeFile(join(fixture, "loop.gba"), new Uint8Array([0]));
    const wrongRom = run([
      "scripts/replay-capture.mjs",
      resultPath,
      manifestPath,
    ]);
    assert.equal(wrongRom.status, 2);
    assert.match(wrongRom.stderr, /SHA-1 mismatch/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
