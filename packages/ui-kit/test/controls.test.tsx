// Button, SegmentedControl, ListRow, Badge and Banner behaviour.
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Badge } from "../src/components/Badge.js";
import { Banner } from "../src/components/Banner.js";
import { Button } from "../src/components/Button.js";
import { ListRow, ListSection } from "../src/components/ListRow.js";
import { SegmentedControl, nextEnabledIndex } from "../src/components/SegmentedControl.js";

describe("Button", () => {
  it("defaults to type=button and maps variant/size/icon/block to classes", () => {
    render(
      <Button variant="danger" size="lg" iconOnly block aria-label="Remove">
        x
      </Button>,
    );
    const button = screen.getByRole("button", { name: "Remove" });
    expect(button.getAttribute("type")).toBe("button");
    expect(button.className).toBe("mk-button mk-button--danger mk-button--lg mk-button--icon mk-button--block");
  });

  it("forwards native props and the ref", () => {
    const onClick = vi.fn();
    let node: HTMLButtonElement | null = null;
    render(
      <Button type="submit" onClick={onClick} ref={(el) => void (node = el)}>
        Go
      </Button>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Go" }));
    expect(onClick).toHaveBeenCalledTimes(1);
    expect(node).toBeInstanceOf(HTMLButtonElement);
    expect(screen.getByRole("button").getAttribute("type")).toBe("submit");
  });
});

describe("SegmentedControl", () => {
  const options = [
    { value: "ptt", label: "PTT" },
    { value: "vox", label: "VOX" },
    { value: "off", label: "Off", disabled: true },
    { value: "mute", label: "Mute" },
  ] as const;

  it("nextEnabledIndex wraps and skips disabled options", () => {
    const enabled = [true, true, false, true];
    expect(nextEnabledIndex(enabled, 1, 1)).toBe(3);
    expect(nextEnabledIndex(enabled, 3, 1)).toBe(0);
    expect(nextEnabledIndex(enabled, 0, -1)).toBe(3);
    expect(nextEnabledIndex([true, false], 0, 1)).toBe(0);
  });

  it("is a radiogroup with one tab stop on the selected option", () => {
    render(<SegmentedControl options={options} value="vox" onChange={() => {}} label="Talk mode" />);
    expect(screen.getByRole("radiogroup", { name: "Talk mode" })).toBeTruthy();
    const radios = screen.getAllByRole("radio");
    expect(radios.map((r) => r.getAttribute("aria-checked"))).toEqual(["false", "true", "false", "false"]);
    expect(radios.map((r) => r.tabIndex)).toEqual([-1, 0, -1, -1]);
    expect((radios[2] as HTMLButtonElement).disabled).toBe(true);
  });

  it("clicking selects; clicking the selected option does not re-fire", () => {
    const onChange = vi.fn();
    render(<SegmentedControl options={options} value="ptt" onChange={onChange} label="Talk mode" />);
    fireEvent.click(screen.getByRole("radio", { name: "PTT" }));
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("radio", { name: "VOX" }));
    expect(onChange).toHaveBeenCalledWith("vox");
  });

  it("arrow keys, Home and End move the selection over enabled options", () => {
    const onChange = vi.fn();
    render(<SegmentedControl options={options} value="vox" onChange={onChange} label="Talk mode" />);
    const group = screen.getByRole("radiogroup");
    fireEvent.keyDown(group, { key: "ArrowRight" });
    expect(onChange).toHaveBeenLastCalledWith("mute");
    expect(document.activeElement).toBe(screen.getByRole("radio", { name: "Mute" }));
    fireEvent.keyDown(group, { key: "ArrowLeft" });
    expect(onChange).toHaveBeenLastCalledWith("ptt");
    fireEvent.keyDown(group, { key: "End" });
    expect(onChange).toHaveBeenLastCalledWith("mute");
    fireEvent.keyDown(group, { key: "Home" });
    expect(onChange).toHaveBeenLastCalledWith("ptt");
    fireEvent.keyDown(group, { key: "a" });
    expect(onChange).toHaveBeenCalledTimes(4);
  });

  it("a disabled group ignores clicks and keys", () => {
    const onChange = vi.fn();
    render(<SegmentedControl options={options} value="ptt" onChange={onChange} label="Talk mode" disabled />);
    fireEvent.click(screen.getByRole("radio", { name: "VOX" }));
    fireEvent.keyDown(screen.getByRole("radiogroup"), { key: "ArrowRight" });
    expect(onChange).not.toHaveBeenCalled();
  });
});

describe("ListRow / ListSection / Badge", () => {
  it("is a static row without onClick and a button with it", () => {
    const onClick = vi.fn();
    const { rerender } = render(<ListRow title="Ana" subtitle="2 hops" trailing={<Badge tone="relayed">relayed</Badge>} />);
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getByText("2 hops").className).toBe("mk-list-row-subtitle");
    rerender(<ListRow title="Ana" onClick={onClick} />);
    fireEvent.click(screen.getByRole("button", { name: "Ana" }));
    expect(onClick).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button").className).toContain("mk-list-row--interactive");
  });

  it("section renders its heading and rows", () => {
    render(
      <ListSection title="Members">
        <ListRow title="Ana" />
      </ListSection>,
    );
    expect(screen.getByRole("heading", { name: "Members" })).toBeTruthy();
    expect(screen.getByText("Ana")).toBeTruthy();
  });

  it("badge tone maps to a modifier class", () => {
    render(
      <>
        <Badge>plain</Badge>
        <Badge tone="blocked">full</Badge>
      </>,
    );
    expect(screen.getByText("plain").className).toBe("mk-badge");
    expect(screen.getByText("full").className).toBe("mk-badge mk-badge--blocked");
    expect(screen.getByText("full").dataset["tone"]).toBe("blocked");
  });
});

describe("Banner", () => {
  it("urgent tones are alerts, others are status", () => {
    const { rerender } = render(<Banner tone="danger" title="Bluetooth is off" />);
    expect(screen.getByRole("alert").textContent).toContain("Bluetooth is off");
    rerender(<Banner tone="warn" title="Saving battery" message="Level 2" />);
    expect(screen.getByRole("status").textContent).toContain("Level 2");
  });

  it("runs its action", () => {
    const onClick = vi.fn();
    render(<Banner tone="warn" title="Mic paused" action={{ label: "Resume", onClick }} />);
    fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect(onClick).toHaveBeenCalledTimes(1);
  });
});

describe("Avatar", () => {
  it("shows up to two initials, a stable per-rider hue and is decorative", async () => {
    const { Avatar, avatarHue, initials } = await import("../src/components/Avatar.js");
    expect(initials("Ana María López")).toBe("AL");
    expect(initials("ñandú")).toBe("Ñ");
    expect(initials("  ")).toBe("?");
    expect(avatarHue("Carla")).toBe(avatarHue(" carla "));
    expect(avatarHue("Carla")).toBeGreaterThanOrEqual(0);
    expect(avatarHue("Carla")).toBeLessThan(360);
    const { container } = render(<Avatar name="Carla Ruiz" level={0.5} speaking dot="active" />);
    const el = container.querySelector(".mk-avatar") as HTMLElement;
    expect(el.getAttribute("aria-hidden")).toBe("true");
    expect(el.textContent).toBe("CR");
    expect(el.dataset["speaking"]).toBe("true");
    expect(el.style.getPropertyValue("--mk-avatar-hue")).toBe(String(avatarHue("Carla Ruiz")));
    expect(container.querySelector(".mk-avatar-ring-level")).not.toBeNull();
    expect(container.querySelector('.mk-avatar-dot[data-tone="active"]')).not.toBeNull();
  });
});
