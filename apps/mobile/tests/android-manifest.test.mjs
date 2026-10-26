// Static contracts: Android app manifest, backup rules, Gradle wiring, MainActivity and
// the plugin stub (WP-18, DESIGN §2.4, §10.2.4, §13.1). The engine library manifest
// (native/android, WP-16) is checked too once that file exists.
import { existsSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

import { TOOLS_NS, androidAttr, elements, read, readXml, repoRoot } from "./contracts/files.mjs";

const manifest = readXml("android/app/src/main/AndroidManifest.xml");
const root = manifest.documentElement;

const permissions = (doc) =>
  elements(doc.documentElement, "uses-permission").map((el) => ({
    name: androidAttr(el, "name"),
    flags: androidAttr(el, "usesPermissionFlags"),
    removed: el.getAttributeNS(TOOLS_NS, "node") === "remove" || el.getAttribute("tools:node") === "remove",
  }));

describe("app AndroidManifest.xml", () => {
  const perms = permissions(manifest);
  const declared = perms.filter((p) => !p.removed).map((p) => p.name);

  it("never declares ACCESS_LOCAL_NETWORK at target 36, and strips it from libraries", () => {
    expect(declared).not.toContain("android.permission.ACCESS_LOCAL_NETWORK");
    expect(perms).toContainEqual({ name: "android.permission.ACCESS_LOCAL_NETWORK", flags: null, removed: true });
  });

  it("declares no telecom permissions and no INTERNET (offline app)", () => {
    for (const p of ["android.permission.MANAGE_OWN_CALLS", "android.permission.FOREGROUND_SERVICE_PHONE_CALL", "android.permission.INTERNET"]) {
      expect(declared).not.toContain(p);
    }
  });

  it("MainActivity is singleTask, exported, and opens convlinkvoice://join", () => {
    const [activity] = elements(root, "activity");
    expect(androidAttr(activity, "name")).toBe(".MainActivity");
    expect(androidAttr(activity, "launchMode")).toBe("singleTask");
    expect(androidAttr(activity, "exported")).toBe("true");
    const filters = elements(activity, "intent-filter");
    const view = filters.find((f) => elements(f, "action").some((a) => androidAttr(a, "name") === "android.intent.action.VIEW"));
    expect(view).toBeDefined();
    const categories = elements(view, "category").map((c) => androidAttr(c, "name")).sort();
    expect(categories).toEqual(["android.intent.category.BROWSABLE", "android.intent.category.DEFAULT"]);
    const data = elements(view, "data").map((d) => [androidAttr(d, "scheme"), androidAttr(d, "host")]);
    expect(data).toEqual([["convlinkvoice", "join"]]);
  });

  it("backup and device transfer exclude the engine secure store", () => {
    const [app] = elements(root, "application");
    expect(androidAttr(app, "dataExtractionRules")).toBe("@xml/data_extraction_rules");
    expect(androidAttr(app, "fullBackupContent")).toBe("@xml/backup_rules");
    const rules = readXml("android/app/src/main/res/xml/data_extraction_rules.xml");
    for (const section of ["cloud-backup", "device-transfer"]) {
      const [node] = elements(rules.documentElement, section);
      const excluded = elements(node, "exclude").map((e) => `${e.getAttribute("domain")}:${e.getAttribute("path")}`);
      expect(excluded, section).toEqual(expect.arrayContaining(["sharedpref:.", "file:meshengine/", "database:."]));
    }
    const legacy = readXml("android/app/src/main/res/xml/backup_rules.xml");
    const legacyExcluded = elements(legacy.documentElement, "exclude").map((e) => `${e.getAttribute("domain")}:${e.getAttribute("path")}`);
    expect(legacyExcluded).toEqual(expect.arrayContaining(["sharedpref:.", "file:meshengine/", "database:."]));
  });

  it("shares only Lab exports through the FileProvider", () => {
    const paths = readXml("android/app/src/main/res/xml/file_paths.xml");
    const entries = Array.from(paths.documentElement.childNodes).filter((n) => n.nodeType === 1);
    expect(entries.map((e) => `${e.tagName}:${e.getAttribute("path")}`)).toEqual(["files-path:lab/", "cache-path:lab/"]);
  });
});

describe("Gradle wiring", () => {
  const settings = read("android/settings.gradle");
  const app = read("android/app/build.gradle");
  const top = read("android/build.gradle");

  it("includes the native/android build and substitutes com.convlink.voice:meshengine", () => {
    expect(settings).toContain("'../../../native/android'");
    expect(settings).toMatch(/includeBuild\(meshengineBuild\)/);
    expect(settings).toMatch(/substitute\(module\('com\.convlink\.voice:meshengine'\)\)\.using\(project\(':meshengine'\)\)/);
    expect(app).toMatch(/implementation 'com\.convlink\.voice:meshengine:[\d.]+'/);
    expect(settings).toContain("apply from: 'capacitor.settings.gradle'");
  });

  it("pins AGP 8.13.2, Kotlin 2.3, SDK 36 / min 31, Java 21", () => {
    expect(top).toContain("com.android.tools.build:gradle:8.13.2");
    expect(top).toMatch(/kotlin-gradle-plugin:2\.3\.\d+/);
    expect(top).toMatch(/minSdkVersion = 31/);
    expect(top).toMatch(/compileSdkVersion = 36/);
    expect(top).toMatch(/targetSdkVersion = 36/);
    expect(app).toContain("applicationId 'com.convlink.voice'");
    expect(app).toContain("jvmToolchain(21)");
    expect(app).toContain("abiFilters 'arm64-v8a', 'x86_64'");
    expect(read("android/gradle/wrapper/gradle-wrapper.properties")).toContain("gradle-8.14.3-");
  });

  it("MainActivity registers the plugin before super.onCreate", () => {
    const activity = read("android/app/src/main/java/com/convlink/voice/MainActivity.kt");
    const register = activity.indexOf("registerPlugin(MeshIntercomPlugin::class.java)");
    const superCall = activity.indexOf("super.onCreate(savedInstanceState)");
    expect(register).toBeGreaterThan(0);
    expect(superCall).toBeGreaterThan(register);
    expect(activity).toMatch(/class MainActivity : BridgeActivity\(\)/);
  });

  it("stays portrait on large screens and handles font-scale changes without recreating the activity", () => {
    const [app] = elements(root, "application");
    const props = elements(app, "property").map((p) => [androidAttr(p, "name"), androidAttr(p, "value")]);
    expect(props).toContainEqual(["android.window.PROPERTY_COMPAT_ALLOW_RESTRICTED_RESIZABILITY", "true"]);
    const [activity] = elements(root, "activity");
    expect(androidAttr(activity, "screenOrientation")).toBe("portrait");
    const changes = (androidAttr(activity, "configChanges") ?? "").split("|");
    expect(changes).toEqual(expect.arrayContaining(["orientation", "screenSize", "uiMode", "fontScale", "fontWeightAdjustment"]));
  });

  it("MainActivity clamps the WebView text zoom to 100-130 % and re-applies it on configuration changes", () => {
    const activity = read("android/app/src/main/java/com/convlink/voice/MainActivity.kt");
    expect(activity).toMatch(/const val MIN_TEXT_ZOOM = 100\b/);
    expect(activity).toMatch(/const val MAX_TEXT_ZOOM = 130\b/);
    expect(activity).toMatch(/coerceIn\(MIN_TEXT_ZOOM, MAX_TEXT_ZOOM\)/);
    expect(activity).toMatch(/override fun onConfigurationChanged\(newConfig: Configuration\)[\s\S]*applyTextZoom\(newConfig\)/);
    expect(activity.indexOf("applyTextZoom(resources.configuration)")).toBeGreaterThan(activity.indexOf("super.onCreate(savedInstanceState)"));
  });

  it("the Android plugin (WP-19, from the WP-18 stub) keeps the platform-only fallback and ★ gating", () => {
    const plugin = read("android/app/src/main/java/com/convlink/voice/plugin/MeshIntercomPlugin.kt");
    expect(plugin).toMatch(/@CapacitorPlugin\(\s*name = "MeshIntercom"/);
    expect(plugin).toContain('.put("engine", "platform-only")');
    expect(plugin).toContain('const val API_VERSION = "1.1"');
    expect(plugin).toContain("useGestureCommand");
  });
});

// The library manifest is WP-16's file; its §10.2.4 rules are checked as soon as it exists.
const libraryManifest = path.join(repoRoot, "native/android/meshengine/src/main/AndroidManifest.xml");

describe.runIf(existsSync(libraryManifest))("meshengine library manifest (WP-16, merged into the app)", () => {
  it("declares §10.2.4 permissions with neverForLocation and the FGS types, without ACCESS_LOCAL_NETWORK or telecom", () => {
    const lib = readXml("meshengine", libraryManifest);
    const perms = permissions(lib);
    const byName = Object.fromEntries(perms.map((p) => [p.name, p]));
    for (const p of ["RECORD_AUDIO", "FOREGROUND_SERVICE_MICROPHONE", "FOREGROUND_SERVICE_CONNECTED_DEVICE", "BLUETOOTH_ADVERTISE", "BLUETOOTH_CONNECT", "POST_NOTIFICATIONS"]) {
      expect(byName[`android.permission.${p}`], p).toBeDefined();
    }
    expect(byName["android.permission.BLUETOOTH_SCAN"]?.flags).toBe("neverForLocation");
    expect(byName["android.permission.NEARBY_WIFI_DEVICES"]?.flags).toBe("neverForLocation");
    for (const p of ["ACCESS_LOCAL_NETWORK", "MANAGE_OWN_CALLS", "FOREGROUND_SERVICE_PHONE_CALL"]) {
      expect(byName[`android.permission.${p}`], p).toBeUndefined();
    }
    const service = elements(lib.documentElement, "service").find((s) => androidAttr(s, "name")?.endsWith("MeshEngineService"));
    expect(androidAttr(service, "foregroundServiceType")).toBe("microphone|connectedDevice|location");
    expect(androidAttr(service, "exported")).toBe("false");
  });
});
