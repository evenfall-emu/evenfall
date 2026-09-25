// Sample the coast at half resolution; the traveler atlas already uses this
// native pixel grid. No smoothing, fractional sprite edges or React frame loop.
const WIDTH = 836;
const HEIGHT = 471;
const HORIZON = 351;
const TAU = Math.PI * 2;
const FRAME_MS = 1000 / 24;
const BOB_SECONDS = 3.8;
export const SUNSET_CLOUD_CYCLE_SECONDS = 120;
const SCARF = { x: 20, y: 11, width: 28, height: 12 };

export interface SunsetRenderer {
  setRunning(running: boolean): void;
  dispose(): void;
}

type Sprite = {
  x: number;
  y: number;
  width: number;
  height: number;
  source: readonly [number, number, number, number];
};

const RAFT: Sprite = {
  x: 450,
  y: 350,
  width: 112,
  height: 72,
  source: [0, 0, 112, 72],
};
const STAR: Sprite = {
  x: 476,
  y: 38,
  width: 18,
  height: 18,
  source: [120, 0, 18, 18],
};
const AIRSHIP: Sprite = {
  x: 608,
  y: 68,
  width: 52,
  height: 38,
  source: [120, 24, 52, 38],
};

// One shared schedule keeps the two cameos from competing for attention.
// Both remain visible in the initial reduced-motion illustration.
export function sunsetFlight(seconds: number, kind: "airship" | "star") {
  const cycle = (seconds + 10) % 64;
  if (kind === "airship") {
    return { visible: cycle < 14, x: -60 + (cycle / 14) * 952, y: 68 };
  }
  const progress = (cycle - 18) / 23;
  return {
    visible: progress >= 0 && progress < 1,
    x: -38 + progress * 912,
    y: 42 + Math.sin(progress * TAU) * 7,
  };
}

