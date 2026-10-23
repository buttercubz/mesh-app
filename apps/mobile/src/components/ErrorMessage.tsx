// Inline text for a refused command or error event (`errors` catalog). Components never
// show raw ErrorCodes or engine text.
import type { ErrorCode } from "@meshapp/engine-api";

import { errorText } from "../engine/client";
import { useT } from "../i18n/index";

export interface ErrorMessageProps {
  readonly code: ErrorCode | undefined;
  readonly params?: Readonly<Record<string, string | number>>;
  readonly className?: string;
}

export function ErrorMessage({ code, params, className }: ErrorMessageProps) {
  // Subscribing to the language keeps the text in sync with a language switch.
  const { language } = useT("errors");
  return (
    <p role="alert" lang={language} className={className ?? "text-sm text-danger"}>
      {errorText(code, params ?? {})}
    </p>
  );
}
