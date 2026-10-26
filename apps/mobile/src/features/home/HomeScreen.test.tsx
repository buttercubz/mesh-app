// @vitest-environment jsdom
// Home, Create and Join (WP-22, DESIGN §1.3 F0-F2, F7): first-launch welcome, Resume
// ride after process death (startEngine + goLive in the same tap), create → lobby,
// join confirm with expired / leader-signature warnings.
import type { UiCommand } from "@meshapp/engine-api";
import { fireEvent, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { useRouter } from "../../app/router";
import { useGroupsStore } from "../../store/useGroupsStore";
import { useSettingsStore } from "../../store/useSettingsStore";
import { CreateGroupScreen } from "../group/CreateGroupScreen";
import { JoinScreen } from "../group/JoinScreen";
import { calls, fixtureResult, mountWith, resetAll, settle } from "../group/testing";
import { HomeScreen } from "./HomeScreen";

afterEach(resetAll);

const GID = "5f2c9a0d4b7e41c8a3d6e9f01b2c3d4e";
const URL = "convlinkvoice://join?v=1#abc";
const top = () => useRouter.getState().stack.at(-1)?.id;

describe("HomeScreen", () => {
  it("welcomes a first-time rider and leads to onboarding", () => {
    useSettingsStore.setState({ onboardingDone: false });
    mountWith(<HomeScreen params={{}} />);
    expect(screen.getByTestId("welcome")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Get started" }));
    expect(top()).toBe("onboarding");
  });

  it("offers Resume ride after process death: startEngine and goLive leave in the tap", async () => {
    useGroupsStore.setState({ groups: [{ id: GID, name: "Sunday ride", role: "member", lastUsed: 1 }], activeGroupId: GID });
    const { plugin } = mountWith(<HomeScreen params={{}} />, (cmd: UiCommand) =>
      cmd.type === "listGroups"
        ? { ok: true, groups: [{ groupId: GID, name: "Sunday ride", role: "member", vehicle: "moto", members: 3, epoch: 0, closed: false }] }
        : { ok: true },
    );
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Resume ride" }));
    // Both calls happen synchronously inside the click, before any promise settles.
    const order = plugin.calls.map((c) => c.method).filter((m) => m === "startEngine" || m === "gestureCommand");
    expect(order).toEqual(["startEngine", "gestureCommand"]);
    expect(calls(plugin, "startEngine")).toEqual([{ groupId: GID }]);
    expect(plugin.gestured).toEqual([{ type: "goLive" }]);
    await settle();
    expect(top()).toBe("lobby");
    expect(plugin.sent).toContainEqual({ type: "setDisplayName", name: "Ana" });
  });

  it("Join scans the invite and opens the confirm screen with the URL in memory", async () => {
    const { plugin } = mountWith(<HomeScreen params={{}} />);
    plugin.scanInviteQr = async () => ({ url: URL, cancelled: false });
    fireEvent.click(screen.getByRole("button", { name: "Join a ride" }));
    await settle();
    expect(useRouter.getState().stack.at(-1)).toMatchObject({ id: "join", params: { url: URL } });
  });
});

describe("Create and Join", () => {
  it("creates the group, opens the lobby and applies the invite expiry", async () => {
    const { plugin } = mountWith(<CreateGroupScreen params={{}} />, (cmd: UiCommand) =>
      cmd.type === "createGroup" ? fixtureResult("groupId createGroup") : { ok: true },
    );
    fireEvent.change(screen.getByLabelText("Group name"), { target: { value: "Sunday ride" } });
    fireEvent.change(screen.getByLabelText("Invite expires after"), { target: { value: "4" } });
    fireEvent.click(screen.getByTestId("create-confirm"));
    await settle();
    await settle();
    expect(plugin.sent).toContainEqual({ type: "createGroup", name: "Sunday ride", vehicle: "moto" });
    expect(calls(plugin, "startEngine")).toEqual([{ groupId: GID }]);
    expect(plugin.sent).toContainEqual({ type: "setInviteExpiry", hours: 4 });
    expect(top()).toBe("lobby");
    expect(useGroupsStore.getState().groups[0]).toMatchObject({ id: GID, name: "Sunday ride", role: "leader" });
  });

  it("join confirm shows the group, then joins with the rider's name", async () => {
    const { plugin } = mountWith(<JoinScreen params={{ url: URL }} />, (cmd: UiCommand) =>
      cmd.type === "parseInvite" ? fixtureResult("inviteInfo valid") : cmd.type === "joinGroup" ? fixtureResult("groupId joinGroup") : { ok: true },
    );
    await settle();
    expect(screen.getByTestId("join-group-name").textContent).toBe("Sunday ride");
    fireEvent.click(screen.getByTestId("join-confirm"));
    await settle();
    await settle();
    expect(plugin.sent).toContainEqual({ type: "joinGroup", url: URL, displayName: "Ana" });
    expect(top()).toBe("lobby");
  });

  it("warns about an expired invite and an unverified leader signature", async () => {
    const info = { ...fixtureResult("inviteInfo valid"), expired: true, leaderSigOk: false };
    mountWith(<JoinScreen params={{ url: URL }} />, () => info);
    await settle();
    expect(screen.getByText("This invite has expired")).toBeTruthy();
    expect(screen.getByText("Leader signature not verified")).toBeTruthy();
    expect(screen.getByTestId("join-confirm")).toHaveProperty("disabled", true);
  });

  it("the pinned confirm buttons submit their form, so the keyboard's Go key works", async () => {
    // Create: the footer button belongs to the form (implicit submission from either input).
    const { plugin } = mountWith(<CreateGroupScreen params={{}} />, (cmd: UiCommand) =>
      cmd.type === "createGroup" ? fixtureResult("groupId createGroup") : { ok: true },
    );
    const groupName = screen.getByLabelText("Group name") as HTMLInputElement;
    const create = screen.getByTestId("create-confirm") as HTMLButtonElement;
    expect(create.type).toBe("submit");
    expect(create.form).not.toBeNull();
    expect(create.form).toBe(groupName.form);
    expect(groupName.getAttribute("enterkeyhint")).toBe("go");
    fireEvent.change(groupName, { target: { value: "Sunday ride" } });
    fireEvent.submit(groupName.form as HTMLFormElement);
    await settle();
    expect(plugin.sent).toContainEqual({ type: "createGroup", name: "Sunday ride", vehicle: "moto" });
  });

  it("join confirm submits the name form", async () => {
    const { plugin } = mountWith(<JoinScreen params={{ url: URL }} />, (cmd: UiCommand) =>
      cmd.type === "parseInvite" ? fixtureResult("inviteInfo valid") : cmd.type === "joinGroup" ? fixtureResult("groupId joinGroup") : { ok: true },
    );
    await settle();
    const name = screen.getByLabelText("Your name") as HTMLInputElement;
    const join = screen.getByTestId("join-confirm") as HTMLButtonElement;
    expect(join.form).not.toBeNull();
    expect(join.form).toBe(name.form);
    expect(name.getAttribute("enterkeyhint")).toBe("go");
    fireEvent.submit(name.form as HTMLFormElement);
    await settle();
    expect(plugin.sent).toContainEqual({ type: "joinGroup", url: URL, displayName: "Ana" });
  });

  it("rejects an invalid invite", async () => {
    mountWith(<JoinScreen params={{ url: URL }} />, () => fixtureResult("inviteInfo invalid"));
    await settle();
    expect(screen.getByText("This is not a valid invite")).toBeTruthy();
    expect(screen.queryByTestId("join-confirm")).toBeNull();
  });
});
