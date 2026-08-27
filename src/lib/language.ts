export function isChineseLanguage(language?: string | null) {
  const value = language?.trim().toLowerCase().replaceAll("_", "-") || "";
  return value.startsWith("zh") || value === "chinese" || value === "中文" || value === "cmn" || value === "zho";
}

/**
 * Mirrors backend transcriber::is_chinese_text – returns true when the text is
 * predominantly Chinese (Han characters), so the segment does not need
 * Chinese→Chinese translation.
 */
export function isChineseText(text: string): boolean {
  let han = 0;
  let kana = 0;
  let hangul = 0;
  let latinWords = 0;
  let inLatin = false;
  for (const ch of text) {
    const cp = ch.codePointAt(0)!;
    // CJK Unified Ideographs and common extension ranges
    if ((cp >= 0x4e00 && cp <= 0x9fff) || (cp >= 0x3400 && cp <= 0x4dbf) || (cp >= 0x20000 && cp <= 0x2a6df) || (cp >= 0xf900 && cp <= 0xfaff)) {
      han++;
      inLatin = false;
    } else if ((cp >= 0x3040 && cp <= 0x309f) || (cp >= 0x30a0 && cp <= 0x30ff)) {
      kana++;
      inLatin = false;
    } else if (cp >= 0xac00 && cp <= 0xd7af) {
      hangul++;
      inLatin = false;
    } else if ((cp >= 0x41 && cp <= 0x5a) || (cp >= 0x61 && cp <= 0x7a) || (cp >= 0xc0 && cp <= 0x24f)) {
      if (!inLatin) { latinWords++; inLatin = true; }
    } else {
      inLatin = false;
    }
  }
  if (kana > 0 || hangul > 0) return false;
  if (han > 0 && latinWords === 0) return true;
  if (han >= 15 || (han >= 4 && han >= latinWords)) return true;
  return false;
}
