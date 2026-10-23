// Static contracts: iOS Info.plist, entitlements, privacy manifest and localised usage
// strings (WP-18, DESIGN §10.1.5, §13.1). Physical behaviour is validated on devices.
import { describe, expect, it } from "vitest";

import { exists, isValidWifiAwareServiceName, readPlist, readStrings } from "./contracts/files.mjs";

const info = readPlist("ios/App/App/Info.plist");
const entitlements = readPlist("ios/App/App/App.entitlements");
const privacy = readPlist("ios/App/App/PrivacyInfo.xcprivacy");

const USAGE = {
  NSMicrophoneUsageDescription: "Your voice is sent to the riders in your group.",
  NSBluetoothAlwaysUsageDescription: "Connects directly to nearby riders' phones to carry the intercom, without internet.",
  NSCameraUsageDescription: "Scan your group's invite QR code.",
  NSLocationWhenInUseUsageDescription:
    "Shows how far each rider is and keeps the intercom running while the screen is locked.",
};

describe("Info.plist", () => {
  it("declares exactly the four background modes (no voip, no push-to-talk)", () => {
    expect([...info.UIBackgroundModes].sort()).toEqual(["audio", "bluetooth-central", "bluetooth-peripheral", "location"]);
    expect(info.UIBackgroundModes).not.toContain("voip");
    expect(info.UIBackgroundModes).not.toContain("push-to-talk");
  });

  it("has the DESIGN usage descriptions and no Always-location request", () => {
    for (const [key, text] of Object.entries(USAGE)) expect(info[key], key).toBe(text);
    expect(info.NSLocationAlwaysAndWhenInUseUsageDescription).toBeUndefined();
    expect(info.NSLocationAlwaysUsageDescription).toBeUndefined();
  });

  it("registers only the convlinkvoice URL scheme", () => {
    const schemes = info.CFBundleURLTypes.flatMap((t) => t.CFBundleURLSchemes);
    expect(schemes).toEqual(["convlinkvoice"]);
  });

  it("uses a single storyboard scene with the app's SceneDelegate (iOS 27 requirement)", () => {
    const manifest = info.UIApplicationSceneManifest;
    expect(manifest.UIApplicationSupportsMultipleScenes).toBe(false);
    const roles = manifest.UISceneConfigurations.UIWindowSceneSessionRoleApplication;
    expect(roles).toHaveLength(1);
    expect(roles[0]).toEqual({
      UISceneConfigurationName: "Default Configuration",
      UISceneDelegateClassName: "$(PRODUCT_MODULE_NAME).SceneDelegate",
      UISceneStoryboardFile: "Main",
    });
    expect(info.UIMainStoryboardFile).toBeUndefined();
    expect(info.UILaunchStoryboardName).toBe("LaunchScreen");
  });

  it("requires arm64 + BLE + microphone, supports Live Activities, declares encryption", () => {
    expect([...info.UIRequiredDeviceCapabilities].sort()).toEqual(["arm64", "bluetooth-le", "microphone"]);
    expect(info.NSSupportsLiveActivities).toBe(true);
    expect(info.ITSAppUsesNonExemptEncryption).toBe(true);
    expect(info.CFBundleIdentifier).toBe("$(PRODUCT_BUNDLE_IDENTIFIER)");
    expect(info.CFBundleDisplayName).toBe("convLink Voice");
    expect(info.CFBundleLocalizations).toEqual(["en", "es"]);
  });

  it("applies the WiFiAwareServices rules (Phase 4 key; none before the entitlement)", () => {
    expect(isValidWifiAwareServiceName("_cvic._udp")).toBe(true);
    for (const bad of ["cvic._udp", "_cvic", "_CVIC._udp", "_a-very-long-service-name._udp", "_c--v._udp", "_-cv._tcp", "_123._udp"]) {
      expect(isValidWifiAwareServiceName(bad), bad).toBe(false);
    }
    const services = info.WiFiAwareServices;
    const entitled = Array.isArray(entitlements["com.apple.developer.wifi-aware"]);
    expect(services !== undefined, "WiFiAwareServices and the wifi-aware entitlement go together").toBe(entitled);
    if (services !== undefined) {
      for (const [name, spec] of Object.entries(services)) {
        expect(isValidWifiAwareServiceName(name), name).toBe(true);
        const roles = Object.keys(spec);
        expect(roles.length, `${name} needs Publishable and/or Subscribable`).toBeGreaterThan(0);
        for (const role of roles) expect(["Publishable", "Subscribable"]).toContain(role);
      }
    } else {
      expect(info.NSLocalNetworkUsageDescription).toBeUndefined();
    }
  });
});

describe("InfoPlist.strings", () => {
  for (const lang of ["en", "es"]) {
    it(`${lang} localises every usage description`, () => {
      expect(exists(`ios/App/App/${lang}.lproj/InfoPlist.strings`)).toBe(true);
      const strings = readStrings(`ios/App/App/${lang}.lproj/InfoPlist.strings`);
      for (const key of Object.keys(USAGE)) expect(strings[key]?.length ?? 0, `${lang} ${key}`).toBeGreaterThan(10);
      if (lang === "en") for (const [key, text] of Object.entries(USAGE)) expect(strings[key]).toBe(text);
    });
  }
});

describe("entitlements", () => {
  it("has time-sensitive notifications and nothing for voip/push-to-talk/push", () => {
    expect(entitlements["com.apple.developer.usernotifications.time-sensitive"]).toBe(true);
    for (const key of Object.keys(entitlements)) {
      expect(key).not.toMatch(/push-to-talk|voip|aps-environment/);
    }
  });

  it("debug entitlements request nothing a personal (free) team cannot provision", () => {
    const debug = readPlist("ios/App/App/App-Debug.entitlements");
    expect(debug["com.apple.developer.usernotifications.time-sensitive"]).toBeUndefined();
    for (const key of Object.keys(debug)) {
      expect(key).not.toMatch(/push-to-talk|voip|aps-environment|time-sensitive|wifi-aware/);
    }
  });
});

describe("PrivacyInfo.xcprivacy", () => {
  it("declares no tracking, no collected data and the two required-reason APIs", () => {
    expect(privacy.NSPrivacyTracking).toBe(false);
    expect(privacy.NSPrivacyTrackingDomains).toEqual([]);
    expect(privacy.NSPrivacyCollectedDataTypes).toEqual([]);
    const reasons = Object.fromEntries(
      privacy.NSPrivacyAccessedAPITypes.map((t) => [t.NSPrivacyAccessedAPIType, t.NSPrivacyAccessedAPITypeReasons]),
    );
    expect(reasons).toEqual({
      NSPrivacyAccessedAPICategoryUserDefaults: ["CA92.1"],
      NSPrivacyAccessedAPICategorySystemBootTime: ["35F9.1"],
    });
  });
});
