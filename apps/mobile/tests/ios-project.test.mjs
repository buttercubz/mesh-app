// Static contracts: Xcode project, scene lifecycle sources, bridge registration and
// stub hand-offs (WP-18, DESIGN §2.4, §10.1.5). The native build itself is verified by
// WP-19 (it needs MeshCoreFFI.xcframework); `plutil -lint` checks project syntax.
import { describe, expect, it } from "vitest";

import { exists, read } from "./contracts/files.mjs";

const project = read("ios/App/App.xcodeproj/project.pbxproj");
const scheme = read("ios/App/App.xcodeproj/xcshareddata/xcschemes/App.xcscheme");
const sceneDelegate = read("ios/App/App/SceneDelegate.swift");
const appDelegate = read("ios/App/App/AppDelegate.swift");
const bridge = read("ios/App/App/BridgeViewController.swift");
const storyboard = read("ios/App/App/Base.lproj/Main.storyboard");

const section = (name) => project.split(`/* Begin ${name} section */`)[1]?.split(`/* End ${name} section */`)[0] ?? "";
const settingValues = (key) => [...project.matchAll(new RegExp(`\\b${key} = ([^;]+);`, "g"))].map((m) => m[1].replace(/"/g, ""));

function swiftBody(source, signature) {
  const at = source.indexOf(signature);
  expect(at, `missing ${signature}`).toBeGreaterThanOrEqual(0);
  const start = source.indexOf("{", at);
  let depth = 1;
  let end = start + 1;
  for (; end < source.length && depth > 0; end += 1) {
    if (source[end] === "{") depth += 1;
    if (source[end] === "}") depth -= 1;
  }
  return source.slice(start + 1, end - 1);
}

describe("Xcode project", () => {
  it("has the App and IntercomWidget targets, the widget embedded in the app", () => {
    const targets = section("PBXNativeTarget");
    expect(targets).toMatch(/\/\* App \*\/ = \{[\s\S]*?productType = "com\.apple\.product-type\.application";/);
    expect(targets).toMatch(/\/\* IntercomWidget \*\/ = \{[\s\S]*?productType = "com\.apple\.product-type\.app-extension";/);
    expect(project).toMatch(/IntercomWidget\.appex in Embed Foundation Extensions/);
    expect(section("PBXTargetDependency")).toContain("IntercomWidget");
  });

  it("references the in-repo local packages, never node_modules", () => {
    const refs = section("XCLocalSwiftPackageReference");
    expect(refs).toContain('relativePath = ../../../../native/ios/MeshPlatformKit;');
    expect(refs).toContain('relativePath = ../../../../native/ios/MeshEngineKit;');
    expect(refs).toContain('relativePath = "CapApp-SPM";');
    expect(refs).not.toMatch(/node_modules/);
    const products = section("XCSwiftPackageProductDependency");
    for (const p of ["MeshEngineKit", "MeshPlatform", "MeshActivityShared"]) expect(products).toContain(`productName = ${p};`);
    expect(products).toContain('productName = "CapApp-SPM";');
  });

  it("targets iOS 26.0, iPhone only, with the app identifiers", () => {
    expect(new Set(settingValues("IPHONEOS_DEPLOYMENT_TARGET"))).toEqual(new Set(["26.0"]));
    expect(new Set(settingValues("TARGETED_DEVICE_FAMILY"))).toEqual(new Set(["1"]));
    expect(new Set(settingValues("PRODUCT_BUNDLE_IDENTIFIER"))).toEqual(new Set(["com.convlink.voice", "com.convlink.voice.IntercomWidget"]));
    // Debug drops time-sensitive notifications so personal (free) teams can install it;
    // Release keeps the full entitlements (App/App-Debug.entitlements explains why).
    expect(settingValues("CODE_SIGN_ENTITLEMENTS")).toEqual(["App/App-Debug.entitlements", "App/App.entitlements"]);
    // The signing team is the developer's own choice in Xcode; only require one is set.
    for (const team of settingValues("DEVELOPMENT_TEAM")) expect(team).toMatch(/^[A-Z0-9]{10}$/);
  });

  it("builds the app-local sources and ships the privacy manifest and localisations", () => {
    const sources = section("PBXSourcesBuildPhase");
    for (const f of ["AppDelegate.swift", "SceneDelegate.swift", "BridgeViewController.swift", "MeshIntercomPlugin.swift", "IntercomWidgetBundle.swift", "IntercomLiveActivity.swift"]) {
      expect(sources, f).toContain(`${f} in Sources`);
    }
    const resources = section("PBXResourcesBuildPhase");
    expect(resources).toContain("PrivacyInfo.xcprivacy in Resources");
    expect(resources).toContain("InfoPlist.strings in Resources");
    expect(section("PBXVariantGroup")).toMatch(/\/\* InfoPlist\.strings \*\/ = \{[\s\S]*?\/\* en \*\/,[\s\S]*?\/\* es \*\/,/);
    const files = section("PBXFileReference");
    expect(files).toContain("path = en.lproj/InfoPlist.strings;");
    expect(files).toContain("path = es.lproj/InfoPlist.strings;");
    expect(project).toMatch(/knownRegions = \(\s*en,\s*es,\s*Base,\s*\);/);
  });

  it("has a shared App scheme", () => {
    expect(scheme).toContain('BlueprintName = "App"');
    expect(scheme).toContain('ReferencedContainer = "container:App.xcodeproj"');
  });
});

describe("scene lifecycle and bridge", () => {
  it("storyboard root is the app's BridgeViewController", () => {
    expect(storyboard).toMatch(/customClass="BridgeViewController" customModule="App" customModuleProvider="target"/);
    expect(storyboard).not.toContain('customClass="CAPBridgeViewController"');
  });

  it("BridgeViewController registers the app-local MeshIntercom plugin", () => {
    expect(bridge).toMatch(/class\s+BridgeViewController\s*:\s*CAPBridgeViewController/);
    expect(swiftBody(bridge, "override func capacitorDidLoad()")).toContain("registerPluginInstance(MeshIntercomPlugin())");
  });

  it("SceneDelegate uses the storyboard window and loads the bridge before forwarding links", () => {
    expect(sceneDelegate).toMatch(/class\s+SceneDelegate\s*:\s*UIResponder,\s*UIWindowSceneDelegate/);
    expect(sceneDelegate).not.toMatch(/window\s*=\s*UIWindow\s*\(/);
    expect(sceneDelegate).not.toMatch(/rootViewController\s*=/);
    const cold = swiftBody(sceneDelegate, "willConnectTo session");
    const load = cold.indexOf("loadViewIfNeeded()");
    expect(load).toBeGreaterThanOrEqual(0);
    expect(cold.indexOf("SceneDelegateProxy.shared.scene(scene, willConnectTo")).toBeGreaterThan(load);
    expect(swiftBody(sceneDelegate, "openURLContexts URLContexts")).toContain("SceneDelegateProxy.shared.scene(scene, openURLContexts: URLContexts)");
    expect(swiftBody(sceneDelegate, "continue userActivity")).toContain("SceneDelegateProxy.shared.scene(scene, continue: userActivity)");
  });

  it("AppDelegate is process-level only", () => {
    expect(appDelegate).toMatch(/@main\s+final\s+class\s+AppDelegate/);
    expect(appDelegate).not.toMatch(/var\s+window\s*:/);
    expect(appDelegate).not.toContain("UIApplicationMain");
  });
});

describe("stub hand-offs", () => {
  it("the iOS bridge (WP-19, replaced the stub) keeps its name and drives the real engine host", () => {
    const plugin = read("ios/App/App/Plugins/MeshIntercomPlugin.swift");
    expect(plugin).toContain("@objc(MeshIntercomPlugin)");
    expect(plugin).toContain('jsName = "MeshIntercom"');
    expect(plugin).toContain('static let apiVersion = "1.1"');
    for (const m of ["getStatus", "startEngine", "gestureCommand", "goLive", "resumeMic", "stopEngine", "send", "ptt", "setKeepScreenOn", "scanInviteQr", "shareFile", "openAppSettings", "requestBatteryExemption"]) {
      expect(plugin, m).toContain(`CAPPluginMethod(name: "${m}"`);
    }
    expect(plugin).toContain("EngineHost(config: .current()");
    // Still the fallback when the host cannot be created.
    expect(plugin).toContain('"engine": "platform-only"');
    expect(plugin).toContain("useGestureCommand");
  });

  it("the widget stub compiles against MeshActivityShared without redefining shared types (→ WP-23)", () => {
    expect(exists("ios/IntercomWidget/IntercomWidgetBundle.swift")).toBe(true);
    const activity = read("ios/IntercomWidget/IntercomLiveActivity.swift");
    expect(activity).toContain("import MeshActivityShared");
    expect(activity).toContain("ActivityConfiguration(for: IntercomAttributes.self)");
    expect(activity).not.toMatch(/struct\s+IntercomAttributes|ContentState\s*:/);
    expect(read("ios/IntercomWidget/Info.plist")).toContain("com.apple.widgetkit-extension");
  });
});
