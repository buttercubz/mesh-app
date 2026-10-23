/** Joins class names, skipping falsy entries (tiny clsx replacement, no dependency). */
export function cx(...parts: ReadonlyArray<string | false | null | undefined>): string {
  let out = "";
  for (const part of parts) {
    if (!part) continue;
    out = out ? `${out} ${part}` : part;
  }
  return out;
}
