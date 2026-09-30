import { betterAuth } from "better-auth/minimal";
import { magicLink } from "better-auth/plugins/magic-link";

// Schema generation only: core auth, database rate limiting, and magic links.
// No real bindings or secrets are loaded by the generator.
export const auth = betterAuth({
  secret: "schema-generation-only-not-a-runtime-secret",
  rateLimit: { enabled: true, storage: "database" },
  plugins: [
    magicLink({
      sendMagicLink: async () => {
        throw new Error("Schema generation only");
      },
    }),
  ],
});
