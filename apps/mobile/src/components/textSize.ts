// iOS text size (Dynamic Type) for the WebView. WKWebView applies the system text size only
// to content using the -apple-system-* font keywords; this app sizes everything in rem, so
// without this iOS riders who enlarged the system text would always get 100 % while Android
// riders get their font scale (MainActivity clamps it to 100-130 %). Here the body text
// style's size (17 px at the default setting) sets the root font size with the same clamp,
// and is re-read when the app returns to the foreground (the setting is changed in Settings
// while the app is in the background). No-op on Android and browsers.

/** The iOS body text size at the default Dynamic Type setting. */
const IOS_BODY_DEFAULT_PX = 17;
/** The design root size (Tailwind/ui-kit rem). */
const ROOT_PX = 16;
/** Same range as the Android WebView text zoom clamp. */
const MIN_SCALE = 1;
const MAX_SCALE = 1.3;

let installed = false;

/** Root font scale for a measured iOS body size (exported for tests). */
export function textScaleFor(bodyPx: number): number {
  if (!Number.isFinite(bodyPx) || bodyPx <= 0) return MIN_SCALE;
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, bodyPx / IOS_BODY_DEFAULT_PX));
}

function measureBodyPx(): number {
  const probe = document.createElement("span");
  probe.setAttribute("aria-hidden", "true");
  probe.style.cssText = "position:absolute;visibility:hidden;pointer-events:none;font:-apple-system-body";
  probe.textContent = "x";
  document.body.append(probe);
  const px = Number.parseFloat(getComputedStyle(probe).fontSize);
  probe.remove();
  return px;
}

function apply(): void {
  const scale = textScaleFor(measureBodyPx());
  const root = document.documentElement;
  if (scale === MIN_SCALE) root.style.removeProperty("font-size");
  else root.style.fontSize = `${Math.round(ROOT_PX * scale * 100) / 100}px`;
}

/** Starts following the iOS text size (idempotent; iOS only, `html.ios` from setPlatformClass). */
export function installIosTextSize(): void {
  if (installed || typeof document === "undefined" || !document.documentElement.classList.contains("ios")) return;
  installed = true;
  apply();
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") apply();
  });
}
