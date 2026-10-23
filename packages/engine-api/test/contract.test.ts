// The TS guards accept every shared fixture, reject every invalid one, and agree with
// the Rust side on gesture gating and result kinds (WP-03 acceptance).
import { describe, expect, it } from "vitest";
import {
  API_VERSION,
  API_VERSION_MAJOR,
  API_VERSION_MINOR,
  COMMAND_RESULT_KINDS,
  ContractError,
  GESTURE_COMMANDS,
  GESTURE_COMMAND_TYPES,
  LAB_SUBSET_COMMAND_TYPES,
  LAB_SUBSET_EVENT_TYPES,
  RESULT_KINDS,
  UI_COMMAND_TYPES,
  UI_EVENT_TYPES,
  explainUiEvent,
  isCommandResult,
  isSnapshot,
  isUiCommand,
  isUiEvent,
  parseCommandResult,
  parseSnapshot,
  parseUiCommand,
  parseUiEvent,
  requiresGesture,
} from "../src/index.js";
import type { CommandType, ResultKind, UiCommand } from "../src/index.js";
import { resultPayloads } from "../src/schema/result.js";
import { commandFixtures, eventFixtures, invalidFixtures, resultFixtures } from "./fixtures.js";

describe("commands", () => {
  it.each(commandFixtures.map((f) => [f.name, f] as const))("%s parses", (_name, fixture) => {
    expect(isUiCommand(fixture.command)).toBe(true);
    const parsed = parseUiCommand(JSON.stringify(fixture.command));
    expect(parsed).toEqual(fixture.command);
    expect(requiresGesture(parsed)).toBe(fixture.requiresGesture);
  });

  it("every command type has a fixture", () => {
    const seen = new Set(commandFixtures.map((f) => f.command.type));
    expect(UI_COMMAND_TYPES.filter((t) => !seen.has(t))).toEqual([]);
  });

  it.each(invalidFixtures.commands.map((f) => [f.name, f.json] as const))(
    "rejects %s",
    (_name, json) => {
      expect(isUiCommand(json)).toBe(false);
      expect(() => parseUiCommand(json)).toThrow(ContractError);
    },
  );
});

describe("events", () => {
  it.each(eventFixtures.map((f) => [f.name, f] as const))("%s parses", (_name, fixture) => {
    expect(explainUiEvent(fixture.event)).toBeUndefined();
    expect(isUiEvent(fixture.event)).toBe(true);
    const parsed = parseUiEvent(JSON.stringify(fixture.event));
    expect(parsed).toEqual(fixture.event);
    if (parsed.type === "snapshot") {
      expect(isSnapshot(parsed.s)).toBe(true);
      expect(parseSnapshot(JSON.stringify(parsed.s))).toEqual(parsed.s);
    }
  });

  it("every event type has a fixture", () => {
    const seen = new Set(eventFixtures.map((f) => f.event.type));
    expect(UI_EVENT_TYPES.filter((t) => !seen.has(t))).toEqual([]);
  });

  it.each(invalidFixtures.events.map((f) => [f.name, f.json] as const))(
    "rejects %s",
    (_name, json) => {
      expect(isUiEvent(json)).toBe(false);
      expect(() => parseUiEvent(json)).toThrow(ContractError);
    },
  );

  it("reports malformed JSON as a contract error with the path of the problem", () => {
    expect(() => parseUiEvent("{not json")).toThrow(ContractError);
    expect(explainUiEvent({ type: "notice", key: "alone", params: { n: true } })).toMatch(
      /^\$\.params\.n: /,
    );
  });
});

describe("results", () => {
  it.each(resultFixtures.map((f) => [f.name, f] as const))("%s parses", (_name, fixture) => {
    expect(isCommandResult(fixture.result)).toBe(true);
    const parsed = parseCommandResult(JSON.stringify(fixture.result));
    expect(parsed).toEqual(fixture.result);
    if (parsed.ok) {
      const kind = COMMAND_RESULT_KINDS[fixture.for as CommandType];
      expect(resultPayloads[kind].check(parsed, "$")).toBeUndefined();
    }
  });

  it("every result kind has a fixture", () => {
    const kinds = new Set<ResultKind>(
      resultFixtures
        .filter((f) => f.result.ok)
        .map((f) => COMMAND_RESULT_KINDS[f.for as CommandType]),
    );
    expect(RESULT_KINDS.filter((k) => !kinds.has(k))).toEqual([]);
  });

  it.each(invalidFixtures.results.map((f) => [f.name, f.json] as const))(
    "rejects %s",
    (_name, json) => {
      expect(isCommandResult(json)).toBe(false);
    },
  );
});

describe("constants", () => {
  it("API version 1.1", () => {
    expect(API_VERSION).toBe("1.1");
    expect(`${API_VERSION_MAJOR}.${API_VERSION_MINOR}`).toBe(API_VERSION);
  });

  it("gesture list is goLive, resumeMic, labStart (audio ≠ off), audioCheck start", () => {
    expect(GESTURE_COMMAND_TYPES).toEqual(["goLive", "resumeMic", "labStart", "audioCheck"]);
    expect(GESTURE_COMMANDS.map((r) => r.when)).toEqual([
      null,
      null,
      { path: ["params", "audio"], op: "notEquals", value: "off" },
      { path: ["op"], op: "equals", value: "start" },
    ]);
    const lab = (audio: "off" | "renderOnly" | "full"): UiCommand => ({
      type: "labStart",
      bench: "T2",
      params: { audio, anchor: "audio", gps: false },
    });
    expect(requiresGesture(lab("off"))).toBe(false);
    expect(requiresGesture(lab("renderOnly"))).toBe(true);
    expect(requiresGesture(lab("full"))).toBe(true);
    expect(requiresGesture({ type: "audioCheck", op: "stop" })).toBe(false);
  });

  it("the Lab subset is part of the full contract", () => {
    expect(LAB_SUBSET_COMMAND_TYPES.every((t) => UI_COMMAND_TYPES.includes(t))).toBe(true);
    expect(LAB_SUBSET_EVENT_TYPES.every((t) => UI_EVENT_TYPES.includes(t))).toBe(true);
    expect(LAB_SUBSET_COMMAND_TYPES).toContain("labStart");
    expect(LAB_SUBSET_EVENT_TYPES).toEqual(
      expect.arrayContaining(["labProgress", "labFinished", "snapshot", "error"]),
    );
  });
});
