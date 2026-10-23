// @vitest-environment jsdom
// Ways into the Field test: the feature registers `fieldTest`, the Settings row pushes
// it, and the Phase 0 Home/Settings stand-ins exist only while no other feature
// registered those screens (WP-22 replaces them).
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ScreenProps } from "../../app/router";

afterEach(() => {
  cleanup();
  vi.resetModules();
});

describe("Field test entry", () => {
  it("registers the Field test and the Phase 0 stand-ins, reachable Home → Settings → Field test", async () => {
    const router = await import("../../app/router");
    const entry = await import("./entry");
    const { FieldTestScreen } = await import("./FieldTestScreen");
    await import("./index");
    expect(router.getScreen(router.SCREEN.fieldTest)?.component).toBe(FieldTestScreen);
    expect(router.getScreen(router.SCREEN.home)?.component).toBe(entry.Phase0HomeScreen);
    expect(router.getScreen(router.SCREEN.settings)?.component).toBe(entry.Phase0SettingsScreen);

    const Home = entry.Phase0HomeScreen;
    render(<Home params={{}} />);
    expect(screen.getByTestId("shell-screen")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(router.currentRoute().id).toBe(router.SCREEN.settings);
    cleanup();

    const Settings = entry.Phase0SettingsScreen;
    render(<Settings params={{}} />);
    fireEvent.click(screen.getByRole("button", { name: /Field test/ }));
    expect(router.currentRoute().id).toBe(router.SCREEN.fieldTest);
  });

  it("never replaces a Home or Settings screen another feature registered", async () => {
    const router = await import("../../app/router");
    const Home = (_p: ScreenProps) => null;
    const Settings = (_p: ScreenProps) => null;
    router.registerScreen(router.SCREEN.home, Home);
    router.registerScreen(router.SCREEN.settings, Settings);
    await import("./index");
    expect(router.getScreen(router.SCREEN.home)?.component).toBe(Home);
    expect(router.getScreen(router.SCREEN.settings)?.component).toBe(Settings);
    expect(router.getScreen(router.SCREEN.fieldTest)).toBeDefined();
  });
});
