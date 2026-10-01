import { existsSync } from "node:fs";
import type { LaunchOptions } from "@playwright/test";

/** Chromium for every browser test, with `extra` flags added. */
export function chromiumLaunch(extra: string[] = []): LaunchOptions {
  return {
    executablePath:
      process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ??
      (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined),
    args: ["--no-sandbox", "--disable-audio-output", ...extra],
  };
}

/**
 * Chromium's software WebGPU adapter, SwiftShader. Without the Vulkan
 * feature, SwiftShader loses the device right after creating it.
 */
export const SOFTWARE_WEBGPU = [
  "--enable-unsafe-webgpu",
  "--enable-unsafe-swiftshader",
  "--enable-features=Vulkan",
  "--use-angle=swiftshader",
];
