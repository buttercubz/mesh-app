// Whether the mounted screen pins a footer (set by ScreenLayout, read by AppToaster to
// show fewer toasts above the footer). A tiny external store: one owner at a time, since
// only the top screen of the stack is mounted.
let docked = false;
const listeners = new Set<() => void>();

/** Called by ScreenLayout while a footer is pinned. */
export function setToastDock(next: boolean): void {
  if (docked === next) return;
  docked = next;
  for (const listener of [...listeners]) listener();
}

export function subscribeToastDock(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export const readToastDock = (): boolean => docked;
