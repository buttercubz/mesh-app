/** Minimum touch target for every interactive primitive (DESIGN §11.4 driving rules). */
export const TARGET_MIN_PX = 44;
/** Size of the ride screen's PTT / mute button (DESIGN §11.4). */
export const PTT_SIZE_PX = 160;
/** UI transition duration ported from convLink (`--duration-ui`). */
export const DURATION_UI_MS = 220;

/** Intercom colour tokens (DESIGN §11.5); each also has a `-foreground` pair. */
export const INTERCOM_TOKENS = [
  "--talk-active",
  "--talk-blocked",
  "--status-relayed",
  "--status-out",
  "--ladder-warn",
  "--danger",
] as const;

export type IntercomToken = (typeof INTERCOM_TOKENS)[number];
