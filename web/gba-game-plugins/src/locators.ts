import { content, type Locator } from "@evenfall/gba/bridge";
import { encodePokemonText } from "./text";

/**
 * Where a Gen III string is stored in the ROM, with its 0xFF terminator:
 * `offset(references(pokemonText("BULBASAUR")), -4)` is the start of a table
 * of name pointers whose entry 1 is Bulbasaur's.
 */
export function pokemonText(text: string): Locator {
  const pattern = Array.from(encodePokemonText(text), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join(" ");
  return { ...content(pattern), id: `pokemonText(${JSON.stringify(text)})` };
}
