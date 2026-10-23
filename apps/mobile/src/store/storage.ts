// Persistence backend for the persisted stores (settings, groups). Only non-secret UI
// state is ever written here (DESIGN §11.3): keys, epoch secrets and invite text live in
// the native secure store and never reach JS storage.
import type { StateStorage } from "zustand/middleware";

/** In-memory storage (tests, private browsing, storage quota errors). */
export function createMemoryStorage(): StateStorage & { dump(): Record<string, string> } {
  const data = new Map<string, string>();
  return {
    getItem: (name) => data.get(name) ?? null,
    setItem: (name, value) => {
      data.set(name, value);
    },
    removeItem: (name) => {
      data.delete(name);
    },
    dump: () => Object.fromEntries(data),
  };
}

const fallback = createMemoryStorage();

function localStorageOrNull(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch (_e) {
    // Accessing localStorage throws when storage is disabled (sandboxed WebView).
    return null;
  }
}

/** The WebView's localStorage when usable, else an in-memory map. */
export const appStorage: StateStorage = {
  getItem(name) {
    const ls = localStorageOrNull();
    if (ls === null) return fallback.getItem(name);
    try {
      return ls.getItem(name);
    } catch (_e) {
      // Storage became unavailable mid-session: serve the in-memory copy.
      return fallback.getItem(name);
    }
  },
  setItem(name, value) {
    const ls = localStorageOrNull();
    if (ls !== null) {
      try {
        ls.setItem(name, value);
        return;
      } catch (_e) {
        // Quota exceeded or storage disabled: keep the value for this session only.
      }
    }
    void fallback.setItem(name, value);
  },
  removeItem(name) {
    const ls = localStorageOrNull();
    try {
      ls?.removeItem(name);
    } catch (_e) {
      // Nothing to remove when storage is unavailable; the fallback is cleared below.
    }
    void fallback.removeItem(name);
  },
};

/** `true` for a plain JSON object (persisted state is untrusted input). */
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** `value` when it is one of `allowed`, else `fallback`. */
export function oneOf<const T extends string>(value: unknown, allowed: readonly T[], fallbackValue: T): T {
  return typeof value === "string" && (allowed as readonly string[]).includes(value) ? (value as T) : fallbackValue;
}

/** `value` when boolean, else `fallback`. */
export function bool(value: unknown, fallbackValue: boolean): boolean {
  return typeof value === "boolean" ? value : fallbackValue;
}
