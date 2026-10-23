// Registers every catalog file under src/i18n/catalogs/ as the namespace named after the
// file (`lab.ts` → `lab`). Features add a namespace by adding a file that exports one
// `defineCatalog(...)` value; nothing else needs editing. Imported first by main.tsx and
// by the Vitest setup, so every module sees a fully loaded i18n instance.
import type { Catalog } from "../i18n/index";
import { isCatalog, registerCatalog } from "../i18n/index";

const modules = import.meta.glob<Record<string, unknown>>("../i18n/catalogs/*.ts", { eager: true });

/** Finds the single catalog a catalog file exports; throws on zero or several. */
export function catalogOf(file: string, exports: Record<string, unknown>): Catalog {
  const found = Object.values(exports).filter(isCatalog);
  const unique = [...new Set(found)];
  if (unique.length !== 1) {
    throw new Error(`${file} must export exactly one defineCatalog(...) value (found ${unique.length})`);
  }
  return unique[0] as Catalog;
}

/** `../i18n/catalogs/ride.ts` → `ride`. */
export function namespaceOf(file: string): string {
  const base = file.slice(file.lastIndexOf("/") + 1);
  return base.replace(/\.ts$/, "");
}

/** The catalogs found at build time, by namespace. */
export const APP_CATALOGS: Readonly<Record<string, Catalog>> = Object.freeze(
  Object.fromEntries(
    Object.entries(modules)
      .filter(([file]) => !file.endsWith(".test.ts"))
      .map(([file, exports]) => [namespaceOf(file), catalogOf(file, exports)]),
  ),
);

for (const [namespace, catalog] of Object.entries(APP_CATALOGS)) registerCatalog(namespace, catalog);

/** Namespaces owned by the shell (WP-18); feature namespaces come on top. */
export const SHELL_NAMESPACES = ["common", "errors", "notices", "prompts"] as const;
