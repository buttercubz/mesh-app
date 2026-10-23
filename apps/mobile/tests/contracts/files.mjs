// Helpers for the static contract tests (DESIGN §13.1): read app files relative to
// apps/mobile, parse XML plists and .strings files without macOS tools.
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import plist from "plist";
import { DOMParser } from "@xmldom/xmldom";

export const appRoot = fileURLToPath(new URL("../../", import.meta.url));
export const repoRoot = path.resolve(appRoot, "../..");

export const appPath = (p) => path.join(appRoot, p);
export const read = (p) => readFileSync(appPath(p), "utf8");
export const exists = (p) => existsSync(appPath(p));

/** Parses an XML property list (Info.plist, entitlements, PrivacyInfo). */
export const readPlist = (p) => plist.parse(read(p));

/** Parses a `"key" = "value";` .strings file into an object. */
export function readStrings(p) {
  const text = read(p).replace(/\/\*[\s\S]*?\*\//g, "");
  const out = {};
  for (const m of text.matchAll(/"((?:[^"\\]|\\.)*)"\s*=\s*"((?:[^"\\]|\\.)*)"\s*;/g)) out[m[1]] = m[2];
  return out;
}

/** Parses an XML file (AndroidManifest, resources) into a DOM document. */
export function readXml(p, file = appPath(p)) {
  const errors = [];
  const doc = new DOMParser({ onError: (level, msg) => level !== "warning" && errors.push(msg) }).parseFromString(
    readFileSync(file, "utf8"),
    "text/xml",
  );
  if (errors.length) throw new Error(`${p}: ${errors.join("; ")}`);
  return doc;
}

export const ANDROID_NS = "http://schemas.android.com/apk/res/android";
export const TOOLS_NS = "http://schemas.android.com/tools";

/** `android:` attribute of an element. */
export const androidAttr = (el, name) => el.getAttributeNS(ANDROID_NS, name) || el.getAttribute(`android:${name}`) || null;

/** Elements by tag name as an array. */
export const elements = (node, tag) => Array.from(node.getElementsByTagName(tag));

/** Bonjour-style Wi-Fi Aware service name (`_name._udp|_tcp`, RFC 6335 label ≤ 15). */
export function isValidWifiAwareServiceName(name) {
  const m = /^_([a-z0-9](?:[a-z0-9-]{0,13}[a-z0-9])?)\._(udp|tcp)$/.exec(name);
  if (!m) return false;
  const label = m[1];
  return !label.includes("--") && /[a-z]/.test(label);
}
