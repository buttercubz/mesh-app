// @vitest-environment jsdom
// Lobby (WP-22 acceptance): keep-screen-on on mount/unmount, Go live through
// gestureCommand with the muted-state text, the native QR (getInvite) auto-hidden after
// hideAfterMs and never kept, ready checks, Start ride for the leader.
import type { Snapshot } from "@meshapp/engine-api";
import { act, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useRouter } from "../../app/router";
import { calls, fixtureResult, fixtureSnapshot, mountWith, resetAll, running, settle } from "../group/testing";
import { LobbyScreen } from "./LobbyScreen";

afterEach(() => {
  vi.useRealTimers();
  resetAll();
});

function lobby(live: boolean, role: "leader" | "member" = "leader"): Snapshot {
  const s = fixtureSnapshot("snapshot ride");
  if (s.session === null) throw new Error("fixture without session");
  return { ...s, talkers: [], session: { ...s.session, ride: "lobby", live, role, mode: "muted", talk: "muted" } };
}

const answer = (cmd: { type: string }) => (cmd.type === "getInvite" ? fixtureResult("invite") : { ok: true as const });

describe("LobbyScreen", () => {
  it("keeps the screen awake while mounted", async () => {
    running(lobby(false));
    const { plugin, unmount } = mountWith(<LobbyScreen params={{}} />);
    await settle();
    expect(calls(plugin, "setKeepScreenOn")).toEqual([{ on: true }]);
    unmount();
    expect(calls(plugin, "setKeepScreenOn")).toEqual([{ on: true }, { on: false }]);
  });

  it("Go live is a ★ command from the tap; live shows the muted-until-ride text", async () => {
    running(lobby(false));
    const { plugin } = mountWith(<LobbyScreen params={{}} />);
    await settle();
    fireEvent.click(screen.getByTestId("go-live"));
    expect(plugin.gestured).toEqual([{ type: "goLive" }]);
    expect(plugin.sent.some((c) => c.type === "goLive")).toBe(false);
    await settle();
    running(lobby(true));
    expect(screen.getByTestId("mic-muted-status").textContent).toContain("Mic ready, muted until the ride starts");
    fireEvent.click(screen.getByTestId("start-ride"));
    await settle();
    expect(plugin.sent).toContainEqual({ type: "rideControl", op: "start" });
  });

  it("members wait for the leader and may start the ride locally (late join, leader away)", async () => {
    const member = lobby(true, "member");
    const withLeader = (status: "direct" | "lost"): Snapshot => ({
      ...member,
      members: member.members.map((m) =>
        m.status === "me" ? { ...m, role: "member" as const } : m.name === "Beto" ? { ...m, role: "leader" as const, status } : m,
      ),
    });
    running(withLeader("direct"));
    const { plugin } = mountWith(<LobbyScreen params={{}} />);
    await settle();
    expect(screen.getByText(/^Waiting for the leader to start the ride/)).toBeTruthy();
    expect(screen.getByTestId("start-ride").textContent).toBe("Start here");
    running(withLeader("lost"));
    expect(screen.getByText(/^The leader is out of range/)).toBeTruthy();
    fireEvent.click(screen.getByTestId("start-ride"));
    await settle();
    expect(plugin.sent).toContainEqual({ type: "rideControl", op: "start" });
  });

  it("shows the native QR with its warning and hides it after hideAfterMs", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    running(lobby(true));
    const { plugin } = mountWith(<LobbyScreen params={{}} />, answer);
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Show QR code" }));
    await settle();
    expect(plugin.sent).toContainEqual({ type: "getInvite", groupId: lobby(true).session?.groupId });
    const img = screen.getByTestId("invite-qr-image");
    expect(img.getAttribute("src")).toBe("data:image/png;base64,iVBORw0KGgo=");
    expect(screen.getByText(/A screenshot of this code lets anyone join/)).toBeTruthy();
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(screen.queryByTestId("invite-qr-image")).toBeNull();
    expect(screen.getByRole("button", { name: "Show again" })).toBeTruthy();
  });

  it("lists ready checks, unconfirmed riders and best-effort voice", async () => {
    running(lobby(true));
    mountWith(<LobbyScreen params={{}} />);
    await settle();
    const list = screen.getByTestId("ready-list");
    const carla = within(list).getByText("Carla").closest("li");
    expect(carla?.textContent).toContain("Best effort");
    expect(list.querySelector('[data-status="unconfirmed"]')?.textContent).toContain("Unconfirmed");
    expect(screen.getByTestId("lobby-reach").textContent).toMatch(/best effort/);
  });

  it("moves to the ride screen when the ride starts", async () => {
    running(lobby(true));
    mountWith(<LobbyScreen params={{}} />);
    act(() => useRouter.getState().push("lobby"));
    await settle();
    running(fixtureSnapshot("snapshot ride"));
    await settle();
    expect(useRouter.getState().stack.at(-1)?.id).toBe("ride");
  });
});
