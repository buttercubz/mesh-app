// @meshapp/ui-kit public API (WP-20, DESIGN §11.5). Styles ship as separate entry
// points: "@meshapp/ui-kit/theme.css" (Tailwind v4 apps) or "@meshapp/ui-kit/styles.css".
export { cx } from "./cx.js";
export { DURATION_UI_MS, INTERCOM_TOKENS, PTT_SIZE_PX, TARGET_MIN_PX, TARGET_RIDE_PX, type IntercomToken } from "./constants.js";
export {
  applyTheme,
  resolveTheme,
  setAccessibilityClasses,
  setPlatformClass,
  type ResolvedTheme,
  type ThemePreference,
  type UiPlatform,
} from "./theme.js";

export { Button, type ButtonProps, type ButtonSize, type ButtonVariant } from "./components/Button.js";
export {
  BigPttButton,
  PTT_RING_CIRCUMFERENCE,
  type BigPttButtonProps,
  type PttStatus,
} from "./components/BigPttButton.js";
export {
  KEYBOARD_POINTER_ID,
  clampLevel,
  createPttGesture,
  type PttGesture,
  type PttGestureState,
  type PttMode,
  type PttReleaseReason,
  type PttSink,
} from "./components/ptt-gesture.js";
export {
  SegmentedControl,
  nextEnabledIndex,
  type SegmentOption,
  type SegmentedControlProps,
} from "./components/SegmentedControl.js";
export { ListRow, ListSection, type ListRowProps, type ListSectionProps } from "./components/ListRow.js";
export { Badge, type BadgeProps, type BadgeTone } from "./components/Badge.js";
export { Avatar, avatarHue, initials, type AvatarDotTone, type AvatarProps } from "./components/Avatar.js";
export {
  QualityBars,
  clampBars,
  qualityTone,
  type QualityBarsProps,
  type QualityTone,
} from "./components/QualityBars.js";
export { Sheet, type SheetCloseReason, type SheetProps } from "./components/Sheet.js";
export { Banner, type BannerAction, type BannerProps, type BannerTone } from "./components/Banner.js";
export { Toast, Toaster, type ToastProps, type ToasterProps } from "./components/Toast.js";
export {
  DEFAULT_TOAST_MS,
  MAX_TOASTS,
  createToaster,
  toaster,
  type ToastInput,
  type ToastItem,
  type ToastTone,
  type ToastStore,
  type TimerApi,
} from "./components/toast-store.js";
export {
  Sparkline,
  sparkDomain,
  sparklinePath,
  type SparkDomain,
  type SparkValue,
  type SparklineProps,
} from "./components/Sparkline.js";
