import { z } from "zod";
import type { Base, Game, Version } from "./schema";

export const patchFormats = ["bps", "ups", "ips"] as const;

/** The hosted patch that turns a version's required base into its output. */
export type CatalogPatch = {
  url: string;
  format: (typeof patchFormats)[number];
  /** Download size in bytes. */
  size: number;
};

export type CatalogVersion = Omit<
  Version,
  "patchUrl" | "patchFormat" | "patchSize"
> & {
  requiredBase: Base | null;
  patch: CatalogPatch | null;
  /** Earlier version IDs whose in-game saves are verified to load in this one. */
  saveCompatibleFrom: string[];
};

type GameFacts = {
  /** In display order. */
  tags: string[];
  /** Cloud playthroughs of any of the game's versions, across accounts. */
  cloudPlaythroughs: number;
};
export type CatalogGame = Game & GameFacts & { versions: CatalogVersion[] };
export type CatalogGameSummary = Game & GameFacts & { versionCount: number };

const id = z
  .string()
  .min(1)
  .max(80)
  .regex(/^[a-z0-9]+(?:[.-][a-z0-9]+)*$/);
const name = z.string().trim().min(1).max(160);
const nullableText = z.string().trim().min(1).max(2_000).nullable();
const sha1 = z.string().regex(/^[0-9a-f]{40}$/);
const crc32 = z.string().regex(/^[0-9a-f]{8}$/);
const httpUrl = z
  .url()
  .refine(
    (value) => value.startsWith("https://") || value.startsWith("http://"),
  );
const nullableHttpUrl = httpUrl.nullable();
const unique = (values: string[]) =>
  new Set(values.map((value) => value.toLocaleLowerCase())).size ===
  values.length;

export const gameInput = z.strictObject({
  internalName: name,
  displayName: name,
  summary: nullableText,
  description: nullableText.optional(),
  imageUrl: nullableHttpUrl,
  franchise: z.string().trim().min(1).max(80).nullable(),
  tags: z
    .array(z.string().trim().min(1).max(40))
    .max(16)
    .refine(unique, "Tags must be unique"),
});

export const baseInput = z.strictObject({
  internalName: name,
  displayName: name,
  sha1,
  crc32,
});

export const versionInput = z
  .strictObject({
    gameId: id,
    internalName: name,
    displayName: name,
    outputSha1: sha1,
    requiredBaseId: id.nullable(),
    sourceRepositoryUrl: nullableHttpUrl,
    sourceRevision: z.string().trim().min(1).max(160).nullable(),
    releasedAt: z.iso.date().nullable(),
    releaseNotes: z.string().trim().min(1).max(20_000).nullable(),
    patch: z
      .strictObject({
        url: httpUrl,
        format: z.enum(patchFormats),
        size: z
          .number()
          .int()
          .positive()
          .max(2 ** 31 - 1),
      })
      .nullable(),
    saveCompatibleFrom: z
      .array(id)
      .max(64)
      .refine(unique, "Versions must be unique"),
  })
  .refine((version) => !version.patch || version.requiredBaseId, {
    message: "A patch needs the base it applies to",
    path: ["patch"],
  });

export const resourceId = id;

export type GameInput = z.infer<typeof gameInput>;
export type BaseInput = z.infer<typeof baseInput>;
export type VersionInput = z.infer<typeof versionInput>;
