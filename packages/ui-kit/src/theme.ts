/**
 * Runtime theme switches for the tokens in tokens.css. Dark is the app default
 * (DESIGN §11.4); `system` follows prefers-color-scheme and keeps following it.
 */

export type ThemePreference = "dark" | "light" | "system";
export type ResolvedTheme = "dark" | "light";
export type UiPlatform = "ios" | "android" | "web";

const DARK_QUERY = "(prefers-color-scheme: dark)";

/** Light/dark for a preference; `system` reads the media query (dark when unknown). */
export function resolveTheme(preference: ThemePreference, prefersDark?: boolean): ResolvedTheme {
  if (preference !== "system") return preference;
  return prefersDark === false ? "light" : "dark";
}

function setResolved(root: HTMLElement, theme: ResolvedTheme) {
  root.classList.toggle("dark", theme === "dark");
  root.dataset["theme"] = theme;
  root.style.colorScheme = theme;
}

function darkQuery(): MediaQueryList | null {
  return typeof window !== "undefined" && typeof window.matchMedia === "function" ? window.matchMedia(DARK_QUERY) : null;
}

/**
 * Applies the preference to `root` (default <html>): toggles `.dark`, sets
 * `data-theme` and `color-scheme`. For `system` it also tracks OS changes; call the
 * returned function to stop tracking (it is a no-op for explicit themes).
 */
export function applyTheme(preference: ThemePreference, root: HTMLElement = document.documentElement): () => void {
  const query = preference === "system" ? darkQuery() : null;
  setResolved(root, resolveTheme(preference, query ? query.matches : undefined));
  if (!query) return () => {};
  const onChange = (e: MediaQueryListEvent) => setResolved(root, e.matches ? "dark" : "light");
  query.addEventListener("change", onChange);
  return () => query.removeEventListener("change", onChange);
}

/**
 * Marks the platform on `root`: `.android` disables backdrop blur on glass surfaces
 * (convLink rule: blur is too costly in the Android WebView), `.ios` / `.web` are
 * informational.
 */
export function setPlatformClass(platform: UiPlatform, root: HTMLElement = document.documentElement): void {
  for (const p of ["ios", "android", "web"] as const) root.classList.toggle(p, p === platform);
}

/** In-app accessibility overrides on top of the OS media queries. */
export function setAccessibilityClasses(
  options: { readonly reduceMotion?: boolean; readonly reduceTransparency?: boolean },
  root: HTMLElement = document.documentElement,
): void {
  if (options.reduceMotion !== undefined) root.classList.toggle("reduce-motion", options.reduceMotion);
  if (options.reduceTransparency !== undefined) root.classList.toggle("reduce-transparency", options.reduceTransparency);
}
