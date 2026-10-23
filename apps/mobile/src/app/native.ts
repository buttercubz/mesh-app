// Native app integration through @capacitor/app (WP-18): Android hardware back button
// (convLink useBackButton pattern) and invite deep links. Web builds skip it.
import { App } from "@capacitor/app";
import type { PluginListenerHandle } from "@capacitor/core";
import { Capacitor } from "@capacitor/core";

import { handleIncomingUrl } from "./deepLinks";
import { handleBackPress } from "./router";

/** Installs the listeners once; returns the remover. */
export function installNativeHandlers(): () => void {
  if (!Capacitor.isNativePlatform()) return () => {};
  const handles: Promise<PluginListenerHandle>[] = [
    App.addListener("backButton", () => {
      // Never exit: backgrounding keeps a running intercom alive (§10.2.3).
      if (!handleBackPress()) void App.minimizeApp();
    }),
    App.addListener("appUrlOpen", ({ url }) => {
      handleIncomingUrl(url);
    }),
  ];
  App.getLaunchUrl().then(
    (launch) => {
      if (launch?.url) handleIncomingUrl(launch.url);
    },
    (e: unknown) => {
      // No launch URL support on this platform version: links still arrive via appUrlOpen.
      console.warn("[app] getLaunchUrl failed", e);
    },
  );
  return () => {
    for (const h of handles) void h.then((handle) => handle.remove());
  };
}

/** The platform the app runs on, for platform classes (`.android` disables blur). */
export function runtimePlatform(): "ios" | "android" | "web" {
  const p = Capacitor.getPlatform();
  return p === "ios" || p === "android" ? p : "web";
}
