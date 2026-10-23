// Settings store (WP-18, DESIGN §11.3, §1.4): rider preferences, persisted with
// name/version/migrate/merge. No secrets. Non-React code reads it with getState().
import type { PttStyle, RoutePreference, TalkMode, Vehicle, VoxSensitivity } from "@meshapp/engine-api";
import {
  DISPLAY_NAME_MAX_BYTES,
  PTT_STYLES,
  ROUTE_PREFERENCES,
  TALK_MODES,
  VEHICLES,
  VOX_SENSITIVITIES,
} from "@meshapp/engine-api";
import type { ThemePreference } from "@meshapp/ui-kit";
import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

import type { AppLanguage } from "../i18n/index";
import { APP_LANGUAGES, deviceLanguage } from "../i18n/index";
import { appStorage, bool, isRecord, oneOf } from "./storage";

export const THEME_PREFERENCES = ["dark", "light", "system"] as const satisfies readonly ThemePreference[];

export interface Settings {
  language: AppLanguage;
  /** Dark is the default (driving rule, §11.4). */
  theme: ThemePreference;
  vehicle: Vehicle;
  /** Talk mode applied when the ride starts (§1.4). */
  defaultMode: TalkMode;
  pttStyle: PttStyle;
  voxSensitivity: VoxSensitivity;
  /** Spoken prompts on top of the earcons (§7.7). */
  spokenPrompts: boolean;
  /** "Share my distance" (opt-in; backs the iOS location keep-alive anchor). */
  shareDistance: boolean;
  /** Haptics on PTT press/release (opt-in). */
  haptics: boolean;
  /** Announced name, ≤ 24 UTF-8 bytes. */
  displayName: string;
  routePreference: RoutePreference;
  /** Transmit with only the built-in mic (moto default off: wind). */
  builtinMicTx: boolean;
  /** Car: keep the screen on while the phone is mounted. */
  keepScreenOnMounted: boolean;
  onboardingDone: boolean;
  /** Android: the one-time battery-optimisation exemption prompt was shown (§1.3 F0). */
  batteryExemptionAsked: boolean;
}

/** Vehicle-dependent defaults (DESIGN §1.4). */
export function vehicleDefaults(
  vehicle: Vehicle,
): Pick<Settings, "defaultMode" | "pttStyle" | "voxSensitivity" | "builtinMicTx" | "keepScreenOnMounted" | "routePreference"> {
  return vehicle === "car"
    ? {
        defaultMode: "ptt",
        pttStyle: "hold",
        voxSensitivity: "medium",
        builtinMicTx: true,
        keepScreenOnMounted: true,
        routePreference: "auto",
      }
    : {
        defaultMode: "vox",
        pttStyle: "hold",
        voxSensitivity: "medium",
        builtinMicTx: false,
        keepScreenOnMounted: false,
        routePreference: "auto",
      };
}

export function defaultSettings(language: AppLanguage = deviceLanguage()): Settings {
  return {
    language,
    theme: "dark",
    vehicle: "moto",
    ...vehicleDefaults("moto"),
    spokenPrompts: true,
    shareDistance: false,
    haptics: false,
    displayName: "",
    onboardingDone: false,
    batteryExemptionAsked: false,
  };
}

const utf8 = new TextEncoder();

/**
 * Trims and limits a display name to DISPLAY_NAME_MAX_BYTES UTF-8 bytes, cutting only at
 * a grapheme boundary (§1.3 F0), so emoji and accented letters are never split.
 */
export function clampDisplayName(name: string, maxBytes: number = DISPLAY_NAME_MAX_BYTES): string {
  const trimmed = name.replace(/\s+/g, " ").trim();
  if (utf8.encode(trimmed).length <= maxBytes) return trimmed;
  const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });
  let out = "";
  let bytes = 0;
  for (const { segment } of segmenter.segment(trimmed)) {
    const size = utf8.encode(segment).length;
    if (bytes + size > maxBytes) break;
    out += segment;
    bytes += size;
  }
  return out.trimEnd();
}

