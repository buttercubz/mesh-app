// Report files of the layout audit: each test writes `parts/<device>-<locale>.json`; the
// global teardown merges them into `report.json` and `summary.md` (counts per kind ×
// screen × platform and the worst items with their screenshots).
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import type { Locale, Platform } from "./devices";
import { DEVICES, EXTREME_ZOOMS } from "./devices";
import type { InsetEmulation } from "./page";
import type { MeasuredProblem, Severity } from "./measure";

export interface ReportProblem extends MeasuredProblem {
  screen: string;
  state: string;
  device: string;
  platform: Platform;
  zoom: number;
  locale: Locale;
  /** Screenshot path relative to the output directory. */
  screenshot: string;
}

export interface CaptureRecord {
  screen: string;
  state: string;
  device: string;
  platform: Platform;
  zoom: number;
  locale: Locale;
  screenshot: string;
  metrics: Record<string, number | null>;
  problemCount: number;
}

export interface ReportPart {
  device: string;
  platform: Platform;
  locale: Locale;
  mode: "audit" | "ci";
  insets: InsetEmulation;
  startedAt: string;
  durationMs: number;
  captures: CaptureRecord[];
  problems: ReportProblem[];
  pxFonts: Array<{ screen: string; state: string; selector: string; text: string; px: number }>;
}

/** Output directory: LAYOUT_OUT, else apps/mobile/test-results/layout (gitignored). */
export function outDir(): string {
  const fallback = fileURLToPath(new URL("../../test-results/layout", import.meta.url));
  return resolve(process.env["LAYOUT_OUT"] ?? fallback);
}

export function writePart(part: ReportPart): void {
  const dir = join(outDir(), "parts");
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, `${part.device}-${part.locale}.json`), JSON.stringify(part, null, 1));
}

const SEVERITY_WEIGHT: Record<Severity, number> = { error: 100, warn: 10, info: 1 };

/** Merges every part into report.json and summary.md; returns the paths. */
export function mergeReport(): { report: string; summary: string } | null {
  const dir = outDir();
  const partsDir = join(dir, "parts");
  let files: string[];
  try {
    files = readdirSync(partsDir).filter((f) => f.endsWith(".json")).sort();
  } catch (_e) {
    // No test wrote a part (filtered run, setup failure): nothing to merge.
    return null;
  }
  const parts = files.map((f) => JSON.parse(readFileSync(join(partsDir, f), "utf8")) as ReportPart);
  const problems = parts.flatMap((p) => p.problems);
  const report = {
    generatedAt: new Date().toISOString(),
    outDir: dir,
    mode: parts[0]?.mode ?? "audit",
    devices: parts.map((p) => ({ device: p.device, platform: p.platform, locale: p.locale, insets: p.insets, captures: p.captures.length, problems: p.problems.length, durationMs: p.durationMs })),
    totals: {
      captures: parts.reduce((n, p) => n + p.captures.length, 0),
      problems: problems.length,
      bySeverity: countBy(problems, (p) => p.severity),
      byKind: countBy(problems, (p) => p.kind),
    },
    problems,
    pxFonts: dedupePxFonts(parts),
    captures: parts.flatMap((p) => p.captures),
  };
  const reportPath = join(dir, "report.json");
  writeFileSync(reportPath, JSON.stringify(report, null, 1));
  const summaryPath = join(dir, "summary.md");
  writeFileSync(summaryPath, summarize(report.problems, parts, report.pxFonts, dir));
  return { report: reportPath, summary: summaryPath };
}

function countBy<T>(list: readonly T[], key: (x: T) => string): Record<string, number> {
  const out: Record<string, number> = {};
  for (const x of list) out[key(x)] = (out[key(x)] ?? 0) + 1;
  return Object.fromEntries(Object.entries(out).sort((a, b) => b[1] - a[1]));
}

function dedupePxFonts(parts: readonly ReportPart[]) {
  const seen = new Map<string, { selector: string; text: string; px: number; screens: Set<string> }>();
  for (const p of parts) {
    for (const f of p.pxFonts) {
      const k = `${f.selector}|${f.px}`;
      const entry = seen.get(k) ?? { selector: f.selector, text: f.text, px: f.px, screens: new Set<string>() };
      entry.screens.add(`${f.screen}/${f.state}`);
      seen.set(k, entry);
    }
  }
  return [...seen.values()].map((e) => ({ selector: e.selector, text: e.text, px: e.px, screens: [...e.screens].sort() }));
}

