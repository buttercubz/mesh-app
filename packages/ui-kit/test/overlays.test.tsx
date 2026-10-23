// Sheet, Toast store and Toaster behaviour.
import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Sheet } from "../src/components/Sheet.js";
import { Toaster } from "../src/components/Toast.js";
import { DEFAULT_TOAST_MS, MAX_TOASTS, createToaster, type TimerApi } from "../src/components/toast-store.js";

describe("Sheet", () => {
  it("renders nothing while closed", () => {
    render(
      <Sheet open={false} onClose={() => {}} title="Invite" closeLabel="Close">
        body
      </Sheet>,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("is a labelled modal dialog that takes focus and locks scrolling", () => {
    const opener = document.createElement("button");
    document.body.append(opener);
    opener.focus();
    const { rerender } = render(
      <Sheet open onClose={() => {}} title="Invite" closeLabel="Close">
        body
      </Sheet>,
    );
    const dialog = screen.getByRole("dialog", { name: "Invite" });
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(document.activeElement).toBe(dialog);
    expect(document.body.style.overflow).toBe("hidden");

    rerender(
      <Sheet open={false} onClose={() => {}} title="Invite" closeLabel="Close">
        body
      </Sheet>,
    );
    expect(document.body.style.overflow).toBe("");
    expect(document.activeElement).toBe(opener);
    opener.remove();
  });

  it("closes on Escape, the close button and a scrim tap, not a panel tap", () => {
    const onClose = vi.fn();
    render(
      <Sheet open onClose={onClose} title="Invite" closeLabel="Close">
        <p>body</p>
      </Sheet>,
    );
    const dialog = screen.getByRole("dialog");
    fireEvent.click(screen.getByText("body"));
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.keyDown(dialog, { key: "Escape" });
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    fireEvent.click(dialog.parentElement as HTMLElement);
    expect(onClose.mock.calls.map((c) => c[0])).toEqual(["escape", "button", "scrim"]);
  });

  it("Tab wraps focus inside the panel", () => {
    render(
      <Sheet open onClose={() => {}} title="Invite" closeLabel="Close">
        <button type="button">Share</button>
      </Sheet>,
    );
    const close = screen.getByRole("button", { name: "Close" });
    const share = screen.getByRole("button", { name: "Share" });
    share.focus();
    fireEvent.keyDown(share, { key: "Tab" });
    expect(document.activeElement).toBe(close);
    fireEvent.keyDown(close, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(share);
  });
});

/** Manual timers: `run()` fires everything due by `now`. */
function fakeTimers() {
  let now = 0;
  let seq = 0;
  const pending = new Map<number, { at: number; fn: () => void }>();
  const api: TimerApi = {
    set: (fn, ms) => {
      const id = ++seq;
      pending.set(id, { at: now + ms, fn });
      return id;
    },
    clear: (handle) => {
      pending.delete(handle as number);
    },
  };
  const advance = (ms: number) => {
    now += ms;
    for (const [id, t] of [...pending]) {
      if (t.at <= now) {
        pending.delete(id);
        t.fn();
      }
    }
  };
  return { api, advance, pending: () => pending.size };
}

describe("toast store", () => {
  it("auto-dismisses after the default duration; 0 keeps the toast", () => {
    const timers = fakeTimers();
    const store = createToaster(timers.api);
    store.show({ title: "Joined" });
    store.show({ title: "Sticky", durationMs: 0 });
    timers.advance(DEFAULT_TOAST_MS - 1);
    expect(store.list().map((t) => t.title)).toEqual(["Joined", "Sticky"]);
    timers.advance(1);
    expect(store.list().map((t) => t.title)).toEqual(["Sticky"]);
    expect(timers.pending()).toBe(0);
  });

  it("same key replaces, the queue is capped and dropped timers are cleared", () => {
    const timers = fakeTimers();
    const store = createToaster(timers.api);
    store.show({ title: "Weak link 1", key: "weak" });
    store.show({ title: "Weak link 2", key: "weak", tone: "warn" });
    expect(store.list().map((t) => t.title)).toEqual(["Weak link 2"]);
    expect(store.list()[0]?.tone).toBe("warn");
    for (let i = 0; i < MAX_TOASTS + 1; i++) store.show({ title: `n${i}` });
    expect(store.list().length).toBe(MAX_TOASTS);
    expect(store.list().map((t) => t.title)).toEqual(["n1", "n2", "n3"]);
    expect(timers.pending()).toBe(MAX_TOASTS);
  });

  it("dismiss and clear stop timers and notify only on change", () => {
    const timers = fakeTimers();
    const store = createToaster(timers.api);
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);
    const id = store.show({ title: "a" });
    store.show({ title: "b" });
    store.dismiss(id);
    store.dismiss(id);
    expect(listener).toHaveBeenCalledTimes(3);
    store.clear();
    store.clear();
    expect(listener).toHaveBeenCalledTimes(4);
    expect(timers.pending()).toBe(0);
    unsubscribe();
    store.show({ title: "c" });
    expect(listener).toHaveBeenCalledTimes(4);
  });
});

describe("Toaster", () => {
  it("renders the queue and dismisses through the store", () => {
    const store = createToaster(fakeTimers().api);
    render(<Toaster toaster={store} dismissLabel="Dismiss" label="Notifications" />);
    act(() => {
      store.show({ title: "Capture lost", tone: "danger" });
      store.show({ title: "Ana joined", description: "2 hops" });
    });
    expect(screen.getByRole("list", { name: "Notifications" }).children.length).toBe(2);
    expect(screen.getByRole("alert").textContent).toContain("Capture lost");
    expect(screen.getByRole("status").textContent).toContain("2 hops");
    fireEvent.click(screen.getAllByRole("button", { name: "Dismiss" })[0] as HTMLElement);
    expect(store.list().map((t) => t.title)).toEqual(["Ana joined"]);
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
