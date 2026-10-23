import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { BigPttButton, PTT_RING_CIRCUMFERENCE } from "../src/components/BigPttButton.js";
import type { PttMode } from "../src/components/ptt-gesture.js";
import { pointer, setVisibility } from "./helpers.js";

function setup(mode: PttMode = "hold", extra: { level?: number; disabled?: boolean } = {}) {
  const onDown = vi.fn();
  const onUp = vi.fn();
  const view = render(
    <BigPttButton mode={mode} onDown={onDown} onUp={onUp} level={extra.level ?? 0} label="Talk" disabled={extra.disabled} />,
  );
  const button = screen.getByRole("button", { name: "Talk" });
  return { onDown, onUp, button, view };
}

afterEach(() => {
  setVisibility("visible");
});

describe("BigPttButton (hold)", () => {
  it("fires onDown on pointerdown and onUp on pointerup", () => {
    const { onDown, onUp, button } = setup();
    pointer(button, "pointerDown");
    expect(onDown).toHaveBeenCalledTimes(1);
    expect(onUp).not.toHaveBeenCalled();
    expect(button.dataset["talking"]).toBe("true");
    expect(button.dataset["status"]).toBe("active");
    pointer(button, "pointerUp");
    expect(onUp).toHaveBeenCalledTimes(1);
    expect(onUp).toHaveBeenCalledWith("pointerup");
    expect(button.dataset["talking"]).toBe("false");
  });

  it("fires onUp on pointercancel", () => {
    const { onDown, onUp, button } = setup();
    pointer(button, "pointerDown");
    pointer(button, "pointerCancel");
    expect(onDown).toHaveBeenCalledTimes(1);
    expect(onUp).toHaveBeenCalledTimes(1);
    expect(onUp).toHaveBeenCalledWith("pointercancel");
  });

  it("fires onUp when the page becomes hidden (visibilitychange)", () => {
    const { onUp, button } = setup();
    pointer(button, "pointerDown");
    setVisibility("hidden");
    expect(onUp).toHaveBeenCalledTimes(1);
    expect(onUp).toHaveBeenCalledWith("hidden");
    // The finger lifting later must not produce a second onUp.
    pointer(button, "pointerUp");
    expect(onUp).toHaveBeenCalledTimes(1);
  });

  it("visibilitychange to visible does not release", () => {
    const { onUp, button } = setup();
    pointer(button, "pointerDown");
    setVisibility("visible");
    expect(onUp).not.toHaveBeenCalled();
  });

  it("fires onUp when pointer capture is lost without pointerup", () => {
    const { onUp, button } = setup();
    pointer(button, "pointerDown");
    pointer(button, "lostPointerCapture");
    expect(onUp).toHaveBeenCalledWith("lostpointercapture");
  });

  it("captures the pointer on press", () => {
    const capture = vi.spyOn(HTMLElement.prototype, "setPointerCapture");
    const { button } = setup();
    pointer(button, "pointerDown", { pointerId: 42 });
    expect(capture).toHaveBeenCalledWith(42);
  });

  it("still works when setPointerCapture throws", () => {
    vi.spyOn(HTMLElement.prototype, "setPointerCapture").mockImplementation(() => {
      throw new DOMException("InvalidStateError");
    });
    const { onDown, onUp, button } = setup();
    pointer(button, "pointerDown");
    pointer(button, "pointerUp");
    expect(onDown).toHaveBeenCalledTimes(1);
    expect(onUp).toHaveBeenCalledTimes(1);
  });

  it("ignores secondary mouse buttons and second fingers", () => {
    const { onDown, onUp, button } = setup();
    pointer(button, "pointerDown", { pointerType: "mouse", button: 2 });
    expect(onDown).not.toHaveBeenCalled();
    pointer(button, "pointerDown", { pointerId: 1 });
    pointer(button, "pointerDown", { pointerId: 2 });
    pointer(button, "pointerUp", { pointerId: 2 });
    expect(onUp).not.toHaveBeenCalled();
    pointer(button, "pointerUp", { pointerId: 1 });
    expect(onDown).toHaveBeenCalledTimes(1);
    expect(onUp).toHaveBeenCalledTimes(1);
  });

  it("fires onUp on unmount while held", () => {
    const { onUp, button, view } = setup();
    pointer(button, "pointerDown");
    view.unmount();
    expect(onUp).toHaveBeenCalledWith("unmount");
  });

  it("does not fire onUp on unmount when idle", () => {
    const { onUp, view } = setup();
    view.unmount();
    setVisibility("hidden");
    expect(onUp).not.toHaveBeenCalled();
  });

  it("releases when disabled while held and ignores presses while disabled", () => {
    const onDown = vi.fn();
    const onUp = vi.fn();
    const ui = (disabled: boolean) => (
      <BigPttButton mode="hold" onDown={onDown} onUp={onUp} level={0} label="Talk" disabled={disabled} />
    );
    const view = render(ui(false));
    const button = screen.getByRole("button", { name: "Talk" });
    pointer(button, "pointerDown");
    view.rerender(ui(true));
    expect(onUp).toHaveBeenCalledWith("disabled");
    pointer(button, "pointerDown");
    expect(onDown).toHaveBeenCalledTimes(1);
  });

  it("supports Space / Enter as hold keys (no auto-repeat presses)", () => {
    const { onDown, onUp, button } = setup();
    fireEvent.keyDown(button, { key: " " });
    fireEvent.keyDown(button, { key: " ", repeat: true });
    expect(onDown).toHaveBeenCalledTimes(1);
    fireEvent.keyUp(button, { key: " " });
    expect(onUp).toHaveBeenCalledWith("keyup");
    fireEvent.keyDown(button, { key: "Enter" });
    fireEvent.blur(button);
    expect(onUp).toHaveBeenCalledTimes(2);
  });

  it("uses the latest callbacks after a re-render", () => {
    const first = vi.fn();
    const second = vi.fn();
    const ui = (onUp: () => void) => <BigPttButton mode="hold" onDown={() => {}} onUp={onUp} level={0} label="Talk" />;
    const view = render(ui(first));
    const button = screen.getByRole("button", { name: "Talk" });
    pointer(button, "pointerDown");
    view.rerender(ui(second));
    setVisibility("hidden");
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
  });
});

