// Device matrix of the layout audit (portrait phones and the Galaxy Tab, CSS px). The
// insets are what env(safe-area-inset-*) reports on the device: Android WebView is
// edge-to-edge at targetSdk 35+ (status bar + gesture or 3-button nav bar), iOS runs
// with viewport-fit=cover and Capacitor `contentInset: "never"`.

export type Platform = "android" | "ios";
export type Locale = "es" | "en";

export interface LayoutDevice {
  readonly id: string;
  readonly platform: Platform;
  readonly label: string;
  readonly width: number;
  readonly height: number;
  readonly deviceScaleFactor: number;
  readonly insets: { readonly top: number; readonly bottom: number; readonly left: number; readonly right: number };
  /** Android WebView textZoom values (percent) of the full audit; iOS: 100 only. */
  readonly zooms: readonly number[];
  readonly userAgent: string;
}

const ANDROID_UA =
  "Mozilla/5.0 (Linux; Android 15; SM-A556B Build/AP3A.240905.015; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/141.0.0.0 Mobile Safari/537.36";
const IOS_UA =
  "Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";

/** Android: 100/130/160 % are the real-world range; 200 % (Android 14 nonlinear) is the extreme. */
const ANDROID_ZOOMS = [100, 130, 160, 200] as const;
/**
 * iOS runs at 100 % by default. The app maps Dynamic Type to the root font size (clamped
 * to 100-130 %, components/textSize.ts), so LAYOUT_IOS_ZOOMS=100,130 audits that range.
 */
const IOS_ZOOMS: readonly number[] = (process.env["LAYOUT_IOS_ZOOMS"] ?? "100")
  .split(",")
  .map((z) => Number(z.trim()))
  .filter((z) => Number.isFinite(z) && z > 0);
/** Zoom levels whose problems are reported at a lower severity. */
export const EXTREME_ZOOMS: ReadonlySet<number> = new Set([200]);

const android = (id: string, label: string, width: number, height: number, top: number, bottom: number, dsf: number): LayoutDevice => ({
  id,
  platform: "android",
  label,
  width,
  height,
  deviceScaleFactor: dsf,
  insets: { top, bottom, left: 0, right: 0 },
  zooms: ANDROID_ZOOMS,
  userAgent: ANDROID_UA,
});

const ios = (id: string, label: string, width: number, height: number, top: number, bottom: number, dsf: number): LayoutDevice => ({
  id,
  platform: "ios",
  label,
  width,
  height,
  deviceScaleFactor: dsf,
  insets: { top, bottom, left: 0, right: 0 },
  zooms: IOS_ZOOMS,
  userAgent: IOS_UA,
});

export const DEVICES: readonly LayoutDevice[] = [
  android("a-small", "Android small (360x640, 3-button nav)", 360, 640, 24, 48, 2),
  android("a-galaxy", "Galaxy S2x/A5x (360x780, gesture nav)", 360, 780, 32, 24, 3),
  android("a-large", "Pixel 7 / Galaxy Plus (412x915)", 412, 915, 32, 24, 2.625),
  android("a-tablet", "Galaxy Tab S10 FE portrait (800x1280)", 800, 1280, 24, 32, 2),
  ios("i-se", "iPhone SE 2/3 (375x667)", 375, 667, 20, 0, 2),
  ios("i-mini", "iPhone 12/13 mini (375x812)", 375, 812, 50, 34, 3),
  ios("i-std", "iPhone 12-15 (390x844, Dynamic Island 59)", 390, 844, 59, 34, 3),
  ios("i-plus", "iPhone 15 Plus / Pro Max (430x932)", 430, 932, 59, 34, 3),
  ios("i-zoom", "375-class iPhone, Display Zoom Larger Text (320x693)", 320, 693, 47, 34, 3),
  // Opt-in (LAYOUT_EXTRA_DEVICES=1): windows the portrait lock does not guarantee — a large
  // screen at a future targetSdk (landscape tablet) and phone split-screen.
  ...(process.env["LAYOUT_EXTRA_DEVICES"] === "1"
    ? [
        android("t-land", "Galaxy Tab S10 FE landscape (1280x800)", 1280, 800, 24, 48, 2),
        android("a-split", "Android phone split-screen half (360x380)", 360, 380, 24, 24, 3),
      ]
    : []),
];

export const LOCALES: readonly Locale[] = ["es", "en"];

export function deviceById(id: string): LayoutDevice {
  const d = DEVICES.find((x) => x.id === id);
  if (d === undefined) throw new Error(`unknown layout device ${id}`);
  return d;
}

/** The CI regression subset: (device, zoom, locale) combinations with assertions. */
export const CI_SUBSET: ReadonlyArray<{ readonly device: string; readonly zoom: number; readonly locale: Locale }> = [
  { device: "a-small", zoom: 130, locale: "es" },
  { device: "i-se", zoom: 100, locale: "es" },
  { device: "i-std", zoom: 100, locale: "en" },
  { device: "a-tablet", zoom: 100, locale: "es" },
];
