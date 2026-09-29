// CPU selection shared with ZiGBA's benchmark method: choose an allowed,
// idle, fast core before building, then pin every observation to it.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { cpus } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
export const sha256 = (bytes) =>
  createHash("sha256").update(bytes).digest("hex");
export const detectTaskset = () =>
  spawnSync("taskset", ["--version"], { encoding: "utf8" }).status === 0;

function allowedCpus() {
  try {
    const line = readFileSync("/proc/self/status", "utf8")
      .split("\n")
      .find((l) => l.startsWith("Cpus_allowed_list:"));
    if (!line) return null;
    const ids = new Set();
    for (const part of line.split(":")[1].trim().split(",")) {
      const [lo, hi] = part.split("-").map(Number);
      for (let id = lo; id <= (hi ?? lo); id++) ids.add(id);
    }
    return ids;
  } catch {
    return null;
  }
}

function sampleCpuIdle() {
  try {
    const map = new Map();
    for (const line of readFileSync("/proc/stat", "utf8").split("\n")) {
      const match = line.match(/^cpu(\d+)\s+(.*)$/);
      if (!match) continue;
      const fields = match[2].trim().split(/\s+/).map(Number);
      map.set(Number(match[1]), fields[3] + (fields[4] ?? 0));
    }
    return map.size ? map : null;
  } catch {
    return null;
  }
}

function cpuMaxKhz(id) {
  try {
    return (
      Number(
        readFileSync(
          `/sys/devices/system/cpu/cpu${id}/cpufreq/cpuinfo_max_freq`,
          "utf8",
        ).trim(),
      ) || 0
    );
  } catch {
    return 0;
  }
}

export async function pickCpu(requested) {
  const allowed = allowedCpus();
  const count = cpus().length;
  if (requested && requested !== "auto") {
    const cpuId = Number(requested);
    if (
      !Number.isInteger(cpuId) ||
      cpuId < 0 ||
      (allowed ? !allowed.has(cpuId) : cpuId >= count)
    )
      throw new Error(
        `--cpu must be one of ${allowed ? [...allowed].join(",") : `0..${count - 1}`} or auto`,
      );
    return { cpuId, note: "explicit" };
  }
  const first = sampleCpuIdle();
  await new Promise((settle) => setTimeout(settle, 250));
  const second = sampleCpuIdle();
  if (first && second) {
    const deltas = [...first.keys()]
      .filter((id) => !allowed || allowed.has(id))
      .map((id) => ({
        id,
        idle: (second.get(id) ?? 0) - (first.get(id) ?? 0),
      }));
    if (deltas.length) {
      const bestIdle = Math.max(...deltas.map((entry) => entry.idle));
      const threshold = Math.max(1, Math.floor(bestIdle / 2));
      let candidates = deltas.filter((entry) => entry.idle >= threshold);
      const nonZero = candidates.filter((entry) => entry.id !== 0);
      if (nonZero.length) candidates = nonZero;
      candidates.sort(
        (a, b) =>
          cpuMaxKhz(b.id) - cpuMaxKhz(a.id) || b.idle - a.idle || a.id - b.id,
      );
      const chosen = candidates[0];
      const khz = cpuMaxKhz(chosen.id);
      return {
        cpuId: chosen.id,
        note: `auto (idle+fastest core${khz ? `, ${(khz / 1e6).toFixed(1)} GHz cap` : ""}, ${first.get(chosen.id)} -> ${second.get(chosen.id)} idle ticks in 250 ms)`,
      };
    }
  }
  return {
    cpuId: allowed?.size ? Math.max(...allowed) : Math.max(0, count - 1),
    note: "auto (no usable /proc/stat sample; picked last allowed CPU)",
  };
}

export function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: "utf8", ...options });
  if (result.error || result.status !== 0)
    throw new Error(
      `${command} failed: ${result.error?.message ?? `${result.stdout ?? ""}${result.stderr ?? ""}`.trim()}`,
    );
  return result;
}
