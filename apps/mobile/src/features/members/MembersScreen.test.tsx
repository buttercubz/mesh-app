// @vitest-environment jsdom
// Members (WP-22 acceptance): honest statuses and flags from the snapshot fixture; Lock
// and Remove first show the unconfirmed members of `previewAccessLoss` and run only after
// the leader confirms; Close and Make leader confirm too; members see no leader tools.
import type { Snapshot, UiCommand } from "@meshapp/engine-api";
import { fireEvent, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { fixtureResult, fixtureSnapshot, mountWith, resetAll, running, settle } from "../group/testing";
import { MembersScreen } from "./MembersScreen";

afterEach(resetAll);

const answer = (cmd: UiCommand) => (cmd.type === "previewAccessLoss" ? fixtureResult("accessLoss") : { ok: true as const });
const leaderCommands = (sent: UiCommand[]) =>
  sent.filter((c) => ["lockGroup", "removeMember", "closeGroup", "transferLeader"].includes(c.type));

describe("MembersScreen", () => {
  it("shows status, voice reach, flags, distance and battery per rider", async () => {
    running(fixtureSnapshot("snapshot ride"));
    mountWith(<MembersScreen params={{}} />);
    await settle();
    const row = (name: string) => screen.getByText(name).closest("li") as HTMLElement;
    expect(row("Beto").textContent).toContain("Direct");
    expect(row("Beto").textContent).toContain("Clear voice");
    expect(row("Beto").textContent).toContain("No headset");
    expect(within(row("Beto")).getByTestId("member-distance").textContent).toBe("40 m · south");
    expect(row("Carla").textContent).toContain("Relayed · 3 hops");
    expect(row("Carla").textContent).toContain("Best effort");
    for (const flag of ["Radio off", "Mic paused", "On a call"]) expect(row("Carla").textContent).toContain(flag);
    expect(row("Carla").textContent).toContain("Does not hear you");
    expect(row("Beto").textContent).toContain("Battery 55 %");
  });

  it("Lock group previews the unconfirmed riders before locking", async () => {
    running(fixtureSnapshot("snapshot ride"));
    const { plugin } = mountWith(<MembersScreen params={{}} />, answer);
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Lock group" }));
    await settle();
    expect(plugin.sent).toContainEqual({ type: "previewAccessLoss" });
    const sheet = screen.getByTestId("leader-sheet-lock");
    expect(within(sheet).getByTestId("access-loss").textContent).toContain("Luis");
    expect(within(sheet).getByText("1 unconfirmed riders would lose access")).toBeTruthy();
    expect(leaderCommands(plugin.sent)).toEqual([]);
    // Waiting closes the sheet without locking.
    fireEvent.click(within(sheet).getByRole("button", { name: "Confirm reachable members first" }));
    expect(screen.queryByTestId("leader-sheet-lock")).toBeNull();
    expect(leaderCommands(plugin.sent)).toEqual([]);
    // Again, and proceed.
    fireEvent.click(screen.getByRole("button", { name: "Lock group" }));
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Lock anyway" }));
    await settle();
    expect(leaderCommands(plugin.sent)).toEqual([{ type: "lockGroup" }]);
    expect(screen.queryByTestId("leader-sheet-lock")).toBeNull();
  });

  it("Remove previews the access loss, then removes the chosen member", async () => {
    const snap = fixtureSnapshot("snapshot ride");
    running(snap);
    const { plugin } = mountWith(<MembersScreen params={{}} />, answer);
    await settle();
    fireEvent.click(within(screen.getByText("Beto").closest("li") as HTMLElement).getByRole("button"));
    fireEvent.click(screen.getByRole("button", { name: "Remove from group" }));
    await settle();
    const sheet = screen.getByTestId("leader-sheet-remove");
    expect(screen.getByRole("dialog", { name: "Remove Beto?" })).toBeTruthy();
    expect(sheet.textContent).toContain("Luis");
    expect(leaderCommands(plugin.sent)).toEqual([]);
    fireEvent.click(within(sheet).getByTestId("leader-confirm"));
    await settle();
    const beto = snap.members.find((m) => m.name === "Beto");
    expect(leaderCommands(plugin.sent)).toEqual([{ type: "removeMember", nodeId: beto?.nodeId }]);
  });

  it("Close group and Make leader ask for confirmation", async () => {
    const snap = fixtureSnapshot("snapshot ride");
    running(snap);
    const { plugin } = mountWith(<MembersScreen params={{}} />, answer);
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Close group" }));
    expect(leaderCommands(plugin.sent)).toEqual([]);
    fireEvent.click(within(screen.getByTestId("leader-sheet-close")).getByTestId("leader-confirm"));
    await settle();
    fireEvent.click(within(screen.getByText("Beto").closest("li") as HTMLElement).getByRole("button"));
    fireEvent.click(screen.getByRole("button", { name: "Make leader" }));
    fireEvent.click(within(screen.getByTestId("leader-sheet-transfer")).getByTestId("leader-confirm"));
    await settle();
    const beto = snap.members.find((m) => m.name === "Beto");
    expect(leaderCommands(plugin.sent)).toEqual([{ type: "closeGroup" }, { type: "transferLeader", nodeId: beto?.nodeId }]);
  });

  it("members get no leader tools", async () => {
    const snap = fixtureSnapshot("snapshot ride");
    const member: Snapshot = { ...snap, session: snap.session === null ? null : { ...snap.session, role: "member" } };
    running(member);
    mountWith(<MembersScreen params={{}} />);
    await settle();
    expect(screen.queryByRole("button", { name: "Lock group" })).toBeNull();
    expect(screen.getByText(/Only the leader can remove riders/)).toBeTruthy();
  });
});