/** Rebuilds settings from untrusted persisted JSON: unknown keys dropped, bad values defaulted. */
export function sanitizeSettings(raw: unknown, base: Settings = defaultSettings()): Settings {
  if (!isRecord(raw)) return base;
  return {
    language: oneOf(raw["language"], APP_LANGUAGES, base.language),
    theme: oneOf(raw["theme"], THEME_PREFERENCES, base.theme),
    vehicle: oneOf(raw["vehicle"], VEHICLES, base.vehicle),
    defaultMode: oneOf(raw["defaultMode"], TALK_MODES, base.defaultMode),
    pttStyle: oneOf(raw["pttStyle"], PTT_STYLES, base.pttStyle),
    voxSensitivity: oneOf(raw["voxSensitivity"], VOX_SENSITIVITIES, base.voxSensitivity),
    spokenPrompts: bool(raw["spokenPrompts"], base.spokenPrompts),
    shareDistance: bool(raw["shareDistance"], base.shareDistance),
    haptics: bool(raw["haptics"], base.haptics),
    displayName: typeof raw["displayName"] === "string" ? clampDisplayName(raw["displayName"]) : base.displayName,
    routePreference: oneOf(raw["routePreference"], ROUTE_PREFERENCES, base.routePreference),
    builtinMicTx: bool(raw["builtinMicTx"], base.builtinMicTx),
    keepScreenOnMounted: bool(raw["keepScreenOnMounted"], base.keepScreenOnMounted),
    onboardingDone: bool(raw["onboardingDone"], base.onboardingDone),
    batteryExemptionAsked: bool(raw["batteryExemptionAsked"], base.batteryExemptionAsked),
  };
}

export const SETTINGS_STORE_NAME = "meshapp.settings";
export const SETTINGS_STORE_VERSION = 1;

/**
 * Migrations from older persisted versions. Version 0 is the pre-release draft, which
 * stored `promptsEnabled` and `lang`; both map onto the v1 names.
 */
export function migrateSettings(persisted: unknown, version: number): Settings {
  let raw: Record<string, unknown> = isRecord(persisted) ? { ...persisted } : {};
  if (version < 1) {
    const { promptsEnabled, lang, ...rest } = raw;
    raw = { ...rest, spokenPrompts: promptsEnabled ?? rest["spokenPrompts"], language: lang ?? rest["language"] };
  }
  return sanitizeSettings(raw);
}

export interface SettingsState extends Settings {
  /** Updates any subset of settings (values are sanitised). */
  update(patch: Partial<Settings>): void;
  /** Changes the vehicle profile and applies its §1.4 defaults. */
  setVehicle(vehicle: Vehicle): void;
  setDisplayName(name: string): void;
  reset(): void;
  /** Rewrites storage with the sanitised settings (drops unknown persisted keys). */
  scrub(): void;
}

function pickSettings(state: Settings): Settings {
  return sanitizeSettings(state, state);
}

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set, get) => ({
      ...defaultSettings(),
      update: (patch) => set(sanitizeSettings({ ...pickSettings(get()), ...patch }, pickSettings(get()))),
      setVehicle: (vehicle) => set({ vehicle, ...vehicleDefaults(vehicle) }),
      setDisplayName: (name) => set({ displayName: clampDisplayName(name) }),
      reset: () => set(defaultSettings()),
      scrub: () => set(pickSettings(get())),
    }),
    {
      name: SETTINGS_STORE_NAME,
      version: SETTINGS_STORE_VERSION,
      storage: createJSONStorage(() => appStorage),
      partialize: (state) => pickSettings(state),
      migrate: (persisted, version) => migrateSettings(persisted, version),
      merge: (persisted, current) => ({ ...current, ...sanitizeSettings(persisted, pickSettings(current)) }),
      onRehydrateStorage: () => (state) => state?.scrub(),
    },
  ),
);
