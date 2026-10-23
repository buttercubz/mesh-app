// @vitest-environment jsdom
// Field test component tests (WP-21 acceptance): the screen driven by the mesh-api
// fixture events (crates/mesh-api/fixtures/events.json, results.json) through the app's
// single event router, against the recording fake plugin.
import { readFileSync } from "node:fs";
import type { CommandResult, UiCommand, UiEvent } from "@meshapp/engine-api";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createAppEngine, handleEngineEvent } from "../../engine/client";
import { applyLanguage } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { useLabStore } from "../../store/useLabStore";
import { createFakePlugin, flush } from "../../../tests/helpers";
import type { FakePlugin } from "../../../tests/helpers";
import { LabEngineContext } from "./context";
import { FieldTestScreen } from "./FieldTestScreen";
import { useFieldTestStore } from "./store";
import { repoFile } from "./testing";

const fixtures = <T,>(file: string) =>
  JSON.parse(readFileSync(repoFile(`crates/mesh-api/fixtures/${file}`), "utf8")) as T;
const EVENTS = fixtures<{ name: string; event: UiEvent }[]>("events.json");
const RESULTS = fixtures<{ name: string; result: CommandResult }[]>("results.json");
const event = (name: string): UiEvent => {
  const found = EVENTS.find((e) => e.name === name);
  if (found === undefined) throw new Error(`event fixture ${name}`);
  return found.event;
};
const result = (name: string): CommandResult => {
  const found = RESULTS.find((r) => r.name === name);
  if (found === undefined) throw new Error(`result fixture ${name}`);
  return found.result;
};
const sinks = { toast: () => {}, now: () => 0 };
const emit = (name: string) => act(() => handleEngineEvent(event(name), sinks));

function mount(): FakePlugin {
  const plugin = createFakePlugin();
  plugin.answer = (cmd: UiCommand) => {
    if (cmd.type === "labStart") return result("labRun");
    if (cmd.type === "labListRuns") return result("labRuns");
    return { ok: true };
  };
  const engine = createAppEngine(plugin, { setStatus: () => {}, setInfo: () => {}, warn: () => {} });
  render(
    <LabEngineContext value={engine}>
      <FieldTestScreen params={{}} />
    </LabEngineContext>,
  );
  return plugin;
}

async function join(): Promise<void> {
  fireEvent.change(screen.getByLabelText("Lab code"), { target: { value: "4711" } });
  fireEvent.click(screen.getByRole("button", { name: "Join" }));
  await act(flush);
  await act(flush);
}

const methods = (plugin: FakePlugin, name: string) => plugin.calls.filter((c) => c.method === name).map((c) => c.args);

afterEach(() => {
  cleanup();
  useFieldTestStore.getState().reset();
  useLabStore.getState().reset();
  useEngineStore.getState().reset();
  applyLanguage("en");
});

