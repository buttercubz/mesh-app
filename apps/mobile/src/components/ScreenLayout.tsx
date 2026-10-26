// Standard screen frame: one fixed-height column (the document never scrolls) with the
// title bar (owner of the top safe area, 48 px back button), the shell engine banner, the
// body (scrollable, or a fixed glance layout for the ride) and a footer pinned to the
// bottom (PTT/confirm stay on screen). Feature screens compose their content inside it.
// - A pinned footer publishes its height as `--app-toast-bottom` so the toast viewport
//   (AppToaster) docks just above it instead of covering the header or the first row.
// - `--app-keyboard-inset` is the on-screen keyboard height where the WebView does not
//   resize for it (iOS WKWebView): the frame shrinks by it so a pinned footer rides above
//   the keyboard. It stays 0 where the WebView resizes itself (Android, browsers).
// - Without a footer the body pads the bottom safe area itself, so the last control can
//   always be scrolled clear of the nav bar / home indicator.
import { Button, cx } from "@meshapp/ui-kit";
import { ChevronLeft } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useRef } from "react";

import { useRouter } from "../app/router";
import { useT } from "../i18n/index";
import { EngineStatusBanner } from "./EngineStatusBanner";
import { installIosTextSize } from "./textSize";
import { setToastDock } from "./toastDock";

export interface ScreenLayoutProps {
  readonly title: ReactNode;
  /** Show the back button (default: when there is a screen to go back to). */
  readonly back?: boolean;
  /** Called instead of popping the stack (e.g. confirm before leaving). */
  readonly onBack?: () => void;
  /** Right side of the title bar (one 48 px action). */
  readonly action?: ReactNode;
  /** Left-aligned large title (Home) instead of the centred compact one. */
  readonly largeTitle?: boolean;
  /** A soft brand glow behind the top of the screen. */
  readonly glow?: boolean;
  /** Pinned under the body (primary action, PTT button). */
  readonly footer?: ReactNode;
  /**
   * `true` (default): the body scrolls. `false`: a fixed glance layout (ride) whose content
   * must fit; on wide landscape windows (`side-footer` variant, index.css) the footer then
   * moves beside the body.
   */
  readonly scroll?: boolean;
  readonly children: ReactNode;
}

export function ScreenLayout({
  title,
  back,
  onBack,
  action,
  footer,
  largeTitle = false,
  glow = false,
  scroll = true,
  children,
}: ScreenLayoutProps) {
  const { t } = useT("common");
  const canGoBack = useRouter((s) => s.stack.length > 1);
  const showBack = back ?? canGoBack;
  const goBack = () => (onBack ? onBack() : useRouter.getState().back());
  const footerRef = useFooterToastDock(footer !== undefined);
  useKeyboardInset();
  // iOS Dynamic Type → root font size (once per app; a no-op elsewhere).
  useEffect(installIosTextSize, []);
  const side = !scroll && footer !== undefined;
  return (
    <div
      data-screen-frame=""
      className="relative isolate flex h-[calc(100dvh-var(--app-keyboard-inset,0px))] flex-col overflow-hidden bg-background text-foreground"
    >
      {glow && <div aria-hidden="true" className="pointer-events-none absolute inset-x-0 top-0 -z-10 h-80 [background:var(--hero-glow)]" />}
      <header className="flex min-h-14 shrink-0 items-center gap-1 px-2 pt-[env(safe-area-inset-top)]">
        {showBack ? (
          <Button variant="plain" iconOnly aria-label={t("action.back")} onClick={goBack} className="text-foreground">
            <ChevronLeft aria-hidden="true" strokeWidth={2.4} />
          </Button>
        ) : (
          !largeTitle && <span className="size-target" aria-hidden="true" />
        )}
        <h1
          className={cx(
            "min-w-0 flex-1",
            largeTitle
              ? "truncate px-2 text-2xl font-bold tracking-tight"
              : "text-center text-[1.0625rem] leading-tight font-semibold",
            // Up to two lines (a long group name stays recognisable); one on the glance
            // layout, whose height budget goes to the status and who is talking.
            !largeTitle && (scroll ? "line-clamp-2 text-balance break-words" : "truncate"),
          )}
        >
          {title}
        </h1>
        {action ?? <span className="size-target" aria-hidden="true" />}
      </header>
      <EngineStatusBanner bar={!scroll} className="shrink-0 px-4 pb-2" />
      <div className={cx("flex min-h-0 flex-1 flex-col", side && "side-footer:flex-row")}>
        <div
          data-screen-body=""
          className={cx(
            "min-h-0 flex-1 px-4 pt-2",
            scroll ? "overflow-y-auto" : "flex flex-col overflow-hidden",
            footer === undefined ? "pb-[calc(env(safe-area-inset-bottom)+1rem)]" : scroll ? "pb-4" : "pb-2",
          )}
        >
          {children}
        </div>
        {footer !== undefined && (
          <footer
            ref={footerRef}
            data-screen-footer=""
            className={cx(
              "shrink-0 border-t border-separator bg-background/80 px-4 pt-2 pb-[max(calc(env(safe-area-inset-bottom)-var(--app-keyboard-inset,0px)),1rem)] backdrop-blur-md android:backdrop-blur-none",
              side &&
                "side-footer:flex side-footer:w-[24rem] side-footer:flex-col side-footer:justify-center side-footer:border-t-0 side-footer:border-s",
            )}
          >
            {footer}
          </footer>
        )}
      </div>
    </div>
  );
}

