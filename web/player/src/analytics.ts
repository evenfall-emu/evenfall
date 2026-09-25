import posthog, { type PostHogConfig } from "posthog-js";

// Analytics are opt-in at build time. Without a PostHog project key and host
// (public ingestion values, not secrets) nothing is initialized or sent. When
// VITE_POSTHOG_HOSTNAME is set, only pages served from exactly that hostname
// send events, so local previews and copies of a deployment stay silent.
const token = import.meta.env.VITE_POSTHOG_KEY ?? "";
const host = import.meta.env.VITE_POSTHOG_HOST ?? "";
const hostname = import.meta.env.VITE_POSTHOG_HOSTNAME ?? "";
export const analyticsEnabled =
  import.meta.env.PROD &&
  token !== "" &&
  host !== "" &&
  (hostname === "" || location.hostname === hostname);

export const analyticsConfig = {
  api_host: host,
  cookieless_mode: "always",
  person_profiles: "never",
  autocapture: false,
  capture_pageview: false,
  capture_pageleave: false,
  disable_session_recording: true,
  advanced_disable_flags: true,
  before_send: (event) => {
    if (event?.event !== "$pageview") return null;
    // Neither auth callback queries nor debug hash routes/referrers belong in analytics.
    event.properties = {
      token: event.properties.token,
      distinct_id: event.properties.distinct_id,
      $cookieless_mode: event.properties.$cookieless_mode,
      $process_person_profile: event.properties.$process_person_profile,
      $current_url: `${location.origin}${location.pathname}`,
      $pathname: location.pathname,
      // PostHog needs these with the request's IP to hash cookieless events.
      $host: event.properties.$host,
      $raw_user_agent: event.properties.$raw_user_agent,
      view: event.properties.view,
      $session_id: event.properties.$session_id,
      $window_id: event.properties.$window_id,
    };
    return event;
  },
} satisfies Partial<PostHogConfig>;

if (analyticsEnabled) posthog.init(token, analyticsConfig);

export function trackView(view: "home" | "player" | "library") {
  if (analyticsEnabled) posthog.capture("$pageview", { view });
}
