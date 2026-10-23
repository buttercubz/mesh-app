// Capacitor 8.5 configuration (WP-18, DESIGN §2.4, §11.3). SPM only (no CocoaPods).
// The MeshIntercom bridges are app-local (ios/App/App/Plugins, android/.../plugin), so
// `pnpm exec cap sync` never adds a package for @meshapp/capacitor-intercom.
// Static contract: tests/capacitor-config.test.mjs.
import type { CapacitorConfig } from "@capacitor/cli";

const config: CapacitorConfig = {
  appId: "com.convlink.voice",
  appName: "convLink Voice",
  webDir: "dist",
  // Offline app: the bundle is served from the device, never from a remote URL
  // (no `server.url`), so the CSP `default-src 'self'` holds on every platform.
  server: {
    androidScheme: "https",
  },
  ios: {
    contentInset: "never",
  },
  android: {
    allowMixedContent: false,
    webContentsDebuggingEnabled: false,
  },
  // `cap sync ios` writes CapApp-SPM/Package.swift with the deployment target as
  // `.iOS(.v26)`, which needs swift-tools 6.2; the CLI default (5.9) breaks package
  // resolution ("'v26' is unavailable") and with it every iOS build.
  experimental: {
    ios: {
      spm: {
        swiftToolsVersion: "6.2",
      },
    },
  },
};

export default config;
