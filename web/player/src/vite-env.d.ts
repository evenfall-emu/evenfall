/// <reference types="vite/client" />

// Optional build-time instance settings; see frontend/README.md.
interface ImportMetaEnv {
  readonly VITE_POSTHOG_KEY?: string;
  readonly VITE_POSTHOG_HOST?: string;
  readonly VITE_POSTHOG_HOSTNAME?: string;
  readonly VITE_PRIVACY_URL?: string;
}