describe("FieldTestScreen", () => {
  it("joins a Lab code and lists stored runs with verdict chips", async () => {
    const plugin = mount();
    await act(flush);
    expect(methods(plugin, "startEngine")).toEqual([]);
    await join();
    expect(methods(plugin, "startEngine")).toEqual([{ lab: { code: "4711" } }]);
    expect(screen.getByTestId("lab-joined").textContent).toBe("Lab code 4711");
    const results = screen.getByTestId("lab-results");
    const chips = within(results).getAllByText(/^(Go|Interrupted)$/);
    expect(chips.map((c) => c.textContent)).toEqual(["Interrupted", "Go"]);
  });

  it("rejects a short code without starting the engine", async () => {
    const plugin = mount();
    fireEvent.change(screen.getByLabelText("Lab code"), { target: { value: "47" } });
    fireEvent.click(screen.getByRole("button", { name: "Join" }));
    await act(flush);
    expect(methods(plugin, "startEngine")).toEqual([]);
    expect(screen.getByText("Enter 4 digits.")).toBeTruthy();
  });

  it("lists every bench with its go/no-go threshold", () => {
    mount();
    const list = screen.getByTestId("lab-benches");
    for (const bench of ["T0", "T0b", "T0c", "T1", "T2", "T3", "T5a", "T6", "T5b", "T8", "voxCal"]) {
      expect(within(list).getByRole("button", { name: `Set up ${bench}` })).toBeTruthy();
    }
    expect(list.textContent).toContain("p95 one-way < 60 ms");
    expect(list.textContent).toContain("rejoin ≤ 10 s");
    expect(within(list).getAllByText("Phase 1")).toHaveLength(3);
  });

  it("shows the live view from fixture labProgress events", async () => {
    mount();
    await emit("labProgress first");
    const live = screen.getByTestId("lab-live");
    expect(live.textContent).toContain("Run r-20261023-0912-ab12");
    expect(screen.getByTestId("lab-anchor-state").getAttribute("data-anchor-ok")).toBe("false");
    await emit("labProgress");
    expect(screen.getByTestId("lab-big-tx").textContent).toContain("93.4");
    expect(screen.getByTestId("lab-big-loss").textContent).toContain("0.4");
    expect(screen.getByTestId("lab-big-rtt").textContent).toContain("62");
    expect(screen.getByTestId("lab-big-rssi").textContent).toContain("-61");
    expect(screen.getByTestId("lab-big-dist").textContent).toContain("12");
    expect(screen.getByTestId("lab-elapsed").textContent).toBe("00:12");
    // The first sample's dropped anchor is still inside the 60 s window.
    expect(screen.getByText("Keep-alive anchor lost — these seconds are excluded")).toBeTruthy();
    expect(screen.getAllByRole("img", { name: /last 60 s/ })).toHaveLength(5);
    expect(screen.getByRole("img", { name: "TX, last 60 s, now 93.4 kbps" })).toBeTruthy();
  });

  it("shows the finished run with its verdict and exports its files", async () => {
    const plugin = mount();
    await emit("labProgress");
    await emit("labFinished");
    expect(screen.queryByTestId("lab-live")).toBeNull();
    const card = screen.getByTestId("lab-finished");
    expect(within(card).getByText("Go").getAttribute("data-verdict")).toBe("go");
    expect(card.textContent).toContain("01:00");
    expect(card.textContent).toContain("92.8 kbps");
    expect(card.textContent).toContain("31.2 ms");
    fireEvent.click(within(card).getByRole("button", { name: "Export files" }));
    await act(flush);
    expect(methods(plugin, "shareFile")).toEqual([
      { path: "lab/r-20261023-0912-ab12-4711.jsonl", mime: "application/x-ndjson" },
      { path: "lab/r-20261023-0912-ab12-summary.csv", mime: "text/csv" },
    ]);
  });

  it("shows an inconclusive run without percentiles", async () => {
    mount();
    await emit("labFinished inconclusive");
    const card = screen.getByTestId("lab-finished");
    expect(within(card).getByText("Inconclusive")).toBeTruthy();
    expect(card.textContent).toContain("–");
  });

  it("T1 config: audio and anchor pre-selected, Start sends labStart through gestureCommand", async () => {
    const plugin = mount();
    fireEvent.click(screen.getByRole("button", { name: "Set up T1" }));
    const sheet = screen.getByTestId("lab-config");
    const audio = within(within(sheet).getByTestId("lab-audio")).getByRole("radio", { name: "Output only" });
    const anchor = within(within(sheet).getByTestId("lab-anchor")).getByRole("radio", { name: "Audio" });
    expect(audio.getAttribute("aria-checked")).toBe("true");
    expect(anchor.getAttribute("aria-checked")).toBe("true");
    const start = within(sheet).getByRole("button", { name: "Start run" }) as HTMLButtonElement;
    expect(start.disabled).toBe(true); // not joined yet
    expect(methods(plugin, "setKeepScreenOn")).toEqual([{ on: true }]);

    act(() => useFieldTestStore.getState().patch({ code: "4711" }));
    expect(start.disabled).toBe(false);
    fireEvent.click(start);
    expect(plugin.gestured).toHaveLength(1);
    expect(plugin.gestured[0]).toMatchObject({
      type: "labStart",
      bench: "T1",
      params: { audio: "renderOnly", anchor: "audio", gps: false, variant: "mount:mounted" },
    });
    await act(flush);
    await act(flush);
    // Started: the sheet closes and the screen may sleep again.
    expect(screen.queryByTestId("lab-config")).toBeNull();
    expect(methods(plugin, "setKeepScreenOn")).toEqual([{ on: true }, { on: false }]);
    expect(useFieldTestStore.getState().started?.bench).toBe("T1");
    expect(screen.getByTestId("lab-live").textContent).toContain("T1 · Range between vehicles");
  });

  it("T0 headset off starts with plain send; the headset variant switches to full audio", async () => {
    const plugin = mount();
    act(() => useFieldTestStore.getState().patch({ code: "4711" }));
    fireEvent.click(screen.getByRole("button", { name: "Set up T0" }));
    const sheet = screen.getByTestId("lab-config");
    fireEvent.click(within(sheet).getByRole("radio", { name: "Headset active" }));
    expect(within(within(sheet).getByTestId("lab-audio")).getByRole("radio", { name: "Full" }).getAttribute("aria-checked")).toBe("true");
    fireEvent.click(within(within(sheet).getByRole("radiogroup", { name: "Headset" })).getByRole("radio", { name: "Off" }));
    fireEvent.click(within(within(sheet).getByTestId("lab-audio")).getByRole("radio", { name: "Off" }));
    fireEvent.click(within(sheet).getByRole("radio", { name: "Peripheral (B, swapped)" }));
    fireEvent.click(within(sheet).getByRole("button", { name: "Start run" }));
    await act(flush);
    expect(plugin.gestured).toEqual([]);
    expect(plugin.sent.find((c) => c.type === "labStart")).toMatchObject({
      params: { audio: "off", anchor: "audio", role: "peripheral", headset: false },
    });
  });

  it("labels the `none` anchor as the suspension control and refuses location when denied", async () => {
    const plugin = mount();
    fireEvent.click(screen.getByRole("button", { name: "Set up T2" }));
    const anchors = screen.getByTestId("lab-anchor");
    fireEvent.click(within(anchors).getByRole("radio", { name: "None (control)" }));
    expect(anchors.textContent).toContain("Control: the phone may suspend.");
    fireEvent.click(screen.getByLabelText("Use GPS for this run"));
    expect(methods(plugin, "requestPermissions")).toEqual([{ permissions: ["location"] }]);
    await act(flush);
    expect((screen.getByLabelText("Use GPS for this run") as HTMLInputElement).checked).toBe(false);
    expect(screen.getByText(/Location is off for this app/)).toBeTruthy();
  });

  it("advises the microphone for full audio and shows config problems", async () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Set up T2" }));
    await act(flush);
    expect(screen.getByText("Full audio needs the microphone.")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Duration (seconds)"), { target: { value: "" } });
    expect(screen.getByTestId("lab-config-problem").textContent).toBe(
      "The duration must be between 1 second and 24 hours.",
    );
  });

  it("marks with the passenger/stop safety note", async () => {
    const plugin = mount();
    await emit("labProgress");
    expect(screen.getByTestId("lab-live").textContent).toContain("Marks are for passengers or at stops");
    fireEvent.click(screen.getByRole("button", { name: "Mark" }));
    const sheet = screen.getByTestId("lab-mark-sheet");
    expect(within(sheet).getByRole("note").textContent).toContain("Riders never touch the phone while moving");
    fireEvent.click(within(sheet).getByRole("button", { name: "Tunnel" }));
    await act(flush);
    expect(plugin.sent.find((c) => c.type === "labMark")).toEqual({ type: "labMark", label: "tunnel" });
    expect(screen.queryByTestId("lab-mark-sheet")).toBeNull();
  });

  it("deletes a stored run after confirmation", async () => {
    const plugin = mount();
    await act(flush);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: "Delete run r-20261023-1001-cd34" }));
    await act(flush);
    expect(plugin.sent.find((c) => c.type === "labDeleteRun")).toEqual({
      type: "labDeleteRun",
      runId: "r-20261023-1001-cd34",
    });
    expect(screen.queryByRole("button", { name: "Delete run r-20261023-1001-cd34" })).toBeNull();
  });

  it("shows the procedure and renders in Spanish", async () => {
    mount();
    fireEvent.click(screen.getAllByRole("button", { name: "How to run the field test" })[0] as HTMLElement);
    expect(screen.getByTestId("lab-procedure").textContent).toContain("Riders never operate the phone while moving");
    await act(async () => applyLanguage("es"));
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Prueba de campo");
    expect(screen.getByTestId("lab-procedure").textContent).toContain("Quien conduce nunca manipula el teléfono");
  });
});
