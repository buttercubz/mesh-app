// Number, date and duration formatting of the Field test in the UI language (Intl
// locale per language, DESIGN Appendix E). Units come from the lab catalog.
import type { AppLanguage } from "../../i18n/index";
import { INTL_LOCALE } from "../../i18n/index";
import { clock } from "./live";

export interface LabFormat {
  /** Fixed number of fraction digits (grouping on). */
  num(value: number, digits?: number): string;
  /** Ratio 0..1 as a percentage number (no sign). */
  pct(ratio: number, digits?: number): string;
  /** Short date + time of an ISO stamp; the raw text when it does not parse. */
  dateTime(iso: string): string;
  /** Run length: `mm:ss` / `h:mm:ss`. */
  duration(seconds: number): string;
}

export function labFormat(language: AppLanguage): LabFormat {
  const locale = INTL_LOCALE[language];
  const cache = new Map<number, Intl.NumberFormat>();
  const nf = (digits: number) => {
    let f = cache.get(digits);
    if (f === undefined) {
      f = new Intl.NumberFormat(locale, { minimumFractionDigits: digits, maximumFractionDigits: digits });
      cache.set(digits, f);
    }
    return f;
  };
  const dtf = new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" });
  return {
    num: (value, digits = 0) => nf(digits).format(value),
    pct: (ratio, digits = 1) => nf(digits).format(ratio * 100),
    dateTime: (iso) => {
      const ms = Date.parse(iso);
      return Number.isFinite(ms) ? dtf.format(new Date(ms)) : iso;
    },
    duration: (seconds) => clock(seconds),
  };
}
