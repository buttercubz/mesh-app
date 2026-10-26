// @vitest-environment jsdom
// Ride screen (WP-22 acceptance): keep-screen-on in PTT (and car "while mounted"), VOX
// disabled on the loudspeaker, PTT through the plugin's fast path, the busy banner on a
// talkDenied fixture, and Go live / Resume (mic) as ★ commands through gestureCommand.
import type { Snapshot } from "@meshapp/engine-api";
import { act, fireEvent, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { useSettingsStore } from "../../store/useSettingsStore";
import { calls, emit, fixtureEvent, fixtureSnapshot, mountWith, resetAll, running, settle } from "../group/testing";
import { RideScreen } from "./RideScreen";

afterEach(resetAll);

function ride(patch: { mode?: "vox" | "ptt" | "muted"; live?: boolean; ladder?: "L0" | "L2"; route?: Snapshot["audio"]["route"] }): Snapshot {
  const s = fixtureSnapshot("snapshot ride");
  if (s.session === null) throw new Error("fixture without session");
  return {
    ...s,
    session: {
      ...s.session,
      mode: patch.mode ?? s.session.mode,
      live: patch.live ?? s.session.live,
      ladder: patch.ladder ?? "L0",
      talk: "idle",
    },
    audio: { ...s.audio, route: patch.route ?? s.audio.route },
  };
}

const keepOn = (plugin: Parameters<typeof calls>[0]) => calls(plugin, "setKeepScreenOn");

describe("RideScreen keep-screen-on", () => {
  it("PTT mode: setKeepScreenOn(true) on mount, false on unmount", async () => {
    running(ride({ mode: "ptt" }));
    const { plugin, unmount } = mountWith(<RideScreen params={{}} />);
    await settle();
    expect(keepOn(plugin)).toEqual([{ on: true }]);
    unmount();
    expect(keepOn(plugin)).toEqual([{ on: true }, { on: false }]);
  });

  it("VOX on a motorcycle lets the phone lock; a mounted car keeps it awake", async () => {
    running(ride({ mode: "vox" }));
    const moto = mountWith(<RideScreen params={{}} />);
    await settle();
    expect(keepOn(moto.plugin)).toEqual([]);
    moto.unmount();
    act(() => useSettingsStore.getState().setVehicle("car"));
    const car = mountWith(<RideScreen params={{}} />);
    await settle();
    expect(keepOn(car.plugin)).toEqual([{ on: true }]);
  });
});

describe("RideScreen talk control", () => {
  it("disables VOX on the phone loudspeaker and falls back to PTT", async () => {
    running(ride({ mode: "vox", route: "builtinSpeaker" }));
    const { plugin } = mountWith(<RideScreen params={{}} />);
    await settle();
    expect(screen.getByRole("radio", { name: "VOX" })).toHaveProperty("disabled", true);
    // The why lives in the status chip's details, one tap away.
    expect(screen.getByTestId("route-chip").textContent).toBe("Phone speaker");
    fireEvent.click(screen.getByTestId("ride-status-chip"));
    expect(screen.getByText(/VOX needs a headset/)).toBeTruthy();
    expect(plugin.sent).toContainEqual({ type: "setTalkMode", mode: "ptt" });
  });

  it("PTT uses the plugin fast path (down, then up)", async () => {
    running(ride({ mode: "ptt" }));
    const { plugin } = mountWith(<RideScreen params={{}} />);
    await settle();
    const button = screen.getByRole("button", { name: "Push to talk" });
    fireEvent.pointerDown(button, { pointerId: 1, button: 0, pointerType: "touch" });
    fireEvent.pointerUp(button, { pointerId: 1, pointerType: "touch" });
    await settle();
    expect(calls(plugin, "ptt").map((a) => (a as { down: boolean }).down)).toEqual([true, false]);
  });

  it("tapping the big button in VOX mutes", async () => {
    running(ride({ mode: "vox" }));
    const { plugin } = mountWith(<RideScreen params={{}} />);
    await settle();
    fireEvent.click(screen.getByTestId("mute-toggle"));
    await settle();
    expect(plugin.sent).toContainEqual({ type: "setTalkMode", mode: "muted" });
  });

  it("shows the busy banner when the engine denies talk (talkDenied fixture)", async () => {
    running(ride({ mode: "ptt" }));
    mountWith(<RideScreen params={{}} />);
    await settle();
    emit(fixtureEvent("talkDenied"));
    expect(screen.getByRole("alert").textContent).toMatch(/Channel full|leader is talking|Network busy/);
  });

  it("Go live and Resume mic go through gestureCommand from the tap", async () => {
    running(ride({ mode: "ptt", live: false }));
    const first = mountWith(<RideScreen params={{}} />);
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Go live" }));
    expect(first.plugin.gestured).toContainEqual({ type: "goLive" });
    expect(first.plugin.sent.some((c) => c.type === "goLive")).toBe(false);
    first.unmount();

    running(ride({ mode: "ptt", ladder: "L2" }));
    const second = mountWith(<RideScreen params={{}} />);
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect(second.plugin.gestured).toContainEqual({ type: "resumeMic" });
  });

  it("the wind banner turns the phone mic on with a plain command (no gesture, no mic-paused)", async () => {
    const s = ride({ mode: "ptt" });
    running({ ...s, audio: { ...s.audio, txMutedNoHeadset: true, captureOk: false } });
    const { plugin } = mountWith(<RideScreen params={{}} />);
    await settle();
    expect(screen.queryByText("Mic paused")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Use phone mic" }));
    await settle();
    expect(plugin.sent).toContainEqual({ type: "setBuiltinMicTx", allowed: true });
    expect(plugin.gestured).toEqual([]);
    expect(useSettingsStore.getState().builtinMicTx).toBe(true);
  });

  it("shows only the most urgent alert as a bar; the chip opens every alert in full in the details sheet", async () => {
    const s = ride({ mode: "ptt", live: false });
    running({ ...s, device: { ...s.device, bluetooth: "off" } });
    const { plugin } = mountWith(<RideScreen params={{}} />);
    await settle();
    const shown = () => [...document.querySelectorAll("[data-banner]")].map((b) => b.getAttribute("data-banner"));
    expect(shown()).toEqual(["bluetoothOff"]);
    // The bar carries the title and the fix, not the message.
    expect(screen.queryByText(/airplane mode/)).toBeNull();
    expect(screen.getByTestId("ride-status-chip").textContent).toContain("1 more");
    fireEvent.click(screen.getByTestId("ride-status-chip"));
    // The sheet lists the primary alert in full, then the collapsed one, each with its fix
    // intact: Go live is still a ★ command issued from the tap.
    const details = screen.getByTestId("ride-status-details");
    expect([...details.querySelectorAll("[data-banner]")].map((b) => b.getAttribute("data-banner"))).toEqual(["bluetoothOff", "notLive"]);
    expect(details.textContent).toMatch(/airplane mode/);
    fireEvent.click(screen.getByRole("button", { name: "Go live" }));
    expect(plugin.gestured).toContainEqual({ type: "goLive" });
  });

  it("lists speaking riders with their level", async () => {
    running(ride({ mode: "vox" }));
    mountWith(<RideScreen params={{}} />);
    await settle();
    const chips = screen.getAllByTestId("talker-chip");
    expect(chips.map((c) => c.textContent)).toEqual([expect.stringContaining("You"), expect.stringContaining("Carla")]);
    expect(screen.getByTestId("heard-by").textContent).toMatch(/^Heard by 1 of \d+$/);
  });
});
