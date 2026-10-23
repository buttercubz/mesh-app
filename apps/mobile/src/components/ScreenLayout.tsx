// Standard screen frame (safe areas, title bar with a 44 px back button, scrollable
// body, footer pinned to the bottom of the viewport so PTT/confirm stay on screen). Feature screens compose their content inside it.
// A pinned footer publishes its height as `--app-toast-bottom` so the toast viewport
// (AppToaster) docks just above it instead of covering the header or the first row.
import { Button } from "@meshapp/ui-kit";
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
  /** Right side of the title bar (one 44 px action). */
  readonly action?: ReactNode;
  /** Pinned under the body (primary action, PTT button). */
  readonly footer?: ReactNode;
  readonly children: ReactNode;
}

export function ScreenLayout({ title, back, onBack, action, footer, children }: ScreenLayoutProps) {
  const { t } = useT("common");
  const canGoBack = useRouter((s) => s.stack.length > 1);
  const showBack = back ?? canGoBack;
  const goBack = () => (onBack ? onBack() : useRouter.getState().back());
  const footerRef = useFooterToastDock(footer !== undefined);
  return (
    <div className="flex h-dvh flex-col overflow-hidden bg-background text-foreground">
      <header className="flex min-h-target shrink-0 items-center gap-2 px-3 pt-[env(safe-area-inset-top)]">
        {showBack ? (
          <Button variant="plain" iconOnly aria-label={t("action.back")} onClick={goBack}>
            <span aria-hidden="true">‹</span>
          </Button>
        ) : (
          <span className="size-target" aria-hidden="true" />
        )}
        <h1 className="min-w-0 flex-1 truncate text-center text-lg font-semibold">{title}</h1>
        {action ?? <span className="size-target" aria-hidden="true" />}
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">{children}</div>
      {footer !== undefined && <footer ref={footerRef} data-screen-footer="" className="shrink-0 px-4 pt-2 pb-[max(env(safe-area-inset-bottom),1rem)]">{footer}</footer>}
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
