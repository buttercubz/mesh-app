// Loads every feature module (`src/features/<name>/index.ts[x]`) so each can register its
// screens with `registerScreen` (app/router.ts). Adding a feature never edits the shell.
const features = import.meta.glob("../features/*/index.{ts,tsx}", { eager: true });

/** Feature folders found at build time (diagnostics and tests). */
export const LOADED_FEATURES: readonly string[] = Object.keys(features)
  .map((file) => file.split("/")[2] ?? file)
  .sort();
