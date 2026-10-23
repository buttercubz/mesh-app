/**
 * Minimal toast queue (no dependency): the app's single event subscription maps engine
 * notices to `toaster.show(...)`; <Toaster /> renders the queue. Framework-free so
 * non-React code can call it (like Zustand's getState()).
 */

export type ToastTone = "neutral" | "success" | "warn" | "danger";

export interface ToastInput {
  readonly title: string;
  readonly description?: string;
  readonly tone?: ToastTone;
  /** Auto-dismiss delay; 0 keeps the toast until dismissed. Default 4000 ms. */
  readonly durationMs?: number;
  /** Same-key toasts replace each other instead of stacking (e.g. a repeated notice). */
  readonly key?: string;
}

export interface ToastItem {
  readonly id: number;
  readonly title: string;
  readonly description?: string;
  readonly tone: ToastTone;
  readonly durationMs: number;
  readonly key?: string;
}

export interface ToastStore {
  show(input: ToastInput): number;
  dismiss(id: number): void;
  clear(): void;
  list(): ReadonlyArray<ToastItem>;
  subscribe(listener: () => void): () => void;
}

export const DEFAULT_TOAST_MS = 4000;
/** Oldest toasts are dropped beyond this many (riders must not face a wall of text). */
export const MAX_TOASTS = 3;

/** Injectable timers (tests use fakes). */
export interface TimerApi {
  readonly set: (fn: () => void, ms: number) => unknown;
  readonly clear: (handle: unknown) => void;
}

const defaultTimers: TimerApi = {
  set: (fn, ms) => setTimeout(fn, ms),
  clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export function createToaster(timers: TimerApi = defaultTimers): ToastStore {
  let items: ReadonlyArray<ToastItem> = [];
  let nextId = 1;
  const handles = new Map<number, unknown>();
  const listeners = new Set<() => void>();

  const publish = (next: ReadonlyArray<ToastItem>) => {
    items = next;
    for (const listener of [...listeners]) listener();
  };

  const stopTimer = (id: number) => {
    const handle = handles.get(id);
    if (handle !== undefined) {
      timers.clear(handle);
      handles.delete(id);
    }
  };

  const dismiss = (id: number) => {
    stopTimer(id);
    if (items.some((t) => t.id === id)) publish(items.filter((t) => t.id !== id));
  };

  return {
    show(input) {
      const id = nextId++;
      const item: ToastItem = {
        id,
        title: input.title,
        description: input.description,
        tone: input.tone ?? "neutral",
        durationMs: Math.max(0, input.durationMs ?? DEFAULT_TOAST_MS),
        key: input.key,
      };
      let kept = items;
      if (item.key !== undefined) {
        for (const t of kept) if (t.key === item.key) stopTimer(t.id);
        kept = kept.filter((t) => t.key !== item.key);
      }
      kept = [...kept, item];
      while (kept.length > MAX_TOASTS) {
        const dropped = kept[0];
        if (dropped) stopTimer(dropped.id);
        kept = kept.slice(1);
      }
      if (item.durationMs > 0) handles.set(id, timers.set(() => dismiss(id), item.durationMs));
      publish(kept);
      return id;
    },
    dismiss,
    clear() {
      for (const id of [...handles.keys()]) stopTimer(id);
      if (items.length) publish([]);
    },
    list: () => items,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

/** Process-wide toaster used by <Toaster /> when no other instance is passed. */
export const toaster: ToastStore = createToaster();
