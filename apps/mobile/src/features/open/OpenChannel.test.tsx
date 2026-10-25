// @vitest-environment jsdom
// Open channels: the Home picker (1-9) with the public notice, joining in one tap
// (joinOpenChannel, startEngine and goLive in order inside the click), the public notice
// on the ride screen, and the local mute / block controls on Members.
import type { Snapshot, UiCommand } from "@meshapp/engine-api";
import { fireEvent, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { useRouter } from "../../app/router";
import { useSettingsStore } from "../../store/useSettingsStore";
import { calls, fixtureSnapshot, mountWith, resetAll, running, settle } from "../group/testing";
import { HomeScreen } from "../home/HomeScreen";
import { MembersScreen } from "../members/MembersScreen";
import { RideScreen } from "../ride/RideScreen";
import { OPEN_CHANNEL_GROUP_IDS } from "./actions";
import { sanitizeOpenPrefs, useOpenChannelStore } from "./store";

afterEach(() => {
  resetAll();
  useOpenChannelStore.setState({ channel: 1, blockedNames: [] });
});

const top = () => useRouter.getState().stack.at(-1)?.id;

function openRide(channel: number | null = 3): Snapshot {
  const s = fixtureSnapshot("snapshot ride");
  if (s.session === null) throw new Error("fixture without session");
  return { ...s, session: { ...s.session, openChannel: channel, role: "member", mode: "ptt" } };
}

describe("Open channel on Home", () => {
  it("picks a channel, shows the public notice and joins in one tap", async () => {
    useSettingsStore.setState({ onboardingDone: true, displayName: "Ana" });
    const { plugin } = mountWith(<HomeScreen params={{}} />, (cmd: UiCommand) =>
      cmd.type === "joinOpenChannel" ? { ok: true, groupId: OPEN_CHANNEL_GROUP_IDS[3] } : cmd.type === "listGroups" ? { ok: true, groups: [] } : { ok: true },
    );
    await settle();
    expect(screen.getByText("Public — anyone on this channel nearby can hear you.")).toBeTruthy();
    const three = screen.getByRole("radio", { name: "Channel 3" });
    fireEvent.click(three);
    expect(three.getAttribute("aria-checked")).toBe("true");
    plugin.calls.length = 0;
    fireEvent.click(screen.getByRole("button", { name: "Go to channel 3" }));
    // All three plugin calls leave synchronously inside the click, in order.
    const order = plugin.calls.map((c) => c.method).filter((m) => m === "send" || m === "startEngine" || m === "gestureCommand");
    expect(order.slice(0, 3)).toEqual(["send", "startEngine", "gestureCommand"]);
    expect(plugin.sent).toContainEqual({ type: "joinOpenChannel", channel: 3, displayName: "Ana" });
    expect(calls(plugin, "startEngine")).toEqual([{ groupId: OPEN_CHANNEL_GROUP_IDS[3] }]);
    expect(plugin.gestured).toEqual([{ type: "goLive" }]);
    await settle();
    await settle();
    expect(top()).toBe("ride");
    expect(plugin.sent).toContainEqual({ type: "setTalkMode", mode: "ptt" });
    expect(useOpenChannelStore.getState().channel).toBe(3);
  });
});

describe("Open channel ride and members", () => {
  it("the ride screen keeps the public notice and names the channel", async () => {
    running(openRide(3));
    mountWith(<RideScreen params={{}} />);
    await settle();
    expect(screen.getByTestId("open-public-notice").textContent).toContain("anyone on this channel");
    expect(screen.getAllByText("Channel 3").length).toBeGreaterThan(0);
  });

  it("a private group ride has no public notice", async () => {
    running(openRide(null));
    mountWith(<RideScreen params={{}} />);
    await settle();
    expect(screen.queryByTestId("open-public-notice")).toBeNull();
  });

  it("mute and block stay local: commands only, block remembered by name", async () => {
    const snap = openRide(3);
    running(snap);
    const other = snap.members.find((m) => m.status !== "me");
    if (other === undefined) throw new Error("fixture without peers");
    const { plugin } = mountWith(<MembersScreen params={{}} />);
    await settle();
    fireEvent.click(screen.getByRole("button", { name: `Mute ${other.name}` }));
    fireEvent.click(screen.getByRole("button", { name: `Block ${other.name}` }));
    await settle();
    expect(plugin.sent).toContainEqual({ type: "setMemberMuted", nodeId: other.nodeId, muted: true });
    expect(plugin.sent).toContainEqual({ type: "setMemberBlocked", nodeId: other.nodeId, blocked: true });
    expect(useOpenChannelStore.getState().blockedNames).toEqual([other.name]);
  });

  it("persisted preferences are sanitised", () => {
    expect(sanitizeOpenPrefs({ channel: 12, blockedNames: ["A", 3, "", "A"] })).toEqual({ channel: 1, blockedNames: ["A"] });
    expect(sanitizeOpenPrefs(null)).toEqual({ channel: 1, blockedNames: [] });
  });
});
