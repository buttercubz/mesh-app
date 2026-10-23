// @meshapp/capacitor-intercom (WP-17, DESIGN §11.1): the TS-only MeshIntercom plugin.
// On iOS/Android the app-local bridges (WP-19) answer; in a browser the web
// implementation is loaded lazily, so native builds never pay for it.
import { registerPlugin } from "@capacitor/core";

import type { MeshIntercomPlugin } from "./definitions.js";
import { MESH_INTERCOM_JS_NAME } from "./definitions.js";
import type { MeshIntercomWeb } from "./web.js";

let webImpl: Promise<MeshIntercomWeb> | undefined;

/**
 * The page's one web implementation. Memoised: Capacitor does not dedupe concurrent
 * first loads, and two instances would split commands and event listeners (the UI's
 * boot `getInfo()` and its `addListener` racing would land on different engines).
 */
export function loadWebImplementation(): Promise<MeshIntercomWeb> {
  webImpl ??= import("./web.js").then((m) => new m.MeshIntercomWeb());
  return webImpl;
}

export const MeshIntercom = registerPlugin<MeshIntercomPlugin>(MESH_INTERCOM_JS_NAME, {
  web: loadWebImplementation,
});

export * from "./definitions.js";
export { IntercomError, isIntercomError } from "./errors.js";
export { PROTOCOL_VERSION } from "./version.js";
