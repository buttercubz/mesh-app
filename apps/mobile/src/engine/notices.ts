// Presentation of engine notices and errors (WP-18, DESIGN §1 principle, §3.7): the
// engine sends i18n keys + params; the app turns them into toasts through the `notices`
// and `errors` catalogs. Helmet cues (earcons/prompts) are played natively; the toast is
// the on-screen copy of the same state.
import type { ErrorCode, NoticeKey, ParamValue } from "@meshapp/engine-api";
import type { ToastInput, ToastTone } from "@meshapp/ui-kit";

import { hasTranslation, translate } from "../i18n/index";

/** Toast tone per notice (complete by construction: every NoticeKey is listed). */
export const NOTICE_TONES: Readonly<Record<NoticeKey, ToastTone>> = {
  memberJoined: "success",
  memberLeft: "neutral",
  memberLost: "warn",
  memberBack: "success",
  alone: "warn",
  partition: "warn",
  captureLost: "danger",
  captureRestored: "success",
  headsetLost: "danger",
  headsetBack: "success",
  onCall: "warn",
  callEnded: "success",
  removed: "danger",
  groupLocked: "warn",
  groupClosed: "danger",
  rideStarted: "success",
  rideEnded: "neutral",
  bluetoothOff: "danger",
  bluetoothOn: "success",
  bluetoothDenied: "danger",
  linkLimitReached: "warn",
  leaderChanged: "neutral",
  inviteLeaderMismatch: "warn",
  liveActivityRenewed: "neutral",
  batteryLow: "warn",
  thermalThrottled: "warn",
  talkLimit: "warn",
  voxLessSensitive: "warn",
  txMutedNoHeadset: "danger",
  labPeerJoined: "success",
  labPeerLost: "warn",
  labAnchorLost: "danger",
};

/** Notices that stay until dismissed or replaced (a rider must not miss them). */
const STICKY: ReadonlySet<NoticeKey> = new Set<NoticeKey>([
  "captureLost",
  "removed",
  "groupClosed",
  "bluetoothDenied",
  "inviteLeaderMismatch",
]);

/** Notices that replace each other on screen (same toast slot). */
const SLOT: Partial<Record<NoticeKey, string>> = {
  captureLost: "capture",
  captureRestored: "capture",
  headsetLost: "headset",
  headsetBack: "headset",
  onCall: "call",
  callEnded: "call",
  bluetoothOff: "bluetooth",
  bluetoothOn: "bluetooth",
  alone: "reach",
  partition: "reach",
  memberLost: "reach",
};

type Params = Readonly<Record<string, ParamValue>>;

/** Toast for a notice event, in the current UI language. */
export function noticeToast(key: NoticeKey, params: Params): ToastInput {
  return {
    title: translate("notices", key, { ...params }),
    tone: NOTICE_TONES[key],
    key: `notice.${SLOT[key] ?? key}`,
    ...(STICKY.has(key) ? { durationMs: 0 } : {}),
  };
}

/** User-facing text for an ErrorCode (`errors.unknown` when the code has no entry). */
export function errorText(code: ErrorCode | undefined, params: Params = {}): string {
  if (code !== undefined && hasTranslation("errors", code)) return translate("errors", code, { ...params });
  return translate("errors", "unknown");
}

/** Toast for an `error` event or a refused command. */
export function errorToast(code: ErrorCode | undefined, params: Params = {}): ToastInput {
  return { title: errorText(code, params), tone: "danger", key: `error.${code ?? "unknown"}` };
}
