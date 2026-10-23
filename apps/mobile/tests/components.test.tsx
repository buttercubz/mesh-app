// @vitest-environment jsdom
// Shell components (WP-18, DESIGN §13.1): useKeepScreenOn on mount/unmount, the App
// root rendering registered screens and the shell placeholder, the API-mismatch banner,
// ScreenLayout back navigation and back handlers, settings side effects.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "../src/app/App";
import type { ScreenProps } from "../src/app/router";
import { handleBackPress, registerScreen, registeredScreenIds, unregisterScreen, useRouter } from "../src/app/router";
import { installSettingsEffects } from "../src/app/settingsEffects";
import { ScreenLayout, useBackHandler, useKeepScreenOn } from "../src/components";
import { applyLanguage } from "../src/i18n/index";
import { useEngineStore } from "../src/store/useEngineStore";
import { useSettingsStore } from "../src/store/useSettingsStore";
import { snapshotFixture } from "./fixtures";

afterEach(() => {
  cleanup();
  useRouter.getState().reset();
  for (const id of registeredScreenIds()) unregisterScreen(id);
  useEngineStore.getState().reset();
  applyLanguage("en");
});

function keepScreenOnClient() {
  const calls: boolean[] = [];
  return { calls, client: { setKeepScreenOn: (on: boolean) => (calls.push(on), Promise.resolve()) } };
}

function manualForeground() {
  const listeners = new Set<() => void>();
  return {
    fire: () => listeners.forEach((l) => l()),
    source: {
      onForeground(l: () => void) {
        listeners.add(l);
        return () => listeners.delete(l);
      },
    },
    size: () => listeners.size,
  };
}

describe("useKeepScreenOn", () => {
  function Probe({ on, client, source }: { on: boolean; client: ReturnType<typeof keepScreenOnClient>["client"]; source: ReturnType<typeof manualForeground>["source"] }) {
    useKeepScreenOn(on, client, source);
    return null;
  }

  it("calls setKeepScreenOn(true) on mount and (false) on unmount", () => {
    const { calls, client } = keepScreenOnClient();
    const fg = manualForeground();
    const view = render(<Probe on client={client} source={fg.source} />);
    expect(calls).toEqual([true]);
    view.unmount();
    expect(calls).toEqual([true, false]);
    expect(fg.size()).toBe(0);
  });

  it("re-applies after a foreground return (bridges reset it on background)", () => {
    const { calls, client } = keepScreenOnClient();
    const fg = manualForeground();
    render(<Probe on client={client} source={fg.source} />);
    fg.fire();
    expect(calls).toEqual([true, true]);
  });

  it("does nothing while off, and releases when switched off", () => {
    const { calls, client } = keepScreenOnClient();
    const fg = manualForeground();
    const view = render(<Probe on={false} client={client} source={fg.source} />);
    expect(calls).toEqual([]);
    view.rerender(<Probe on client={client} source={fg.source} />);
    view.rerender(<Probe on={false} client={client} source={fg.source} />);
    expect(calls).toEqual([true, false]);
  });
});

describe("App shell", () => {
  it("renders the shell placeholder for an unregistered home", () => {
    render(<App />);
    expect(screen.getByTestId("shell-screen")).toBeTruthy();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("convLink Voice");
    expect(document.querySelector('[data-screen="home"]')).not.toBeNull();
  });

  it("renders the registered screen with its params, and translates on language change", () => {
    function Lobby({ params }: ScreenProps) {
      return <p data-testid="lobby">{String(params["groupId"])}</p>;
    }
    registerScreen("lobby", Lobby);
    render(<App />);
    act(() => useRouter.getState().push("lobby", { groupId: "g1" }));
    expect(screen.getByTestId("lobby").textContent).toBe("g1");
    act(() => applyLanguage("es"));
    expect(screen.getByRole("main").getAttribute("aria-label")).toBe("Contenido principal");
  });

  it("says when a pushed screen does not exist in this build", () => {
    render(<App />);
    act(() => useRouter.getState().push("join", { url: "convlinkvoice://join#x" }));
    expect(screen.getByRole("status").textContent).toMatch(/not available/);
  });

  it("shows the update banner on an API major mismatch, and the crash banner", () => {
    render(<App />);
    act(() =>
      useEngineStore.getState().setInfo({ platform: "ios", engine: "full", apiVersion: "2.0", protocolVersion: 1, appVersion: "1" }),
    );
    expect(screen.getByText("Update needed")).toBeTruthy();
    expect(document.querySelector('[data-api-compat="mismatch"]')).not.toBeNull();
    act(() => {
      useEngineStore.getState().setInfo({ platform: "ios", engine: "full", apiVersion: "1.1", protocolVersion: 1, appVersion: "1" });
      useEngineStore.getState().applySnapshot(snapshotFixture({ engine: "crashed" }));
    });
    expect(screen.queryByText("Update needed")).toBeNull();
    expect(screen.getByText("The intercom engine stopped")).toBeTruthy();
  });
});

describe("ScreenLayout and back handling", () => {
  it("shows a back button only when there is somewhere to go", () => {
    function Page() {
      return <ScreenLayout title="Members">body</ScreenLayout>;
    }
    registerScreen("members", Page);
    registerScreen("home", Page);
    render(<App />);
    expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
    act(() => useRouter.getState().push("members"));
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(useRouter.getState().stack).toHaveLength(1);
  });

  it("docks toasts above a pinned footer and releases the dock on unmount", () => {
    const root = document.documentElement;
    const { unmount } = render(
      <ScreenLayout title="Ride" footer={<button type="button">PTT</button>}>
        body
      </ScreenLayout>,
    );
    // jsdom has no layout: the footer measures 0 px, so only the gap remains.
    expect(root.style.getPropertyValue("--app-toast-bottom")).toBe("8px");
    unmount();
    expect(root.style.getPropertyValue("--app-toast-bottom")).toBe("");
    render(<ScreenLayout title="Members">body</ScreenLayout>);
    expect(root.style.getPropertyValue("--app-toast-bottom")).toBe("");
  });

  it("useBackHandler consumes the back press while mounted", () => {
    const handler = vi.fn(() => true);
    function Sheet() {
      useBackHandler(handler);
      return null;
    }
    useRouter.getState().push("members");
    const view = render(<Sheet />);
    expect(handleBackPress()).toBe(true);
    expect(handler).toHaveBeenCalledTimes(1);
    expect(useRouter.getState().stack).toHaveLength(2);
    view.unmount();
    expect(handleBackPress()).toBe(true);
    expect(useRouter.getState().stack).toHaveLength(1);
  });
});

describe("settings effects", () => {
  beforeEach(() => useSettingsStore.getState().reset());

  it("applies language and theme, and re-sends prompts while running", async () => {
    const syncPrompts = vi.fn(() => Promise.resolve({ ok: true } as const));
    const stop = installSettingsEffects({ syncPrompts });
    act(() => useSettingsStore.getState().update({ language: "es", theme: "light" }));
    expect(document.documentElement.lang).toBe("es");
    expect(document.documentElement.dataset["theme"]).toBe("light");
    expect(syncPrompts).not.toHaveBeenCalled();
    useEngineStore.getState().setStatus("running");
    act(() => useSettingsStore.getState().update({ spokenPrompts: false }));
    expect(syncPrompts).toHaveBeenCalledTimes(1);
    act(() => useSettingsStore.getState().update({ haptics: true }));
    expect(syncPrompts).toHaveBeenCalledTimes(1);
    stop();
  });
});
