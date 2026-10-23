// Loads the golden JSON fixtures shared with the Rust tests (crates/mesh-api/fixtures/).
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const dir = new URL("../../../crates/mesh-api/fixtures/", import.meta.url);

function load<T>(file: string): T {
  return JSON.parse(readFileSync(fileURLToPath(new URL(file, dir)), "utf8")) as T;
}

export interface CommandFixture {
  name: string;
  requiresGesture: boolean;
  command: Record<string, unknown> & { type: string };
}

export interface EventFixture {
  name: string;
  event: Record<string, unknown> & { type: string };
}

export interface ResultFixture {
  name: string;
  for: string;
  result: Record<string, unknown> & { ok: boolean };
}

export interface InvalidFixture {
  name: string;
  json: unknown;
}

export const commandFixtures = load<CommandFixture[]>("commands.json");
export const eventFixtures = load<EventFixture[]>("events.json");
export const resultFixtures = load<ResultFixture[]>("results.json");
export const invalidFixtures = load<{
  commands: InvalidFixture[];
  events: InvalidFixture[];
  results: InvalidFixture[];
}>("invalid.json");
