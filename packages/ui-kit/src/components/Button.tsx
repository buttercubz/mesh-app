import type { ButtonHTMLAttributes, Ref } from "react";
import { cx } from "../cx.js";

export type ButtonVariant = "primary" | "secondary" | "plain" | "danger";
export type ButtonSize = "md" | "lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  readonly variant?: ButtonVariant;
  readonly size?: ButtonSize;
  /** Square 44 px icon button; give it an `aria-label`. */
  readonly iconOnly?: boolean;
  /** Full-width block button. */
  readonly block?: boolean;
  readonly ref?: Ref<HTMLButtonElement>;
}

/** Capsule button with a 44 px minimum target (DESIGN §11.4). Defaults to type="button". */
export function Button({
  variant = "secondary",
  size = "md",
  iconOnly = false,
  block = false,
  className,
  type = "button",
  ref,
  ...rest
}: ButtonProps) {
  return (
    <button
      ref={ref}
      type={type}
      data-slot="button"
      data-variant={variant}
      className={cx(
        "mk-button",
        `mk-button--${variant}`,
        size === "lg" && "mk-button--lg",
        iconOnly && "mk-button--icon",
        block && "mk-button--block",
        className,
      )}
      {...rest}
    />
  );
}