describe("BigPttButton (latch)", () => {
  it("latches on the first tap and releases on the second", () => {
    const { onDown, onUp, button } = setup("latch");
    pointer(button, "pointerDown");
    pointer(button, "pointerUp");
    expect(onDown).toHaveBeenCalledTimes(1);
    expect(onUp).not.toHaveBeenCalled();
    expect(button.getAttribute("aria-pressed")).toBe("true");
    pointer(button, "pointerDown");
    expect(onUp).toHaveBeenCalledWith("toggle");
    pointer(button, "pointerUp");
    expect(onUp).toHaveBeenCalledTimes(1);
    expect(button.getAttribute("aria-pressed")).toBe("false");
  });

  it("releases a latched talk on visibilitychange and on pointercancel of the latching press", () => {
    const { onUp, button } = setup("latch");
    pointer(button, "pointerDown");
    pointer(button, "pointerUp");
    setVisibility("hidden");
    expect(onUp).toHaveBeenCalledWith("hidden");
    setVisibility("visible");
    pointer(button, "pointerDown");
    pointer(button, "pointerCancel");
    expect(onUp).toHaveBeenLastCalledWith("pointercancel");
    expect(onUp).toHaveBeenCalledTimes(2);
  });

  it("switching mode while talking releases", () => {
    const onUp = vi.fn();
    const ui = (mode: PttMode) => <BigPttButton mode={mode} onDown={() => {}} onUp={onUp} level={0} label="Talk" />;
    const view = render(ui("latch"));
    const button = screen.getByRole("button", { name: "Talk" });
    pointer(button, "pointerDown");
    pointer(button, "pointerUp");
    view.rerender(ui("hold"));
    expect(onUp).toHaveBeenCalledWith("mode");
    expect(button.hasAttribute("aria-pressed")).toBe(false);
  });
});

describe("BigPttButton rendering", () => {
  it("draws the input level as the ring", () => {
    const { view } = setup("hold", { level: 0.25 });
    const ring = view.getByTestId("ptt-level");
    expect(Number(ring.getAttribute("stroke-dashoffset"))).toBeCloseTo(PTT_RING_CIRCUMFERENCE * 0.75, 5);
    view.rerender(<BigPttButton mode="hold" onDown={() => {}} onUp={() => {}} level={7} label="Talk" />);
    expect(Number(ring.getAttribute("stroke-dashoffset"))).toBeCloseTo(0, 5);
  });

  it("uses the 160 px class, blocks touch scrolling and shows the engine status", () => {
    const view = render(
      <BigPttButton mode="hold" onDown={() => {}} onUp={() => {}} level={0} label="Talk" status="blocked" />,
    );
    const button = view.getByRole("button", { name: "Talk" });
    expect(button.classList.contains("mk-ptt")).toBe(true);
    expect(button.dataset["status"]).toBe("blocked");
    expect(button.getAttribute("type")).toBe("button");
    expect(fireEvent.contextMenu(button)).toBe(false); // long-press menu suppressed
  });

  it("uses ariaLabel as the accessible name when given", () => {
    render(<BigPttButton mode="hold" onDown={() => {}} onUp={() => {}} level={0} label="🎙" ariaLabel="Push to talk" />);
    expect(screen.getByRole("button", { name: "Push to talk" })).toBeTruthy();
  });
});