function summarize(
  problems: readonly ReportProblem[],
  parts: readonly ReportPart[],
  pxFonts: ReturnType<typeof dedupePxFonts>,
  dir: string,
): string {
  const lines: string[] = [];
  const kinds = [...new Set(problems.map((p) => p.kind))].sort();
  const screens = [...new Set(problems.map((p) => p.screen))].sort();
  lines.push("# Layout audit summary", "");
  lines.push(`Generated ${new Date().toISOString()} from ${parts.length} device×locale runs, ${parts.reduce((n, p) => n + p.captures.length, 0)} captures, ${problems.length} problems.`, "");
  lines.push(`Output: \`${dir}\` (report.json, shots/<device>/<locale>/z<zoom>/<screen>--<state>.jpg).`, "");
  lines.push("## Insets emulation", "");
  for (const p of parts) {
    lines.push(`- ${p.device}/${p.locale}: ${p.insets.method}, env top ${p.insets.envTop} bottom ${p.insets.envBottom}${p.insets.error !== undefined ? ` (${p.insets.error})` : ""}`);
  }
  lines.push("", "## Problems by severity", "");
  const sev = countBy(problems, (p) => p.severity);
  lines.push(`error ${sev["error"] ?? 0}, warn ${sev["warn"] ?? 0}, info ${sev["info"] ?? 0}`, "");

  for (const platform of ["android", "ios"] as const) {
    const pl = problems.filter((p) => p.platform === platform);
    lines.push(`## ${platform === "android" ? "Android" : "iOS"}: kind × screen (all zooms and locales)`, "");
    if (pl.length === 0) {
      lines.push("No problems.", "");
      continue;
    }
    const plKinds = kinds.filter((k) => pl.some((p) => p.kind === k));
    lines.push(`| screen | ${plKinds.join(" | ")} | total |`);
    lines.push(`|---|${plKinds.map(() => "---:").join("|")}|---:|`);
    for (const s of screens) {
      const row = pl.filter((p) => p.screen === s);
      if (row.length === 0) continue;
      lines.push(`| ${s} | ${plKinds.map((k) => row.filter((p) => p.kind === k).length || "").join(" | ")} | ${row.length} |`);
    }
    lines.push("");
  }

  // The field report: banners pushing the PTT screen into scrolling. One row per ride
  // state, one column per device, at 100 % text (es or en): S = the screen needs scrolling,
  // H = a banner/PTT/mode control or another control is hidden or cut, X = horizontal
  // overflow, Z = a control under a system bar; "ok" = no error.
  const rideStates = [...new Set(problems.filter((p) => p.screen === "ride").map((p) => p.state))];
  const allRideStates = [...new Set(parts.flatMap((p) => p.captures.filter((c) => c.screen === "ride").map((c) => c.state)))];
  const deviceIds = DEVICES.map((d) => d.id).filter((id) => parts.some((p) => p.device === id));
  if (allRideStates.length > 0) {
    lines.push("## Ride screen at 100 % text: errors per state and device", "");
    lines.push("S = needs vertical scroll, H = key control/banner hidden or cut, X = horizontal overflow, Z = control under a system bar, ok = no error (es or en).", "");
    lines.push(`| ride state | ${deviceIds.join(" | ")} |`, `|---|${deviceIds.map(() => ":---:").join("|")}|`);
    for (const st of allRideStates) {
      const cells = deviceIds.map((id) => {
        const errs = problems.filter((p) => p.screen === "ride" && p.state === st && p.device === id && p.zoom === 100 && p.severity === "error");
        const flags = new Set<string>();
        for (const p of errs) {
          if (p.kind === "bodyScroll" || p.kind === "pageVScroll") flags.add("S");
          else if (p.kind === "hiddenKey" || p.kind === "hiddenPinned" || p.kind === "pttSmall" || p.kind === "occluded") flags.add("H");
          else if (p.kind === "hOverflow" || p.kind === "offscreenX") flags.add("X");
          else if (p.kind === "underSafeArea") flags.add("Z");
        }
        return flags.size === 0 ? "ok" : [...flags].sort().join("");
      });
      lines.push(`| ${st} | ${cells.join(" | ")} |`);
    }
    lines.push("", `Ride states with at least one problem of any severity: ${rideStates.length} of ${allRideStates.length}.`, "");
  }

  lines.push("## Errors by device × zoom (ride screen)", "");
  const rideErr = problems.filter((p) => p.screen === "ride" && p.severity === "error");
  const combos = [...new Set(problems.map((p) => `${p.device}@${p.zoom}`))].sort();
  lines.push("| device@zoom | ride errors | ride states with errors |", "|---|---:|---:|");
  for (const c of combos) {
    const list = rideErr.filter((p) => `${p.device}@${p.zoom}` === c);
    if (list.length > 0) lines.push(`| ${c} | ${list.length} | ${new Set(list.map((p) => p.state)).size} |`);
  }
  lines.push("");

  // Worst items: one row per (screen, state, kind, selector), ranked by severity, then by
  // how many device/zoom/locale configurations show it, then by magnitude.
  // Configurations count only realistic text sizes (the 200 % extreme is reported apart);
  // the worst case shown is the largest one at a realistic size when there is one.
  const groups = new Map<string, { sample: ReportProblem; configs: Set<string>; maxMag: number; worst: ReportProblem }>();
  for (const p of problems) {
    const k = `${p.screen}|${p.state}|${p.kind}|${p.selector}`;
    const realistic = !EXTREME_ZOOMS.has(p.zoom);
    const g = groups.get(k) ?? { sample: p, configs: new Set<string>(), maxMag: -1, worst: p };
    if (realistic) g.configs.add(`${p.device}/${p.zoom}/${p.locale}`);
    const worstRealistic = !EXTREME_ZOOMS.has(g.worst.zoom);
    if ((realistic && (!worstRealistic || p.magnitude > g.maxMag)) || (!realistic && !worstRealistic && p.magnitude > g.maxMag) || g.maxMag < 0) {
      g.maxMag = p.magnitude;
      g.worst = p;
    }
    if (SEVERITY_WEIGHT[p.severity] > SEVERITY_WEIGHT[g.sample.severity]) g.sample = p;
    groups.set(k, g);
  }
  const ranked = [...groups.values()].sort(
    (a, b) =>
      SEVERITY_WEIGHT[b.sample.severity] - SEVERITY_WEIGHT[a.sample.severity] ||
      b.configs.size - a.configs.size ||
      b.maxMag - a.maxMag,
  );
  // At most two rows per (screen, kind), so the list shows 20 different problems.
  const perKind = new Map<string, number>();
  const top = ranked.filter((g) => {
    const k = `${g.sample.screen}|${g.sample.kind}`;
    const n = perKind.get(k) ?? 0;
    perKind.set(k, n + 1);
    return n < 2;
  });
  lines.push("## Top 20 worst items", "");
  lines.push("Ranked by severity, then by the number of device/zoom/locale configurations showing it, then by size; at most two rows per screen and kind.", "");
  lines.push("| # | severity | screen/state | kind | element | configs | worst case | screenshot |", "|---:|---|---|---|---|---:|---|---|");
  top.slice(0, 20).forEach((g, i) => {
    const w = g.worst;
    const el = `\`${w.selector}\`${w.text !== "" ? ` "${w.text.slice(0, 40).replace(/\|/g, "/")}"` : ""}`;
    lines.push(
      `| ${i + 1} | ${g.sample.severity} | ${w.screen}/${w.state} | ${w.kind} | ${el} | ${g.configs.size} | ${w.device}@${w.zoom}% ${w.locale}: ${w.detail.replace(/\|/g, "/")} | ${join(dir, w.screenshot)} |`,
    );
  });
  lines.push("");

  if (pxFonts.length > 0) {
    lines.push("## Font sizes in px (escape the textZoom emulation; WebView textZoom scales them anyway)", "");
    for (const f of pxFonts) lines.push(`- \`${f.selector}\` ${f.px}px "${f.text}" on ${f.screens.slice(0, 6).join(", ")}${f.screens.length > 6 ? ", …" : ""}`);
    lines.push("");
  }
  lines.push(`Screenshots root: ${join(dir, "shots")}`, "");
  return lines.join("\n");
}
