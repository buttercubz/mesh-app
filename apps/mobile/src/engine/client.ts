// Public engine entry point for UI code (WP-18 interface): `engine` — the typed client
// that routes GESTURE_COMMANDS through gestureCommand — and `useEngineEvents()`, the
// single event subscription. Features import from here, never from the plugin package
// (lint rule `no-restricted-imports` in eslint.config.js).
export { createAppEngine, engine } from "./appEngine";
export type { AppEngine, AppEngineDeps } from "./appEngine";
export {
  documentForeground,
  engineEventsRefCount,
  handleEngineEvent,
  startEngineEvents,
  useEngineEvents,
} from "./useEngineEvents";
export type { EventSinks, ForegroundSource } from "./useEngineEvents";
export { NOTICE_TONES, errorText, errorToast, noticeToast } from "./notices";
export { promptLang, promptTemplates, setPromptsCommand } from "./prompts";
export type { SetPromptsCommand } from "./prompts";
export { isOk, requiresGesture } from "@meshapp/engine-api";
export type {
  AwareStatus,
  DiagExportScope,
  DiagMarkLabel,
  DiagStatus,
  IntercomInfo,
  IntercomPermission,
  IntercomPermissionStatus,
} from "@meshapp/capacitor-intercom";
export { DIAG_MARK_LABELS } from "@meshapp/capacitor-intercom";
