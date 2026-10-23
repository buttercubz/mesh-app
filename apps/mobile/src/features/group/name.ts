// Display-name limits (DESIGN §1.3 F0): 24 UTF-8 bytes, cut at a grapheme boundary.
import { DISPLAY_NAME_MAX_BYTES } from "@meshapp/engine-api";

const utf8 = new TextEncoder();

/** UTF-8 byte length of a string. */
export function byteLength(text: string): number {
  return utf8.encode(text).length;
}

/** Keeps typing within the byte limit (grapheme-safe), without trimming inner spaces. */
export function limitName(text: string): string {
  if (byteLength(text) <= DISPLAY_NAME_MAX_BYTES) return text;
  const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });
  let out = "";
  for (const { segment } of segmenter.segment(text)) {
    if (byteLength(out + segment) > DISPLAY_NAME_MAX_BYTES) break;
    out += segment;
  }
  return out;
}
