import { afterEach, expect, it, vi } from "vitest";

afterEach(() => {
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  vi.doUnmock("posthog-js");
  vi.resetModules();
});

function stubLocation(hostname = "zigba.ursid.ai") {
  vi.stubGlobal("location", {
    hostname,
    host: hostname,
    origin: `https://${hostname}`,
    pathname: "/",
    search: "?account=1&error_description=private",
    hash: "#private",
  });
}

async function loadAnalytics(settings: Record<string, string>) {
  for (const name of [
    "VITE_POSTHOG_KEY",
    "VITE_POSTHOG_HOST",
    "VITE_POSTHOG_HOSTNAME",
  ])
    vi.stubEnv(name, settings[name] ?? "");
  vi.stubEnv("PROD", true);
  const posthog = { init: vi.fn(), capture: vi.fn() };
  vi.doMock("posthog-js", () => ({ default: posthog }));
  const analytics = await import("../src/analytics");
  analytics.trackView("home");
  return { ...analytics, posthog };
}

it("stays off without a build-time key and host, and honors the hostname guard", async () => {
  const official = {
    VITE_POSTHOG_KEY: "public-token",
    VITE_POSTHOG_HOST: "https://eu.i.posthog.com",
    VITE_POSTHOG_HOSTNAME: "zigba.ursid.ai",
  };
  for (const [settings, hostname] of [
    [{}, "zigba.ursid.ai"],
    [{ ...official, VITE_POSTHOG_KEY: "" }, "zigba.ursid.ai"],
    [{ ...official, VITE_POSTHOG_HOST: "" }, "zigba.ursid.ai"],
    [official, "127.0.0.1"],
    [official, "zigba.example"],
  ] as const) {
    stubLocation(hostname);
    const { analyticsEnabled, posthog } = await loadAnalytics(settings);
    expect(analyticsEnabled, `${hostname} ${JSON.stringify(settings)}`).toBe(
      false,
    );
    expect(posthog.init).not.toHaveBeenCalled();
    expect(posthog.capture).not.toHaveBeenCalled();
    vi.resetModules();
  }

  stubLocation("zigba.ursid.ai");
  const enabled = await loadAnalytics(official);
  expect(enabled.analyticsEnabled).toBe(true);
  expect(enabled.posthog.init).toHaveBeenCalledWith(
    "public-token",
    enabled.analyticsConfig,
  );
  expect(enabled.posthog.capture).toHaveBeenCalledWith("$pageview", {
    view: "home",
  });
  vi.resetModules();

  // Without a hostname restriction, a self-hosted build sends from anywhere.
  stubLocation("zigba.example");
  const selfHosted = await loadAnalytics({
    ...official,
    VITE_POSTHOG_HOSTNAME: "",
  });
  expect(selfHosted.analyticsEnabled).toBe(true);
});

it("allows only cookieless pageviews without session replay or autocapture", async () => {
  stubLocation();
  vi.stubEnv("VITE_POSTHOG_HOST", "https://eu.i.posthog.com");
  const { analyticsConfig } = await import("../src/analytics");
  expect(analyticsConfig).toMatchObject({
    api_host: "https://eu.i.posthog.com",
    cookieless_mode: "always",
    person_profiles: "never",
    autocapture: false,
    capture_pageview: false,
    capture_pageleave: false,
    disable_session_recording: true,
    advanced_disable_flags: true,
  });

  const event = {
    uuid: "test-event",
    event: "$pageview",
    properties: {
      token: "public-token",
      distinct_id: "$posthog_cookieless",
      $cookieless_mode: true,
      $process_person_profile: false,
      view: "library",
      $session_id: "session",
      $window_id: "window",
      $current_url:
        "https://zigba.ursid.ai/?account=1&error_description=private#private",
      $host: "zigba.ursid.ai",
      $raw_user_agent: "Test browser UA",
      $referrer: "https://example.com/private",
      game: "private ROM title",
    },
  };
  expect(analyticsConfig.before_send(event)).toEqual({
    uuid: "test-event",
    event: "$pageview",
    properties: {
      token: "public-token",
      distinct_id: "$posthog_cookieless",
      $cookieless_mode: true,
      $process_person_profile: false,
      view: "library",
      $session_id: "session",
      $window_id: "window",
      $current_url: "https://zigba.ursid.ai/",
      $host: "zigba.ursid.ai",
      $raw_user_agent: "Test browser UA",
      $pathname: "/",
    },
  });
  expect(
    analyticsConfig.before_send({ ...event, event: "$exception" }),
  ).toBeNull();
});
