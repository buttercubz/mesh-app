// Browser services used by the web implementation, behind one interface so tests (and
// the Phase 1 simulator) can substitute them: page visibility (the web stand-in for
// "foreground-active"), Screen Wake Lock, permissions, file share/download.
import type { PermissionState } from "@capacitor/core";

import type { IntercomPermission } from "../definitions.js";

export type Visibility = "visible" | "hidden";

export interface WakeLockHandle {
  release(): Promise<void>;
}

export interface SharedFile {
  name: string;
  data: string;
  mime: string;
}

export interface WebEnvironment {
  /** `visible` only while the page is in the foreground (no document ⇒ `hidden`). */
  visibility(): Visibility;
  /** Subscribes to visibility changes; returns the unsubscribe function. */
  onVisibilityChange(listener: (visibility: Visibility) => void): () => void;
  /** Wall clock in ms. */
  now(): number;
  /** App version from `<meta name="app-version">`, else a web placeholder. */
  appVersion(): string;
  /** The page's query string (`location.search`, e.g. `?sim=chain5&loss=0.1`); `""` if none. */
  query(): string;
  /** Screen Wake Lock; `null` when unsupported or refused. */
  requestWakeLock(): Promise<WakeLockHandle | null>;
  queryPermission(permission: IntercomPermission): Promise<PermissionState>;
  requestPermission(permission: IntercomPermission): Promise<PermissionState>;
  /** Share sheet when available, else a download; `false` when neither exists. */
  shareFile(file: SharedFile): Promise<boolean>;
}

/** Placeholder version when the page does not declare one. */
export const WEB_APP_VERSION_FALLBACK = "0.0.0-web";

const doc = (): Document | undefined => (typeof document === "undefined" ? undefined : document);
const nav = (): Navigator | undefined => (typeof navigator === "undefined" ? undefined : navigator);

/** Browser permission names behind each intercom permission (radios are simulated). */
const QUERY_NAME: Partial<Record<IntercomPermission, string>> = {
  microphone: "microphone",
  camera: "camera",
  location: "geolocation",
};

function notificationState(): PermissionState {
  if (typeof Notification === "undefined") return "denied";
  const p = Notification.permission;
  return p === "default" ? "prompt" : p;
}

async function queryPermission(permission: IntercomPermission): Promise<PermissionState> {
  if (permission === "bluetooth" || permission === "nearbyWifi") return "granted";
  if (permission === "notifications") return notificationState();
  const name = QUERY_NAME[permission];
  const permissions = nav()?.permissions;
  if (name === undefined || permissions === undefined) return "prompt";
  try {
    const status = await permissions.query({ name: name as PermissionName });
    return status.state;
  } catch (_e) {
    // Browsers without this permission name (e.g. Firefox "microphone"): unknown.
    return "prompt";
  }
}

async function requestMedia(constraints: MediaStreamConstraints): Promise<PermissionState> {
  const media = nav()?.mediaDevices;
  if (media === undefined || typeof media.getUserMedia !== "function") return "denied";
  try {
    const stream = await media.getUserMedia(constraints);
    for (const track of stream.getTracks()) track.stop();
    return "granted";
  } catch (_e) {
    // NotAllowedError (user/policy refusal) or no device: not usable, report denied.
    return "denied";
  }
}

function requestLocation(): Promise<PermissionState> {
  const geo = nav()?.geolocation;
  if (geo === undefined) return Promise.resolve("denied");
  return new Promise((resolve) => {
    geo.getCurrentPosition(
      () => resolve("granted"),
      (error) => {
        // PERMISSION_DENIED is a refusal; timeout/unavailable still means it was granted.
        resolve(error.code === error.PERMISSION_DENIED ? "denied" : "granted");
      },
      { timeout: 10_000, maximumAge: 60_000 },
    );
  });
}

async function requestPermission(permission: IntercomPermission): Promise<PermissionState> {
  switch (permission) {
    case "bluetooth":
    case "nearbyWifi":
      return "granted";
    case "microphone":
      return requestMedia({ audio: true });
    case "camera":
      return requestMedia({ video: true });
    case "location":
      return requestLocation();
    case "notifications": {
      if (typeof Notification === "undefined") return "denied";
      const p = await Notification.requestPermission();
      return p === "default" ? "prompt" : p;
    }
  }
}

async function shareFile(file: SharedFile): Promise<boolean> {
  const n = nav();
  const blob = new Blob([file.data], { type: file.mime });
  if (n !== undefined && typeof n.share === "function" && typeof File !== "undefined") {
    const shared = new File([blob], file.name, { type: file.mime });
    if (typeof n.canShare === "function" && n.canShare({ files: [shared] })) {
      try {
        await n.share({ files: [shared], title: file.name });
      } catch (e) {
        // The user dismissing the share sheet is not an error (native share sheets
        // resolve on cancel too); anything else is.
        if (!(e instanceof DOMException && e.name === "AbortError")) throw e;
      }
      return true;
    }
  }
  const d = doc();
  if (d === undefined || typeof URL.createObjectURL !== "function") return false;
  const url = URL.createObjectURL(blob);
  const a = d.createElement("a");
  a.href = url;
  a.download = file.name;
  a.rel = "noopener";
  d.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 0);
  return true;
}

/** The real browser environment. */
export function browserEnvironment(): WebEnvironment {
  return {
    visibility: () => (doc()?.visibilityState === "visible" ? "visible" : "hidden"),
    onVisibilityChange(listener) {
      const d = doc();
      if (d === undefined) return () => {};
      const handler = (): void => listener(d.visibilityState === "visible" ? "visible" : "hidden");
      d.addEventListener("visibilitychange", handler);
      return () => d.removeEventListener("visibilitychange", handler);
    },
    now: () => Date.now(),
    query: () => (typeof location === "undefined" ? "" : location.search),
    appVersion: () =>
      doc()?.querySelector<HTMLMetaElement>('meta[name="app-version"]')?.content || WEB_APP_VERSION_FALLBACK,
    async requestWakeLock() {
      const wakeLock = nav()?.wakeLock;
      if (wakeLock === undefined) return null;
      try {
        return await wakeLock.request("screen");
      } catch (_e) {
        // NotAllowedError (hidden page, battery saver, policy): best effort, no lock.
        return null;
      }
    },
    queryPermission,
    requestPermission,
    shareFile,
  };
}
