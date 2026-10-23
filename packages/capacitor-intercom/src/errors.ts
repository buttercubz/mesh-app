// Plugin rejections. Native bridges reject with `call.reject(code, code)`, so a
// rejection exposes the mesh-api `ErrorCode` both as `code` and as `message`; the web
// implementation does the same, which is what `rejectionCode()` of
// @meshapp/engine-api reads.
import type { ErrorCode } from "@meshapp/engine-api";
import { ERROR_CODES } from "@meshapp/engine-api";

const errorCodes = new Set<string>(ERROR_CODES);

/** A plugin rejection carrying a contract `ErrorCode`. */
export class IntercomError extends Error {
  readonly code: ErrorCode;
  /** Optional developer detail (never user-facing; the UI maps `code` through i18n). */
  readonly detail: string | undefined;

  constructor(code: ErrorCode, detail?: string) {
    super(code);
    this.name = "IntercomError";
    this.code = code;
    this.detail = detail;
  }
}

/** `true` when `value` is a plugin rejection with a contract `ErrorCode`. */
export function isIntercomError(value: unknown): value is { code: ErrorCode; message: string } {
  if (typeof value !== "object" || value === null) return false;
  const code = (value as Record<string, unknown>)["code"];
  return typeof code === "string" && errorCodes.has(code);
}
