// Test-only helpers of the WP-22 features (imported by *.test.tsx files, never by app
// code): render a screen against the recording fake plugin, feed the mesh-api fixture
// events through the app's single event router, and reset the stores.
import { readFileSync } from "node:fs";
import type { CommandResult, Snapshot, UiCommand, UiEvent } from "@meshapp/engine-api";
import { act, cleanup, render } from "@testing-library/react";
import type { ReactElement } from "react";
import { createElement } from "react";

import { useRouter } from "../../app/router";
import { createAppEngine, handleEngineEvent } from "../../engine/client";
import type { AppEngine } from "../../engine/client";
import { applyLanguage } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { useGroupsStore } from "../../store/useGroupsStore";
import { defaultSettings, useSettingsStore } from "../../store/useSettingsStore";
import { createFakePlugin, flush } from "../../../tests/helpers";
import type { FakePlugin } from "../../../tests/helpers";
import { repoFile } from "../lab/testing";
import { resetRideDefaults } from "../ride/defaults";
import { IntercomEngineContext } from "./engine";

const fixtures = <T>(file: string) =>
  JSON.parse(readFileSync(repoFile(`crates/mesh-api/fixtures/${file}`), "utf8")) as T;

const EVENTS = fixtures<{ name: string; event: UiEvent }[]>("events.json");
const RESULTS = fixtures<{ name: string; result: CommandResult }[]>("results.json");

/** A mesh-api fixture event by name (crates/mesh-api/fixtures/events.json). */
export function fixtureEvent(name: string): UiEvent {
  const found = EVENTS.find((e) => e.name === name);
  if (found === undefined) throw new Error(`event fixture ${name}`);
  return found.event;
}

/** A mesh-api fixture result by name (crates/mesh-api/fixtures/results.json). */
export function fixtureResult(name: string): CommandResult {
  const found = RESULTS.find((r) => r.name === name);
  if (found === undefined) throw new Error(`result fixture ${name}`);
  return found.result;
}

/** The snapshot carried by a snapshot fixture event. */
export function fixtureSnapshot(name: string): Snapshot {
  const e = fixtureEvent(name);
  if (e.type !== "snapshot") throw new Error(`${name} is not a snapshot`);
  return e.s;
}

/** Delivers an event through the app's event router (stores, toasts muted). */
export function emit(event: UiEvent): void {
  act(() => handleEngineEvent(event, { toast: () => {}, now: () => Date.now() }));
}

/** Puts a running engine with `snapshot` in the store. */
export function running(snapshot: Snapshot): void {
  act(() => {
    useEngineStore.getState().applySnapshot(snapshot);
    useEngineStore.getState().setStatus("running");
  });
}

export interface Mounted {
  readonly plugin: FakePlugin;
  readonly engine: AppEngine;
  readonly unmount: () => void;
}

/** Renders `ui` with an engine over a fresh fake plugin answering through `answer`. */
export function mountWith(ui: ReactElement, answer?: (cmd: UiCommand) => CommandResult): Mounted {
  const plugin = createFakePlugin();
  if (answer !== undefined) plugin.answer = answer;
  const engine = createAppEngine(plugin, {
    setStatus: (s) => useEngineStore.getState().setStatus(s),
    setInfo: () => {},
    warn: () => {},
  });
  const view = render(createElement(IntercomEngineContext, { value: engine }, ui));
  return { plugin, engine, unmount: view.unmount };
}

/** Lets pending plugin promises settle inside act(). */
export async function settle(): Promise<void> {
  await act(flush);
  await act(flush);
}

/** Arguments of every call of one plugin method. */
export const calls = (plugin: FakePlugin, method: string): unknown[] =>
  plugin.calls.filter((c) => c.method === method).map((c) => c.args);

/** Resets every store the features touch (afterEach). */
export function resetAll(): void {
  cleanup();
  useEngineStore.getState().reset();
  useSettingsStore.setState({ ...defaultSettings("en"), onboardingDone: true, displayName: "Ana" });
  useGroupsStore.setState({ groups: [], activeGroupId: null });
  useRouter.getState().reset();
  resetRideDefaults();
  applyLanguage("en");
}
