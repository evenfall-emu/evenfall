import { z } from "zod";
import type { Base, Game, Version } from "./schema";

export type CatalogBase = Base & { verified: boolean };
export type CatalogVersion = Version & { requiredBase: CatalogBase | null };
export type CatalogGame = Game & { versions: CatalogVersion[] };
export type CatalogGameSummary = Game & { versionCount: number };

const id = z
  .string()
  .min(1)
  .max(80)
  .regex(/^[a-z0-9]+(?:[.-][a-z0-9]+)*$/);
const name = z.string().trim().min(1).max(160);
const nullableText = z.string().trim().min(1).max(2_000).nullable();
const sha256 = z.string().regex(/^[0-9a-f]{64}$/);
const nullableHttpUrl = z
  .url()
  .refine(
    (value) => value.startsWith("https://") || value.startsWith("http://"),
  )
  .nullable();

export const gameInput = z.strictObject({
  internalName: name,
  displayName: name,
  summary: nullableText,
  imageUrl: nullableHttpUrl,
});

export const baseInput = z.strictObject({
  internalName: name,
  displayName: name,
  sha256: sha256.nullable(),
});

export const versionInput = z.strictObject({
  gameId: id,
  internalName: name,
  displayName: name,
  outputSha256: sha256,
  requiredBaseId: id.nullable(),
  sourceRepositoryUrl: nullableHttpUrl,
  sourceRevision: z.string().trim().min(1).max(160).nullable(),
});

export const resourceId = id;

export type GameInput = z.infer<typeof gameInput>;
export type BaseInput = z.infer<typeof baseInput>;
export type VersionInput = z.infer<typeof versionInput>;
