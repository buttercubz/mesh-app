// Invite deep links (DESIGN §11.1, §8.1): `convlinkvoice://join?v=1#<invite>` arrives
// through @capacitor/app `appUrlOpen` (SceneDelegate path on iOS, singleTask intent on
// Android). The URL goes straight to the join screen as an in-memory param, which passes
// it to `parseInvite`; it is never stored. Anything else is ignored.
import { SCREEN, useRouter } from "./router";

export const URL_SCHEME = "convlinkvoice";

/** `true` for a `convlinkvoice://join…` invite link. */
export function isInviteUrl(url: unknown): url is string {
  if (typeof url !== "string" || url.length > 4096) return false;
  return /^convlinkvoice:\/\/join(?:[/?#]|$)/i.test(url.trim());
}

/** Opens the join screen for an invite link; returns whether the URL was used. */
export function handleIncomingUrl(url: unknown): boolean {
  if (!isInviteUrl(url)) return false;
  useRouter.getState().push(SCREEN.join, { url: url.trim() });
  return true;
}
