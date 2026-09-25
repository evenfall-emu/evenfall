import { useMemo } from "react";
import type { PokemonResources } from "../src/bridges";
import type {
  PokemonArtworkRef,
  PokemonItemArtworkRef,
  PokemonTrainerArtworkRef,
} from "../src/model";

/** Bounded, resource-scoped PNG cache. No ROM bytes or pixels enter snapshots. */
export function usePokemonArtwork(resources: PokemonResources | undefined) {
  return useMemo(() => {
    const cache = new Map<string, { src?: string; reason?: string }>();
    return (
      ref:
        | PokemonArtworkRef
        | PokemonItemArtworkRef
        | PokemonTrainerArtworkRef
        | undefined,
    ): { src?: string; reason?: string } => {
      if (!ref || !resources) return {};
      const key = JSON.stringify(ref);
      const cached = cache.get(key);
      if (cached) return cached;
      let result: { src?: string; reason?: string };
      try {
        const image = resources.artwork(ref);
        const canvas = document.createElement("canvas");
        canvas.width = image.width;
        canvas.height = image.height;
        const context = canvas.getContext("2d");
        if (!context) throw new Error("Canvas is unavailable");
        const data = context.createImageData(image.width, image.height);
        data.data.set(image.pixels);
        context.putImageData(data, 0, 0);
        result = { src: canvas.toDataURL() };
      } catch (error) {
        result = {
          reason: error instanceof Error ? error.message : String(error),
        };
      }
      if (cache.size >= 64) cache.delete(cache.keys().next().value!);
      cache.set(key, result);
      return result;
    };
  }, [resources]);
}