export function createSunsetRenderer(
  canvas: HTMLCanvasElement,
  background: HTMLImageElement,
  travelers: HTMLImageElement,
): SunsetRenderer {
  const context = canvas.getContext("2d", { alpha: false });
  if (!context) throw new Error("2D canvas is unavailable");
  const ctx = context;
  ctx.imageSmoothingEnabled = false;

  const plate = document.createElement("canvas");
  plate.width = WIDTH;
  plate.height = HEIGHT;
  const base = plate.getContext("2d")!;
  base.imageSmoothingEnabled = false;
  base.drawImage(background, 0, 0, WIDTH, HEIGHT);

  // Separate only the loose scarf. Its right edge stays attached to the neck;
  // the free end trails left, clear of the bear's head and backpack.
  const rider = document.createElement("canvas");
  rider.width = RAFT.width;
  rider.height = RAFT.height;
  const riderContext = rider.getContext("2d")!;
  riderContext.imageSmoothingEnabled = false;
  riderContext.drawImage(
    travelers,
    ...RAFT.source,
    0,
    0,
    rider.width,
    rider.height,
  );
  const scarf = document.createElement("canvas");
  scarf.width = SCARF.width;
  scarf.height = SCARF.height;
  const scarfContext = scarf.getContext("2d")!;
  // Step above the backpack's top edge at the bottom-right of the cutout.
  const scarfOutline = new Path2D("M20 11 H48 V22 H42 V23 H20 Z");
  scarfContext.translate(-SCARF.x, -SCARF.y);
  scarfContext.clip(scarfOutline);
  scarfContext.drawImage(rider, 0, 0);
  riderContext.save();
  riderContext.clip(scarfOutline);
  riderContext.clearRect(SCARF.x, SCARF.y, SCARF.width, SCARF.height);
  riderContext.restore();

  // Lift the painted foam from the ocean, retaining the original colors.
  // Each breaker grows, rolls forward, then dissolves into the next swell.
  const foam = document.createElement("canvas");
  foam.width = WIDTH;
  foam.height = 34;
  const foamContext = foam.getContext("2d")!;
  foamContext.drawImage(plate, 0, 387, WIDTH, 34, 0, 0, WIDTH, 34);
  const foamPixels = foamContext.getImageData(0, 0, WIDTH, 34);
  for (let i = 0; i < foamPixels.data.length; i += 4) {
    const r = foamPixels.data[i],
      g = foamPixels.data[i + 1],
      b = foamPixels.data[i + 2];
    const brightness = r * 0.21 + g * 0.72 + b * 0.07;
    foamPixels.data[i + 3] =
      g > 95 && brightness > 112
        ? Math.min(220, Math.round((brightness - 100) * 2.8))
        : 0;
  }
  foamContext.putImageData(foamPixels, 0, 0);

  let seconds = 0;
  let running = false;
  let disposed = false;
  let request = 0;
  let previous = 0;
  let lastPaint = 0;

  function sprite(s: Sprite, x = s.x, y = s.y) {
    ctx.drawImage(
      travelers,
      ...s.source,
      Math.round(x),
      Math.round(y),
      s.width,
      s.height,
    );
  }

  function clouds() {
    // A two-minute drift of the painted upper sky. Overscan covers the edges,
    // and a smooth taper stops before the cliffs, beacon and lower cloud bank.
    const drift = Math.sin((seconds * TAU) / SUNSET_CLOUD_CYCLE_SECONDS) * 18;
    for (let y = 108; y < 234; y += 2) {
      const fadeIn = Math.min(1, (y - 108) / 20);
      const fadeOut = Math.min(1, (234 - y) / 28);
      const edge = Math.min(fadeIn, fadeOut);
      const weight = edge * edge * (3 - 2 * edge);
      const margin = Math.round(19 * weight);
      const shift = Math.round(drift * weight);
      ctx.drawImage(
        plate,
        0,
        y,
        WIDTH,
        2,
        shift - margin,
        y,
        WIDTH + margin * 2,
        2,
      );
    }
  }

  function stardust() {
    // Reconstruct only the last 1.4 seconds of emissions. No growing particle
    // list, timers or random state: pausing freezes the complete trail too.
    const interval = 0.16;
    const lifetime = 1.4;
    const latest = Math.floor(seconds / interval);
    for (let i = 0; i < 9; i++) {
      const emission = latest - i;
      const born = emission * interval;
      const age = seconds - born;
      if (born < 0 || age > lifetime) continue;
      const flight = sunsetFlight(born, "star");
      if (!flight.visible) continue;
      const side = ((emission * 7) % 5) - 2;
      const x = Math.round(flight.x + 4 - age * 3);
      const y = Math.round(flight.y + 9 + side + age * (side + 2));
      ctx.globalAlpha = (1 - age / lifetime) * 0.7;
      ctx.fillStyle = emission % 3 ? "#ffcc81" : "#fff2bf";
      ctx.fillRect(x, y, 1, 1);
      if (emission % 4 === 0 && age < 0.7) {
        ctx.globalAlpha *= 0.6;
        ctx.fillRect(x - 1, y, 3, 1);
        ctx.fillRect(x, y - 1, 1, 3);
      }
    }
    ctx.globalAlpha = 1;
  }

  function raft(still: boolean) {
    const phase = (seconds * TAU) / BOB_SECONDS;
    const y = Math.round(RAFT.y + (still ? 0 : Math.sin(phase) * 2));
    // Painted contact foam is part of the raft, so it never detaches as we bob.
    ctx.drawImage(rider, RAFT.x, y);
    // The scarf lifts against the raft's downward bob.
    const scarfPhase = phase + Math.PI;
    const flutter = still
      ? 0
      : Math.sin(scarfPhase - 0.8) * 1.8 +
        Math.sin(scarfPhase * 2 - 0.4) * 0.35;
    for (let x = 0; x < SCARF.width; x++) {
      // Keep the last three columns anchored, then ease into a broad sway
      // across the red tail. The loose half moves together with the wind.
      const freedom = Math.min(1, Math.max(0, (SCARF.width - 3 - x) / 18));
      const bend = freedom * freedom * (3 - 2 * freedom);
      const lift = Math.round(flutter * bend);
      ctx.drawImage(
        scarf,
        x,
        0,
        1,
        SCARF.height,
        RAFT.x + SCARF.x + x,
        y + SCARF.y + lift,
        1,
        SCARF.height,
      );
    }
    if (!still) {
      // Flicker only the lantern's painted light, without a blurred halo.
      ctx.globalAlpha = 0.16 + Math.sin((seconds * TAU) / 1.7) * 0.1;
      ctx.globalCompositeOperation = "screen";
      ctx.drawImage(rider, 95, 32, 6, 8, RAFT.x + 95, y + 32, 6, 8);
      ctx.globalCompositeOperation = "source-over";
      ctx.globalAlpha = 1;
    }
  }

  function draw(still = false) {
    ctx.globalAlpha = 1;
    ctx.drawImage(plate, 0, 0);
    clouds();

    if (!still) {
      // Thin strips bend the painted waves and golden reflection.
      // The coast is excluded; displacement starts at the horizon.
      for (let y = HORIZON; y < HEIGHT; y += 2) {
        const depth = (y - HORIZON) / (HEIGHT - HORIZON);
        const shift = Math.round(
          Math.sin(y * 0.29 + seconds * 1.35) * (0.5 + depth * 3.5),
        );
        const rise = Math.round(
          Math.sin(depth * 9 - seconds * 1.2) * depth * 2.5,
        );
        const left = y < 378 ? Math.ceil((140 * (378 - y)) / 27) : 0;
        const sampleY = Math.min(HEIGHT - 2, Math.max(HORIZON, y + rise));
        const sourceX = Math.max(left, left - shift);
        const destX = Math.max(left, left + shift);
        const width = WIDTH - Math.max(sourceX, destX);
        ctx.drawImage(plate, sourceX, sampleY, width, 2, destX, y, width, 2);
      }

      for (const offset of [0, 0.5]) {
        const progress = (seconds / 6.5 + offset) % 1;
        const swell = Math.sin(progress * Math.PI);
        ctx.globalAlpha = swell * swell * 0.55;
        const width = Math.round(WIDTH * (0.94 + progress * 0.12));
        ctx.drawImage(
          foam,
          Math.round((WIDTH - width) / 2),
          Math.round(386 + progress * 35),
          width,
          Math.round(22 + progress * 16),
        );
      }
      ctx.globalAlpha = 1;

      // Pulse the existing painted beacon instead of drawing a new light.
      ctx.globalAlpha = (1 + Math.sin((seconds * TAU) / 5)) * 0.2;
      ctx.globalCompositeOperation = "screen";
      ctx.drawImage(plate, 209, 286, 12, 13, 209, 286, 12, 13);
      ctx.globalCompositeOperation = "source-over";
      ctx.globalAlpha = 1;
    }

    raft(still);
    if (!still) stardust();
    for (const [kind, art] of [
      ["airship", AIRSHIP],
      ["star", STAR],
    ] as const) {
      const flight = sunsetFlight(seconds, kind);
      if (still) sprite(art);
      else if (flight.visible) sprite(art, flight.x, flight.y);
    }
    canvas.dataset.sceneTime = seconds.toFixed(2);
  }

  function tick(now: number) {
    if (!running || disposed) return;
    if (previous) seconds += Math.min((now - previous) / 1000, 0.1);
    previous = now;
    if (now - lastPaint >= FRAME_MS) {
      draw();
      lastPaint = now;
    }
    request = requestAnimationFrame(tick);
  }

  draw(true);
  canvas.dataset.motion = "paused";
  return {
    setRunning(next) {
      if (disposed || running === next) return;
      running = next;
      canvas.dataset.motion = running ? "playing" : "paused";
      if (running) {
        previous = 0;
        lastPaint = 0;
        request = requestAnimationFrame(tick);
      } else cancelAnimationFrame(request);
    },
    dispose() {
      disposed = true;
      running = false;
      cancelAnimationFrame(request);
    },
  };
}
