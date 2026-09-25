/** Decode one bounded Gen III fixed-width text field. */
export function decodePokemonText(
  bytes: Uint8Array,
  language = 2,
): { text: string; warnings: readonly string[] } {
  const warnings: string[] = [];
  let text = "";

  if (language === 1) {
    warnings.push("Japanese Gen III text is unsupported");
    return { text: bytes.some((byte) => byte !== 0xff) ? "�" : "", warnings };
  }
  if (language < 2 || language > 7)
    warnings.push(`Unrecognized Pokémon language ${language}`);

  for (const byte of bytes) {
    if (byte === 0xff) break;
    const glyph = internationalGlyph(byte);
    if (glyph === null) {
      const kind = byte === 0xfc || byte === 0xfd ? "control code" : "glyph";
      warnings.push(`Unsupported Gen III ${kind} 0x${hex(byte)}`);
      text += "�";
    } else {
      text += glyph;
    }
  }
  return { text, warnings };
}

function internationalGlyph(byte: number): string | null {
  if (byte === 0) return " ";
  if (byte >= 0xa1 && byte <= 0xaa) return String(byte - 0xa1);
  if (byte >= 0xbb && byte <= 0xd4)
    return String.fromCharCode("A".charCodeAt(0) + byte - 0xbb);
  if (byte >= 0xd5 && byte <= 0xee)
    return String.fromCharCode("a".charCodeAt(0) + byte - 0xd5);
  return glyphs.get(byte) ?? null;
}

function hex(value: number): string {
  return value.toString(16).toUpperCase().padStart(2, "0");
}

const glyphs = new Map<number, string>([
  ...Object.entries({
    1: "À",
    2: "Á",
    3: "Â",
    4: "Ç",
    5: "È",
    6: "É",
    7: "Ê",
    8: "Ë",
    9: "Ì",
    11: "Î",
    12: "Ï",
    13: "Ò",
    14: "Ó",
    15: "Ô",
    16: "Œ",
    17: "Ù",
    18: "Ú",
    19: "Û",
    20: "Ñ",
    21: "ß",
    22: "à",
    23: "á",
    25: "ç",
    26: "è",
    27: "é",
    28: "ê",
    29: "ë",
    30: "ì",
    32: "î",
    33: "ï",
    34: "ò",
    35: "ó",
    36: "ô",
    37: "œ",
    38: "ù",
    39: "ú",
    40: "û",
    41: "ñ",
    42: "º",
    43: "ª",
  }).map(([byte, glyph]) => [Number(byte), glyph] as const),
  [0x2d, "&"],
  [0x2e, "+"],
  [0x35, "="],
  [0x36, ";"],
  [0x51, "¿"],
  [0x52, "¡"],
  [0x53, "PK"],
  [0x54, "MN"],
  [0x5a, "Í"],
  [0x5b, "%"],
  [0x5c, "("],
  [0x5d, ")"],
  [0x68, "â"],
  [0x6f, "í"],
  ...Array.from("!?.-·…“”‘’♂♀¥,×/").map(
    (glyph, index) => [0xab + index, glyph] as const,
  ),
  [0xef, "▶"],
  [0xf0, ":"],
  [0xf1, "Ä"],
  [0xf2, "Ö"],
  [0xf3, "Ü"],
  [0xf4, "ä"],
  [0xf5, "ö"],
  [0xf6, "ü"],
  [0xfe, "\n"],
]);
