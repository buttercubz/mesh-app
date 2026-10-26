// Dev-only hook for the layout audit (e2e/layout.spec.ts, playwright.layout.config.ts).
// main.tsx loads it only on `vite dev` (`import.meta.env.DEV`) AND with `?simDebug=1`, so a
// production bundle never contains it (the import is removed with the dead branch). It lets
// the harness freeze the engine store on a crafted Snapshot (the simulator's own snapshots
// are then ignored), move the screen stack, change settings, show toasts and emulate the
// native platform, so every banner, sheet and list state can be measured in seconds.
// It holds no secrets and writes nothing the app would not write itself.
import type { Snapshot, TalkDeniedReason } from "@meshapp/engine-api";
import type { ToastInput, UiPlatform } from "@meshapp/ui-kit";
import { setPlatformClass, toaster } from "@meshapp/ui-kit";

import type { AwareStatus } from "../engine/client";
import { engine } from "../engine/client";
import { translate } from "../i18n/index";
import type { EngineData } from "../store/useEngineStore";
import { useEngineStore } from "../store/useEngineStore";
import type { GroupMeta } from "../store/useGroupsStore";
import { useGroupsStore } from "../store/useGroupsStore";
import type { Settings } from "../store/useSettingsStore";
import { useSettingsStore } from "../store/useSettingsStore";
import type { ScreenParams } from "./router";
import { useRouter } from "./router";

/** The handle published as `globalThis.__layoutHarness`. */
export interface LayoutHarness {
  /** The current store snapshot (a JSON copy), or `null`. */
  snapshot(): Snapshot | null;
  /** The last snapshot the engine sent (also while frozen), or `null`. */
  liveSnapshot(): Snapshot | null;
  /**
   * Shows `snapshot` and ignores the engine's snapshots, talk refusals, status changes and
   * toasts until `unfreeze()`. `status` defaults to "running" (a live session).
   */
  freeze(snapshot: Snapshot, extra?: Partial<Pick<EngineData, "status" | "compat">>): void;
  /** Busy banner input: a talk refusal `agoMs` ago (`null` clears it). */
  talkDenied(reason: TalkDeniedReason | null, agoMs?: number): void;
  /** Back to the engine's own state (re-applies the last live snapshot). */
  unfreeze(): void;
  frozen(): boolean;
  /** Clears the stack down to `id`, then pushes `then` (each with its params). */
  go(id: string, params?: ScreenParams, then?: ReadonlyArray<readonly [string, ScreenParams?]>): void;
  stack(): string[];
  settings(patch: Partial<Settings>): void;
  /** Saved groups on Home (engine refreshes are ignored afterwards). */
  groups(list: readonly GroupMeta[]): void;
  toast(input: ToastInput): void;
  clearToasts(): void;
  /** Native look: the `ios`/`android` class on <html> and the engine info platform. */
  platform(platform: UiPlatform): void;
  /** Wi-Fi Aware support as reported to Settings (the web plugin always says unsupported). */
  aware(status: AwareStatus | null): void;
  /** A catalog string in the current language (button names for the harness). */
  t(namespace: string, key: string, params?: Record<string, unknown>): string;
}

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

export function installLayoutHarness(): LayoutHarness {
  const store = useEngineStore;
  const original = {
    applySnapshot: store.getState().applySnapshot,
    talkDenied: store.getState().talkDenied,
    setStatus: store.getState().setStatus,
  };
  const realAwareStatus = engine.awareStatus;
  const realShow = toaster.show.bind(toaster);
  let frozen = false;
  let live: Snapshot | null = store.getState().snapshot;

  // Always remember what the engine sends, so `unfreeze()` can restore it.
  store.setState({
    applySnapshot: (snapshot, at) => {
      live = snapshot;
      if (!frozen) original.applySnapshot(snapshot, at);
    },
    talkDenied: (reason, at) => {
      if (!frozen) original.talkDenied(reason, at);
    },
    setStatus: (status) => {
      if (!frozen) original.setStatus(status);
    },
  });

  // Engine notices (riders joining, refused commands of the frozen state) would pop up
  // at random over a crafted state: only the harness's own toasts show while frozen.
  toaster.show = (input) => (frozen ? -1 : realShow(input));

  const harness: LayoutHarness = {
    snapshot: () => {
      const s = store.getState().snapshot;
      return s === null ? null : clone(s);
    },
    liveSnapshot: () => (live === null ? null : clone(live)),
    freeze: (snapshot, extra = {}) => {
      frozen = true;
      store.setState({
        snapshot: clone(snapshot),
        snapshotAt: Date.now(),
        route: snapshot.audio.route,
        status: extra.status ?? "running",
        ...(extra.compat !== undefined ? { compat: extra.compat } : {}),
      });
    },
    talkDenied: (reason, agoMs = 0) => {
      store.setState({ lastTalkDenied: reason === null ? null : { reason, at: Date.now() - agoMs } });
    },
    unfreeze: () => {
      frozen = false;
      store.setState({ lastTalkDenied: null, compat: "ok" });
      if (live !== null) original.applySnapshot(live);
    },
    frozen: () => frozen,
    go: (id, params, then = []) => {
      const router = useRouter.getState();
      router.reset(id, params);
      for (const [next, p] of then) useRouter.getState().push(next, p);
    },
    stack: () => useRouter.getState().stack.map((r) => r.id),
    settings: (patch) => useSettingsStore.getState().update(patch),
    groups: (list) => {
      useGroupsStore.setState({
        groups: list.map((g) => ({ ...g })),
        activeGroupId: null,
        // Home refreshes the list from the engine on mount: keep the crafted one.
        syncFromEngine: () => {},
      });
    },
    toast: (input) => {
      realShow(input);
    },
    clearToasts: () => toaster.clear(),
    platform: (platform) => {
      setPlatformClass(platform);
      const info = store.getState().info;
      if (info !== null) store.setState({ info: { ...info, platform } });
    },
    aware: (status) => {
      engine.awareStatus = status === null ? realAwareStatus : () => Promise.resolve({ ...status });
    },
    t: (namespace, key, params) => translate(namespace, key, params),
  };
  Object.defineProperty(globalThis, "__layoutHarness", { configurable: true, enumerable: false, value: harness });
  return harness;
}
