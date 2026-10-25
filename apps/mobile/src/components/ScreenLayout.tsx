// Standard screen frame (safe areas, title bar with a 48 px back button, scrollable
// body, footer pinned to the bottom of the viewport so PTT/confirm stay on screen). Feature screens compose their content inside it.
// A pinned footer publishes its height as `--app-toast-bottom` so the toast viewport
// (AppToaster) docks just above it instead of covering the header or the first row.
import { Button, cx } from "@meshapp/ui-kit";
import { ChevronLeft } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useRef } from "react";

import { useRouter } from "../app/router";
import { useT } from "../i18n/index";

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
  readonly children: ReactNode;
}

export function ScreenLayout({ title, back, onBack, action, footer, largeTitle = false, glow = false, children }: ScreenLayoutProps) {
  const { t } = useT("common");
  const canGoBack = useRouter((s) => s.stack.length > 1);
  const showBack = back ?? canGoBack;
  const goBack = () => (onBack ? onBack() : useRouter.getState().back());
  const footerRef = useFooterToastDock(footer !== undefined);
  return (
    <div className="relative isolate flex h-dvh flex-col overflow-hidden bg-background text-foreground">
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
            "min-w-0 flex-1 truncate",
            largeTitle ? "px-2 text-2xl font-bold tracking-tight" : "text-center text-[1.0625rem] font-semibold",
          )}
        >
          {title}
        </h1>
        {action ?? <span className="size-target" aria-hidden="true" />}
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 pt-2 pb-4">{children}</div>
      {footer !== undefined && (
        <footer
          ref={footerRef}
          data-screen-footer=""
          className="shrink-0 border-t border-separator bg-background/80 px-4 pt-3 pb-[max(env(safe-area-inset-bottom),1rem)] backdrop-blur-md android:backdrop-blur-none"
        >
          {footer}
        </footer>
      )}
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
      root.style.setProperty(TOAST_BOTTOM_VAR, `${Math.ceil(el.getBoundingClientRect().height) + TOAST_FOOTER_GAP_PX}px`);
    };
    update();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    observer?.observe(el);
    return () => {
      observer?.disconnect();
      root.style.removeProperty(TOAST_BOTTOM_VAR);
    };
  }, [enabled]);
  return ref;
}
