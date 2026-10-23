// i18n infrastructure (WP-18, DESIGN §11.5; convLink pattern).
//
// - Catalogs are flat `key → text` maps built with `defineCatalog`: English is the
//   source, Spanish must implement exactly the same keys (checked at compile time and by
//   tests/i18n-parity.test.ts).
// - One catalog file per namespace in `src/i18n/catalogs/<namespace>.ts`; the app
//   bootstrap (`src/app/i18n.ts`) registers every file it finds there, so a feature adds
//   a namespace by adding a file. Owners: common/errors/notices/prompts (WP-18), lab
//   (WP-21), onboarding/group/ride/settings (WP-22).
// - Keys may contain dots (`demo.line1`): key and namespace separators are disabled.
// - Interpolation uses i18next syntax (`{{name}}`); the engine sends params, never text.
//
// This module never imports catalogs, so catalog files can import `defineCatalog` from
// here without an import cycle.
import i18next from "i18next";
import type { TOptions } from "i18next";
import { initReactI18next, useTranslation } from "react-i18next";

/** UI languages (DESIGN L9): neutral Spanish and English. */
export const APP_LANGUAGES = ["en", "es"] as const;
export type AppLanguage = (typeof APP_LANGUAGES)[number];

/** A flat message catalog: key → text (i18next interpolation syntax). */
export type MessageCatalog = Readonly<Record<string, string>>;

/** English source catalog plus its Spanish translation, key-for-key. */
export interface Catalog<E extends MessageCatalog = MessageCatalog> {
  readonly en: E;
  readonly es: { readonly [K in keyof E]: string };
}

/** Runtime brand so the bootstrap can find the catalog among a file's exports. */
const CATALOG_BRAND = "meshapp.i18n.catalog";

/**
 * Declares a namespace catalog. Spanish must provide every English key and no other
 * (excess keys are rejected for inline object literals), so a missing translation is a
 * type error. Both call forms are accepted: `defineCatalog(en, es)` (convLink) and
 * `defineCatalog({ en, es })`.
 */
export function defineCatalog<const E extends MessageCatalog>(catalog: {
  en: E;
  es: { [K in keyof E]: string };
}): Catalog<E>;
export function defineCatalog<const E extends MessageCatalog>(en: E, es: { [K in keyof E]: string }): Catalog<E>;
export function defineCatalog<const E extends MessageCatalog>(
  first: E | { en: E; es: { [K in keyof E]: string } },
  second?: { [K in keyof E]: string },
): Catalog<E> {
  const pair =
    second === undefined ? (first as { en: E; es: { [K in keyof E]: string } }) : { en: first as E, es: second };
  const catalog = { en: Object.freeze({ ...pair.en }), es: Object.freeze({ ...pair.es }) };
  // A string-keyed property (not a module-level Symbol) so calling this during an
  // import cycle never touches an uninitialised binding.
  Object.defineProperty(catalog, CATALOG_BRAND, { value: true, enumerable: false });
  return Object.freeze(catalog) as Catalog<E>;
}

/** `true` when `value` was built by `defineCatalog`. */
export function isCatalog(value: unknown): value is Catalog {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as Record<string, unknown>)[CATALOG_BRAND] === true
  );
}

/** Keys present in one language and missing in the other (empty when in parity). */
export function catalogParityErrors(catalog: Catalog): string[] {
  const en = Object.keys(catalog.en);
  const es = Object.keys(catalog.es);
  const errors: string[] = [];
  for (const k of en) {
    if (!Object.hasOwn(catalog.es, k)) errors.push(`es is missing "${k}"`);
    else if (typeof catalog.es[k] !== "string" || catalog.es[k] === "") errors.push(`es "${k}" is empty`);
  }
  for (const k of es) if (!Object.hasOwn(catalog.en, k)) errors.push(`es has extra key "${k}"`);
  for (const k of en) if (catalog.en[k] === "") errors.push(`en "${k}" is empty`);
  return errors;
}

/** Normalises any value (persisted setting, navigator language) to an app language. */
export function normalizeLanguage(value: unknown): AppLanguage {
  if (typeof value !== "string") return "en";
  return value.toLowerCase().startsWith("es") ? "es" : "en";
}

/** The device language as an app language (first run default). */
export function deviceLanguage(): AppLanguage {
  if (typeof navigator === "undefined") return "en";
  const preferred = navigator.languages?.[0] ?? navigator.language;
  return normalizeLanguage(preferred);
}

/** Intl locale per language (Appendix E: neutral `es`). */
export const INTL_LOCALE: Readonly<Record<AppLanguage, string>> = { en: "en-US", es: "es" };

/** The single i18next instance (bundled resources, synchronous init). */
export const i18n = i18next.createInstance();

void i18n.use(initReactI18next).init({
  lng: "en",
  fallbackLng: "en",
  supportedLngs: [...APP_LANGUAGES],
  defaultNS: "common",
  ns: [],
  resources: {},
  keySeparator: false,
  nsSeparator: false,
  interpolation: { escapeValue: false },
  returnNull: false,
  returnEmptyString: false,
  initAsync: false,
  react: { useSuspense: false },
});

const registered = new Set<string>();

/** Registers (or replaces) one namespace in both languages. */
export function registerCatalog(namespace: string, catalog: Catalog): void {
  if (!/^[a-z][A-Za-z0-9]*$/.test(namespace)) throw new Error(`invalid i18n namespace "${namespace}"`);
  i18n.addResourceBundle("en", namespace, { ...catalog.en }, false, true);
  i18n.addResourceBundle("es", namespace, { ...catalog.es }, false, true);
  registered.add(namespace);
}

/** Namespaces registered so far, sorted. */
export function registeredNamespaces(): string[] {
  return [...registered].sort();
}

/** The registered text table of one namespace and language (empty when unknown). */
export function catalogTexts(namespace: string, language: AppLanguage): Readonly<Record<string, string>> {
  const bundle = i18n.getResourceBundle(language, namespace) as Record<string, string> | undefined;
  return bundle ?? {};
}

/** The language currently applied to the UI. */
export function currentLanguage(): AppLanguage {
  return normalizeLanguage(i18n.language);
}

/** Switches the UI language and the document `lang` attribute. */
export function applyLanguage(language: AppLanguage): void {
  if (i18n.language !== language) void i18n.changeLanguage(language);
  if (typeof document !== "undefined") document.documentElement.lang = language;
}

export type TranslateParams = TOptions & Record<string, unknown>;

/** Translates outside React (stores, the event fan-out, engine prompts). */
export function translate(namespace: string, key: string, params?: TranslateParams, language?: AppLanguage): string {
  const t = i18n.getFixedT(language ?? currentLanguage(), namespace);
  return t(key, params ?? {});
}

/** `true` when `key` exists in `namespace` (current language, with English fallback). */
export function hasTranslation(namespace: string, key: string): boolean {
  return i18n.exists(key, { ns: namespace });
}

/** React hook: `t` bound to one namespace; re-renders on language change. */
export function useT(namespace = "common") {
  const { t, i18n: instance } = useTranslation(namespace, { i18n });
  return { t, language: normalizeLanguage(instance.language) };
}
