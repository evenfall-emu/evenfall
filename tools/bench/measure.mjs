// Pure measurement logic for tools/emu-bench/run.mjs: the harness output
// contract, the round schedule and the summary statistics.

export const CYCLES_PER_FRAME = 280896;
export const CYCLES_PER_LINE = 1232;
export const REALTIME_HZ = 59.7275;

// Parses and checks one harness's `benchmark: {JSON}` line against the plan.
// Throws on anything that would make the observation incomparable.
export function parseObservation(output, { frames, boot }) {
  const lines = output
    .split("\n")
    .filter((line) => line.startsWith("benchmark: "));
  if (lines.length !== 1)
    throw new Error(`expected one benchmark line, found ${lines.length}`);
  const line = JSON.parse(lines[0].slice("benchmark: ".length));
  if (line.schema !== 1)
    throw new Error(`unsupported harness schema ${line.schema}`);
  if (line.frames !== frames)
    throw new Error(`harness ran ${line.frames} frames, plan says ${frames}`);
  if (line.boot !== boot)
    throw new Error(`harness booted ${line.boot}, plan says ${boot}`);
  if (line.video !== "full" || line.audio !== "exact") {
    throw new Error(
      `harness policy video=${line.video} audio=${line.audio}, plan says full/exact`,
    );
  }
  if (!(typeof line.seconds === "number" && line.seconds > 0))
    throw new Error("harness reported no loop time");
  if (!/^[0-9a-f]{16}$/.test(line.framebufferHash ?? ""))
    throw new Error("harness reported no framebuffer hash");
  // Every harness stops at an instruction or event boundary after the target;
  // more than a scanline past it is a different amount of guest time.
  const target = BigInt(frames) * BigInt(CYCLES_PER_FRAME);
  const cycle = BigInt(line.cycle);
  if (cycle < target || cycle >= target + BigInt(CYCLES_PER_LINE)) {
    throw new Error(
      `harness stopped at cycle ${cycle}, outside [${target}, ${target + BigInt(CYCLES_PER_LINE)})`,
    );
  }
  return line;
}

// Rotates the emulator order per round and workload, so no emulator always
// runs first (or right after a given other) on the pinned CPU.
export function roundOrder(emulators, round, workloadIndex) {
  const shift = (round + workloadIndex) % emulators.length;
  return [...emulators.slice(shift), ...emulators.slice(0, shift)];
}

const median = (values) => {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = sorted.length >> 1;
  return sorted.length % 2
    ? sorted[middle]
    : (sorted[middle - 1] + sorted[middle]) / 2;
};

const geomean = (values) =>
  Math.exp(
    values.reduce((sum, value) => sum + Math.log(value), 0) / values.length,
  );

// Summarizes measured (non-warmup) observations. Each emulator's time ratio
// is the geometric mean over rounds of its loop time divided by the
// baseline's in the same round; the range is the smallest and largest round.
export function summarize({ observations, workloads, emulators, baseline }) {
  const rows = workloads.map((workload) => {
    const mine = observations.filter(
      (o) => o.workload === workload.name && !o.warmup,
    );
    const rounds = [...new Set(mine.map((o) => o.round))].sort((a, b) => a - b);
    const seconds = (emulator, round) =>
      mine.find((o) => o.emulator === emulator && o.round === round)?.seconds;
    const results = {};
    for (const emulator of emulators) {
      const times = rounds.map((round) => seconds(emulator, round));
      const ratios = rounds.map(
        (round) => seconds(emulator, round) / seconds(baseline, round),
      );
      const hashes = [
        ...new Set(
          observations
            .filter(
              (o) => o.workload === workload.name && o.emulator === emulator,
            )
            .map((o) => o.framebufferHash),
        ),
      ];
      const medianSeconds = median(times);
      results[emulator] = {
        medianSeconds,
        fps: workload.frames / medianSeconds,
        realtime: workload.frames / medianSeconds / REALTIME_HZ,
        timeRatio: geomean(ratios),
        timeRatioRange: [Math.min(...ratios), Math.max(...ratios)],
        framebufferHash: hashes.length === 1 ? hashes[0] : null,
        deterministic: hashes.length === 1,
      };
    }
    return {
      workload: workload.name,
      frames: workload.frames,
      rounds: rounds.length,
      results,
      image: imageAgreement(results),
    };
  });
  const overall = Object.fromEntries(
    emulators.map((emulator) => [
      emulator,
      geomean(rows.map((row) => row.results[emulator].timeRatio)),
    ]),
  );
  return { baseline, rows, geomeanTimeRatio: overall };
}

// Groups emulators by final image. `agree` means every emulator drew the same
// last frame; it is a check that each ran the workload, not a correctness
// verdict (see README for known rendering differences between emulators).
export function imageAgreement(results) {
  const groups = new Map();
  for (const [emulator, result] of Object.entries(results)) {
    const key = result.framebufferHash ?? "nondeterministic";
    groups.set(key, [...(groups.get(key) ?? []), emulator]);
  }
  if (groups.size === 1 && !groups.has("nondeterministic"))
    return { status: "agree", groups: [...groups.values()] };
  return { status: "differ", groups: [...groups.values()] };
}

export function formatTable(summary, emulators) {
  const cell = (result) =>
    `${Math.round(result.fps).toString().padStart(7)} fps ${result.timeRatio.toFixed(2).padStart(5)}`;
  const width = 18;
  const header = [
    "workload".padEnd(12),
    "frames".padStart(6),
    ...emulators.map((e) => e.padStart(width)),
    "  image",
  ].join(" ");
  const lines = [header];
  for (const row of summary.rows) {
    const image =
      row.image.status === "agree"
        ? "agree"
        : `differ: ${row.image.groups.map((g) => g.join("=")).join(" | ")}`;
    lines.push(
      [
        row.workload.padEnd(12),
        String(row.frames).padStart(6),
        ...emulators.map((e) => cell(row.results[e]).padStart(width)),
        `  ${image}`,
      ].join(" "),
    );
  }
  lines.push(
    [
      "".padEnd(12),
      "geomean".padStart(6),
      ...emulators.map((e) =>
        summary.geomeanTimeRatio[e].toFixed(2).padStart(width),
      ),
    ].join(" "),
  );
  return lines.join("\n");
}
