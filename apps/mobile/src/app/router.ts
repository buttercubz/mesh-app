// Tiny screen-stack router (WP-18, DESIGN §11.3). Features register their screens with
// `registerScreen(id, component)` from their `src/features/<name>/index.ts` (loaded by
// app/features.ts); navigation is a Zustand stack. Params live in memory only, so a
// transient value such as an invite URL from a deep link is never persisted.
import type { ComponentType } from "react";
import { create } from "zustand";

/** Values a screen can receive (serialisable, never secrets that must outlive the screen). */
export type ScreenParamValue = string | number | boolean | null;
export type ScreenParams = Readonly<Record<string, ScreenParamValue>>;

export interface ScreenProps {
  readonly params: ScreenParams;
}

export type ScreenComponent = ComponentType<ScreenProps>;

export interface ScreenOptions {
  /**
   * The ride screen and similar: the Android back button does not pop it (riders must
   * not leave a ride by accident); a back handler on the screen decides instead.
   */
  readonly blockBack?: boolean;
}

export interface ScreenEntry {
  readonly id: string;
  readonly component: ScreenComponent;
  readonly options: ScreenOptions;
}

/** Well-known screen ids shared between the shell and the features (WP-21/22). */
export const SCREEN = {
  /** Start screen (WP-22 home; the shell placeholder until it exists). */
  home: "home",
  /** Join confirmation; receives `{ url }` from a `convlinkvoice://join` deep link. */
  join: "join",
  onboarding: "onboarding",
  lobby: "lobby",
  ride: "ride",
  members: "members",
  settings: "settings",
  fieldTest: "fieldTest",
} as const;

export const INITIAL_SCREEN: string = SCREEN.home;

const registry = new Map<string, ScreenEntry>();

const ID = /^[a-z][A-Za-z0-9.-]*$/;

export interface Route {
  readonly id: string;
  readonly params: ScreenParams;
  /** Unique per push, used as the React key so a re-pushed screen remounts. */
  readonly key: number;
}

export interface RouterState {
  stack: readonly Route[];
  /** Bumped on every registration so the shell re-renders late registrations. */
  registryVersion: number;
  push(id: string, params?: ScreenParams): void;
  /** Replaces the top screen. */
  replace(id: string, params?: ScreenParams): void;
  /** Pops the top screen; `false` when already at the root. */
  back(): boolean;
  /** Clears the stack down to one screen (e.g. after leaving a group). */
  reset(id?: string, params?: ScreenParams): void;
}

let nextKey = 1;
const route = (id: string, params: ScreenParams = {}): Route => ({ id, params: { ...params }, key: nextKey++ });

export const useRouter = create<RouterState>()((set, get) => ({
  stack: [route(INITIAL_SCREEN)],
  registryVersion: 0,
  push: (id, params) => set({ stack: [...get().stack, route(id, params)] }),
  replace: (id, params) => set({ stack: [...get().stack.slice(0, -1), route(id, params)] }),
  back: () => {
    const { stack } = get();
    if (stack.length <= 1) return false;
    set({ stack: stack.slice(0, -1) });
    return true;
  },
  reset: (id = INITIAL_SCREEN, params) => set({ stack: [route(id, params)] }),
}));

/** Registers (or replaces, for hot reload) the component of screen `id`. */
export function registerScreen(id: string, component: ScreenComponent, options: ScreenOptions = {}): void {
  if (!ID.test(id)) throw new Error(`invalid screen id "${id}"`);
  registry.set(id, { id, component, options });
  useRouter.setState((s) => ({ registryVersion: s.registryVersion + 1 }));
}

/** The registered screen, or `undefined`. */
export function getScreen(id: string): ScreenEntry | undefined {
  return registry.get(id);
}

/** Every registered screen id, sorted. */
export function registeredScreenIds(): string[] {
  return [...registry.keys()].sort();
}

/** Removes a registration (tests). */
export function unregisterScreen(id: string): void {
  if (registry.delete(id)) useRouter.setState((s) => ({ registryVersion: s.registryVersion + 1 }));
}

/** The route on top of the stack. */
export function currentRoute(): Route {
  const { stack } = useRouter.getState();
  return stack[stack.length - 1] ?? route(INITIAL_SCREEN);
}

// ---------------------------------------------------------------------------------
// Back handling (Android hardware back, convLink useBackButton pattern)
// ---------------------------------------------------------------------------------

/** Returns `true` when it consumed the back press. */
export type BackHandler = () => boolean;

const backHandlers: BackHandler[] = [];

/** Adds a handler that sees back presses before the stack; returns its remover. */
export function pushBackHandler(handler: BackHandler): () => void {
  backHandlers.push(handler);
  return () => {
    const i = backHandlers.lastIndexOf(handler);
    if (i >= 0) backHandlers.splice(i, 1);
  };
}

/**
 * Handles one back press: the newest handler first, then the stack (unless the top
 * screen blocks back). Returns `false` when nothing consumed it (the caller then sends
 * the app to the background; it never kills a running intercom).
 */
export function handleBackPress(): boolean {
  for (let i = backHandlers.length - 1; i >= 0; i -= 1) {
    const handler = backHandlers[i];
    if (handler !== undefined && handler()) return true;
  }
  if (getScreen(currentRoute().id)?.options.blockBack === true) return true;
  return useRouter.getState().back();
}
