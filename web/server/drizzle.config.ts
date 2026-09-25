import { defineConfig } from "drizzle-kit";

export default defineConfig({
  schema: ["./src/schema.ts", "./src/auth-schema.ts"],
  out: "./migrations",
  dialect: "sqlite",
});
