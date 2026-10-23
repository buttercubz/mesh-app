// Package shape (WP-17 acceptance): TS-only plugin, C6 surface compatible with the
// engine-api client, lazy web implementation.
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type { EnginePlugin } from "@meshapp/engine-api";
import { describe, expect, expectTypeOf, it } from "vitest";

import type { MeshIntercomPlugin } from "../src/definitions.js";
import { INTERCOM_PERMISSIONS, MESH_INTERCOM_JS_NAME } from "../src/definitions.js";
import { MeshIntercom, loadWebImplementation } from "../src/index.js";

const pkgDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const pkg = JSON.parse(readFileSync(join(pkgDir, "package.json"), "utf8")) as Record<string, unknown>;

describe("package", () => {
  it("has no Capacitor native plugin metadata and no native directories", () => {
    expect(pkg).not.toHaveProperty("capacitor");
    for (const dir of ["ios", "android", "Package.swift"]) {
      expect(existsSync(join(pkgDir, dir))).toBe(false);
    }
    expect(Object.keys(pkg["peerDependencies"] as object)).toEqual(["@capacitor/core"]);
  });

  it("exports the plugin types from dist", () => {
    expect(pkg).toMatchObject({ types: "./dist/index.d.ts", main: "./dist/index.js" });
  });

  it("registers MeshIntercom with a lazy web implementation", async () => {
    expect(MESH_INTERCOM_JS_NAME).toBe("MeshIntercom");
    // Outside a native shell the proxy resolves the web implementation on first call.
    const info = await MeshIntercom.getInfo();
    expect(info.platform).toBe("web");
  });

  it("creates one web implementation even for concurrent first calls", async () => {
    const [a, b] = await Promise.all([loadWebImplementation(), loadWebImplementation()]);
    expect(a).toBe(b);
    expect(await loadWebImplementation()).toBe(a);
  });
});

describe("C6 definitions", () => {
  it("MeshIntercomPlugin satisfies the engine-api client's plugin interface", () => {
    expectTypeOf<MeshIntercomPlugin>().toExtend<EnginePlugin>();
  });

  it("ptt carries the UI timestamp and setKeepScreenOn a flag", () => {
    expectTypeOf<MeshIntercomPlugin["ptt"]>().parameter(0).toEqualTypeOf<{ down: boolean; tUiMs?: number }>();
    expectTypeOf<MeshIntercomPlugin["setKeepScreenOn"]>().parameter(0).toEqualTypeOf<{ on: boolean }>();
    expectTypeOf<MeshIntercomPlugin["gestureCommand"]>()
      .parameter(0)
      .toEqualTypeOf<{ command: string }>();
  });

  it("setNotificationCatalog takes a title and key → text templates (WP-19 change request)", () => {
    expectTypeOf<MeshIntercomPlugin["setNotificationCatalog"]>()
      .parameter(0)
      .toEqualTypeOf<{ title: string; templates: Record<string, string> }>();
    expectTypeOf<ReturnType<MeshIntercomPlugin["setNotificationCatalog"]>>().toEqualTypeOf<Promise<void>>();
  });

  it("lists every intercom permission once", () => {
    expect(new Set(INTERCOM_PERMISSIONS).size).toBe(6);
  });
});
