// Static contract: capacitor.config.ts and package wiring (WP-18, DESIGN §2.4, §11.3).
import { describe, expect, it } from "vitest";

import config from "../capacitor.config.ts";
import { read } from "./contracts/files.mjs";

describe("capacitor.config", () => {
  it("uses the working identifiers (Appendix E) and serves the local bundle", () => {
    expect(config.appId).toBe("com.convlink.voice");
    expect(config.appName).toBe("convLink Voice");
    expect(config.webDir).toBe("dist");
    expect(config.server?.url).toBeUndefined();
    expect(config.server?.cleartext).not.toBe(true);
    expect(config.android?.allowMixedContent).toBe(false);
    expect(config.android?.webContentsDebuggingEnabled).toBe(false);
  });

  it("starts edge-to-edge with CSS insets, viewport-fit cover and light bar icons (dark default theme)", () => {
    expect(config.plugins?.SystemBars).toEqual({ insetsHandling: "css", initialViewportFitValueHint: "cover", style: "DARK" });
    expect(config.ios?.contentInset).toBe("never");
  });

  it("is SPM only and never registers the TS-only plugin as a native package", () => {
    expect(config.ios?.packageClassList).toBeUndefined();
    const spm = read("ios/App/CapApp-SPM/Package.swift");
    expect(spm).toContain("capacitor-swift-pm");
    expect(spm).not.toMatch(/capacitor-intercom/);
    expect(read("android/capacitor.settings.gradle")).not.toMatch(/capacitor-intercom/);
  });

  it("pins Capacitor 8.5 and pnpm-only tooling", () => {
    const pkg = JSON.parse(read("package.json"));
    for (const dep of ["@capacitor/core", "@capacitor/ios", "@capacitor/android"]) expect(pkg.dependencies[dep]).toMatch(/^~8\.5\./);
    expect(pkg.devDependencies["@capacitor/cli"]).toMatch(/^~8\.5\./);
    expect(pkg.dependencies.react).toMatch(/^~19\.2\./);
    expect(JSON.stringify(pkg.scripts)).not.toMatch(/\bnpx\b|\bnpm\b|\byarn\b/);
  });
});
