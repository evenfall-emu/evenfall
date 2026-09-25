import { betterAuth } from "better-auth/minimal";
import { drizzleAdapter } from "better-auth/adapters/drizzle";
import { APIError, createAuthMiddleware } from "better-auth/api";
import { magicLink } from "better-auth/plugins/magic-link";
import { drizzle } from "drizzle-orm/d1";
import * as schema from "./auth-schema";

export interface AuthBindings {
  DB?: D1Database;
  BETTER_AUTH_URL?: string;
  BETTER_AUTH_SECRET?: string;
  DISCORD_CLIENT_ID?: string;
  DISCORD_CLIENT_SECRET?: string;
  EMAIL?: SendEmail;
  EMAIL_FROM?: string;
}

/** The fixed local Vite origin; every other instance must be an HTTPS origin. */
const LOCAL_ORIGIN = "http://127.0.0.1:5173";

function acceptedOrigin(value: string | undefined): value is string {
  if (value === LOCAL_ORIGIN) return true;
  try {
    const url = new URL(value ?? "");
    return url.protocol === "https:" && url.origin === value;
  } catch {
    return false;
  }
}

export function createAuth(env: AuthBindings) {
  const baseURL = env.BETTER_AUTH_URL;
  if (
    !env.DB ||
    !env.BETTER_AUTH_SECRET ||
    env.BETTER_AUTH_SECRET.length < 32 ||
    !acceptedOrigin(baseURL)
  )
    return null;

  return betterAuth({
    appName: "ZiGBA",
    baseURL,
    basePath: "/api/auth",
    secret: env.BETTER_AUTH_SECRET,
    trustedOrigins: [baseURL],
    database: drizzleAdapter(drizzle(env.DB, { schema }), {
      provider: "sqlite",
      schema,
    }),
    session: {
      expiresIn: 60 * 60 * 24 * 7,
      updateAge: 60 * 60 * 24,
      cookieCache: { enabled: false },
    },
    account: {
      encryptOAuthTokens: true,
      accountLinking: { enabled: true, allowDifferentEmails: false },
    },
    user: {
      validateUserInfo({ source }) {
        if (source.oauth?.providerId !== "discord") return;
        const profile = source.oauth.profile;
        if (
          typeof profile?.email !== "string" ||
          !profile.email ||
          profile.verified !== true
        ) {
          return {
            error: "discord_email_not_verified",
            errorDescription:
              "Use a verified Discord email or sign in by email",
          };
        }
      },
    },
    socialProviders:
      env.DISCORD_CLIENT_ID && env.DISCORD_CLIENT_SECRET
        ? {
            discord: {
              clientId: env.DISCORD_CLIENT_ID,
              clientSecret: env.DISCORD_CLIENT_SECRET,
            },
          }
        : {},
    advanced: {
      useSecureCookies: baseURL.startsWith("https:"),
      ipAddress: { ipAddressHeaders: ["cf-connecting-ip"] },
    },
    rateLimit: {
      enabled: true,
      storage: "database",
      customRules: { "/sign-in/magic-link": { window: 60, max: 3 } },
    },
    hooks: {
      before: createAuthMiddleware(async (ctx) => {
        // Validate on initiation too, before emailing an attacker-chosen link.
        for (const field of [
          "callbackURL",
          "newUserCallbackURL",
          "errorCallbackURL",
          "redirectTo",
        ]) {
          const target = ctx.body?.[field];
          if (target === undefined) continue;
          let valid = false;
          try {
            valid =
              typeof target === "string" &&
              new URL(target, baseURL).origin === baseURL;
          } catch {
            /* malformed URLs are rejected below */
          }
          if (!valid)
            throw new APIError("FORBIDDEN", {
              message: "Invalid callback URL",
            });
        }
      }),
    },
    telemetry: { enabled: false },
    plugins: [
      magicLink({
        expiresIn: 300,
        storeToken: "hashed",
        async sendMagicLink({ email, url }) {
          const recipient = email.trim().toLowerCase();
          try {
            if (
              !env.EMAIL ||
              !env.EMAIL_FROM ||
              !(await allowMagicLink(env.DB!, recipient))
            )
              return;
            await env.EMAIL.send({
              from: { email: env.EMAIL_FROM, name: "ZiGBA" },
              to: recipient,
              subject: "Sign in to ZiGBA",
              text: `Sign in to ZiGBA:\n\n${url}\n\nThis link expires in 5 minutes and can be used once. If you did not request it, ignore this email.`,
            });
          } catch {
            // Do not expose suppression/delivery status, recipient addresses,
            // provider exceptions, or magic-link tokens in responses or logs.
            console.error("Magic-link delivery failed");
          }
        },
      }),
    ],
  });
}

async function allowMagicLink(db: D1Database, email: string): Promise<boolean> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(email),
  );
  const identity = Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
  const now = Date.now();
  // Reuse the auth rate-limit store with a separate key namespace. Each UPSERT
  // is atomic, so parallel isolates cannot all pass a read-then-write check.
  const results = await db.batch(
    [
      { window: 15 * 60_000, max: 3 },
      { window: 24 * 60 * 60_000, max: 10 },
    ].map(({ window, max }) => {
      const key = `recipient:${identity}:${window}`;
      const cutoff = now - window;
      return db
        .prepare(
          `
      INSERT INTO rate_limit (id, key, count, last_request) VALUES (?, ?, 1, ?)
      ON CONFLICT(key) DO UPDATE SET
        count = CASE WHEN last_request <= ? THEN 1 ELSE count + 1 END,
        last_request = CASE WHEN last_request <= ? THEN excluded.last_request ELSE last_request END
      WHERE last_request <= ? OR count < ?
      RETURNING count
    `,
        )
        .bind(key, key, now, cutoff, cutoff, cutoff, max);
    }),
  );
  return results.every((result) => result.results.length === 1);
}
