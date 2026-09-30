/**
 * A search key: case, accents and punctuation folded away, so "pokefan" finds
 * "Pokéfan" and "route 103 partner" finds "Route 103 · Partner".
 */
export function searchKey(text: string): string {
  return text
    .normalize("NFKD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();
}