/** CSS custom property read by the toast viewport (distance from the viewport bottom). */
const TOAST_BOTTOM_VAR = "--app-toast-bottom";
/** Gap between the pinned footer and the lowest toast. */
const TOAST_FOOTER_GAP_PX = 8;

/**
 * While a footer is pinned, keeps `--app-toast-bottom` on the document root equal to the
 * footer's height (which already includes the bottom safe area) plus a small gap. Only
 * the top screen of the stack is mounted, so one owner at a time; the variable is
 * removed on unmount and toasts fall back to the bottom safe area.
 */
function useFooterToastDock(enabled: boolean) {
  const ref = useRef<HTMLElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!enabled || el === null) return undefined;
    const root = document.documentElement;
    const update = () => {
      const r = el.getBoundingClientRect();
      // Beside the body (wide landscape) the footer does not cover the bottom edge: toasts
      // keep the default dock above the bottom safe area.
      if (r.left > 0) root.style.removeProperty(TOAST_BOTTOM_VAR);
      else root.style.setProperty(TOAST_BOTTOM_VAR, `${Math.ceil(r.height) + TOAST_FOOTER_GAP_PX}px`);
    };
    update();
    setToastDock(true);
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    observer?.observe(el);
    return () => {
      observer?.disconnect();
      root.style.removeProperty(TOAST_BOTTOM_VAR);
      setToastDock(false);
    };
  }, [enabled]);
  return ref;
}

/** CSS custom property: height of an on-screen keyboard the layout viewport ignores. */
const KEYBOARD_VAR = "--app-keyboard-inset";
/** Smaller differences are browser chrome or rounding, not a keyboard. */
const KEYBOARD_MIN_PX = 80;

/**
 * iOS WKWebView never resizes the layout viewport for the keyboard (only the visual
 * viewport shrinks) and pans the page to reveal the focused input. This keeps
 * `--app-keyboard-inset` equal to the covered height and undoes the pan, so the frame
 * (and its pinned footer) ends right above the keyboard. Where the WebView resizes
 * itself (Android with the system-bar IME padding, desktop browsers) the difference
 * stays 0 and nothing changes.
 */
function useKeyboardInset() {
  useEffect(() => {
    const vv = typeof window === "undefined" ? undefined : window.visualViewport;
    if (vv === undefined || vv === null) return undefined;
    const root = document.documentElement;
    const update = () => {
      const covered = Math.round(window.innerHeight - vv.height);
      const inset = covered >= KEYBOARD_MIN_PX ? covered : 0;
      if (inset > 0) root.style.setProperty(KEYBOARD_VAR, `${inset}px`);
      else root.style.removeProperty(KEYBOARD_VAR);
      if (inset > 0 && (vv.offsetTop > 0 || window.scrollY > 0)) window.scrollTo(0, 0);
    };
    update();
    vv.addEventListener("resize", update);
    vv.addEventListener("scroll", update);
    return () => {
      vv.removeEventListener("resize", update);
      vv.removeEventListener("scroll", update);
      root.style.removeProperty(KEYBOARD_VAR);
    };
  }, []);
}
