// In-page layout checks of the layout audit. `measureLayout` and `findPxFonts` are passed
// to `page.evaluate`, so each is self-contained (no imports, no module-level helpers).

export type Severity = "error" | "warn" | "info";

export type ProblemKind =
  /** The document scrolls horizontally. */
  | "hOverflow"
  /** An element sticks out of the viewport horizontally (not clipped by a scroller). */
  | "offscreenX"
  /** The document scrolls vertically although screens are a fixed h-dvh frame. */
  | "pageVScroll"
  /** The screen body needs vertical scrolling (ride, and screens with a pinned footer). */
  | "bodyScroll"
  /** Ride: a banner, the PTT/mute button, the mode control or another key/interactive element is not fully visible. */
  | "hiddenKey"
  /** A pinned (header/footer/toast/sheet) interactive element is not fully inside the viewport. */
  | "hiddenPinned"
  /** An interactive element is covered by another element at its centre. */
  | "occluded"
  /** Text spills out of its box. */
  | "textOverflow"
  /** Text is cut by overflow:hidden without an ellipsis. */
  | "textClipped"
  /** Ellipsis truncation of key information (names, counts, titles, labels). */
  | "truncatedKey"
  /** Ellipsis truncation of secondary text. */
  | "truncated"
  /** A button/chip/badge/segment label wraps to 2+ lines. */
  | "wrap"
  /** A word is broken across lines. */
  | "midWordBreak"
  /** Text squeezed into a column so narrow that it shows about one word per line. */
  | "squeezed"
  /** A heading ends with a single-word line. */
  | "orphan"
  /** Two interactive elements overlap. */
  | "overlap"
  /** An interactive element lies in the status-bar / notch / home-indicator / nav-bar zone. */
  | "underSafeArea"
  /** Non-interactive text lies in a safe-area zone. */
  | "textUnderSafeArea"
  /** Touch target smaller than 44x44 px. */
  | "smallTarget"
  /** The PTT/mute button is smaller than 120 px. */
  | "pttSmall"
  /** Font size in px: escapes the textZoom emulation (WebView textZoom scales it anyway). */
  | "pxFont"
  /** The harness could not set up or measure a state. */
  | "harnessError";

export interface MeasuredProblem {
  kind: ProblemKind;
  severity: Severity;
  selector: string;
  testid: string | null;
  text: string;
  detail: string;
  /** Size of the problem (px, %, lines) for ranking. */
  magnitude: number;
}

export interface MeasureOptions {
  insetTop: number;
  insetBottom: number;
  /** The ride screen (strict: no scroll, every key element visible). */
  ride: boolean;
  /** Extreme zoom: severities are lowered one step. */
  extreme: boolean;
  /** Probe reading of env(safe-area-inset-top/bottom) in px. */
  probe?: boolean;
}

export interface MeasureResult {
  problems: MeasuredProblem[];
  metrics: {
    vw: number;
    vh: number;
    docScrollWidth: number;
    docScrollHeight: number;
    bodyScrollHeight: number | null;
    bodyClientHeight: number | null;
    envTop: number | null;
    envBottom: number | null;
    rootFontSize: number;
  };
}

export function measureLayout(opts: MeasureOptions): MeasureResult {
  const problems: MeasuredProblem[] = [];
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const doc = document.documentElement;
  const TOL = 1.5;

  type R = { left: number; top: number; right: number; bottom: number };
  const area = (r: R) => Math.max(0, r.right - r.left) * Math.max(0, r.bottom - r.top);
  const inter = (a: R, b: R): R => ({
    left: Math.max(a.left, b.left),
    top: Math.max(a.top, b.top),
    right: Math.min(a.right, b.right),
    bottom: Math.min(a.bottom, b.bottom),
  });
  const viewport: R = { left: 0, top: 0, right: vw, bottom: vh };

  const styleCache = new Map<Element, CSSStyleDeclaration>();
  const cs = (el: Element) => {
    let s = styleCache.get(el);
    if (s === undefined) {
      s = getComputedStyle(el);
      styleCache.set(el, s);
    }
    return s;
  };

  const lower = (s: Severity): Severity => (opts.extreme ? (s === "error" ? "warn" : "info") : s);

  const testidOf = (el: Element): string | null => {
    const own = el.closest("[data-testid]");
    return own === null ? null : own.getAttribute("data-testid");
  };

  const describe = (el: Element): string => {
    const parts: string[] = [];
    let cur: Element | null = el;
    for (let depth = 0; cur !== null && depth < 4; depth += 1, cur = cur.parentElement) {
      const tid = cur.getAttribute("data-testid");
      const banner = cur.getAttribute("data-banner");
      let part = cur.tagName.toLowerCase();
      if (tid !== null) part = `[data-testid="${tid}"]`;
      else if (banner !== null) part = `[data-banner="${banner}"]`;
      else {
        const role = cur.getAttribute("role");
        const cls = [...cur.classList].filter((c) => c.startsWith("mk-")).slice(0, 2);
        if (cls.length > 0) part += "." + cls.join(".");
        if (role !== null) part += `[role=${role}]`;
      }
      parts.unshift(part);
      if (tid !== null || banner !== null) break;
    }
    return parts.join(" > ");
  };

  const textOf = (el: Element): string => {
    const label = el.getAttribute("aria-label");
    const t = ((el as HTMLElement).innerText ?? el.textContent ?? "").replace(/\s+/g, " ").trim();
    const s = t !== "" ? t : (label ?? "");
    return s.length > 80 ? s.slice(0, 77) + "..." : s;
  };

  const add = (kind: ProblemKind, severity: Severity, el: Element | null, detail: string, magnitude: number) => {
    problems.push({
      kind,
      severity: lower(severity),
      selector: el === null ? "document" : describe(el),
      testid: el === null ? null : testidOf(el),
      text: el === null ? "" : textOf(el),
      detail,
      magnitude: Math.round(magnitude * 10) / 10,
    });
  };

  const isSrOnly = (el: Element): boolean => {
    for (let a: Element | null = el; a !== null && a !== document.body; a = a.parentElement) {
      if (a.classList.contains("sr-only")) return true;
    }
    return false;
  };

  const rendered = (el: Element): boolean => {
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return false;
    const s = cs(el);
    if (s.visibility === "hidden" || s.display === "none") return false;
    for (let a: Element | null = el; a !== null && a !== document.body; a = a.parentElement) {
      if (Number(cs(a).opacity) === 0) return false;
    }
    return !isSrOnly(el);
  };

  /** The part of `el` not clipped by scrolling/hidden ancestors nor the viewport. */
  const clipChain = (el: Element): { rect: R; scroller: Element | null } => {
    let r: R = el.getBoundingClientRect();
    let scroller: Element | null = null;
    for (let a = el.parentElement; a !== null && a !== doc; a = a.parentElement) {
      const s = cs(a);
      if (s.overflowX !== "visible" || s.overflowY !== "visible") {
        const ar = a.getBoundingClientRect();
        const he = a as HTMLElement;
        const box: R = {
          left: ar.left + he.clientLeft,
          top: ar.top + he.clientTop,
          right: ar.left + he.clientLeft + (he.clientWidth || ar.width),
          bottom: ar.top + he.clientTop + (he.clientHeight || ar.height),
        };
        r = inter(r, box);
        if (scroller === null && (s.overflowY === "auto" || s.overflowY === "scroll")) scroller = a;
      }
      if (s.position === "fixed") break;
    }
    return { rect: inter(r, viewport), scroller };
  };

  // ---- frame ------------------------------------------------------------------------
  // ScreenLayout marks its parts with data attributes (older builds: class names).
  const frame = document.querySelector("main [data-screen-frame]") ?? document.querySelector("main > div.h-dvh");
  const body =
    (frame?.querySelector("[data-screen-body]") as HTMLElement | null) ??
    (frame?.querySelector(":scope > div.overflow-y-auto") as HTMLElement | null) ??
    null;
  const footer = frame?.querySelector("[data-screen-footer]") ?? frame?.querySelector(":scope > footer") ?? null;
  const dialog = [...document.querySelectorAll('[role="dialog"]')].find((d) => rendered(d)) ?? null;
  const toasts = [...document.querySelectorAll(".mk-toast")].filter((t) => rendered(t));

  // (a) horizontal overflow of the document.
  const docScrollWidth = doc.scrollWidth;
  if (docScrollWidth > doc.clientWidth + 0.5) {
    add("hOverflow", "error", null, `document scrollWidth ${docScrollWidth} > ${doc.clientWidth}`, docScrollWidth - doc.clientWidth);
  }
  // (b) vertical: the frame is h-dvh, the page itself must not scroll.
  const docScrollHeight = Math.max(doc.scrollHeight, document.body.scrollHeight);
  if (frame !== null && docScrollHeight > vh + TOL) {
    add(
      "pageVScroll",
      opts.ride ? "error" : "warn",
      null,
      `page scrollHeight ${docScrollHeight} > viewport ${vh} (content pushed below the fold)`,
      docScrollHeight - vh,
    );
  }
  const bodyOverflow = body === null ? 0 : body.scrollHeight - body.clientHeight;
  if (body !== null && bodyOverflow > TOL && (opts.ride || footer !== null)) {
    // A few px of scroll hide nothing (rounding, padding): reported, but only as info.
    add(
      "bodyScroll",
      opts.ride && bodyOverflow > 4 ? "error" : "info",
      body,
      `screen body needs ${Math.round(bodyOverflow)} px of vertical scroll (${body.clientHeight} px visible of ${body.scrollHeight})`,
      bodyOverflow,
    );
  }
  const maxScroll = body === null ? 0 : Math.max(0, body.scrollHeight - body.clientHeight);
  const scrollLeft = body === null ? 0 : maxScroll - body.scrollTop;

  // ---- element sets --------------------------------------------------------------------
  const scope: Element = dialog ?? document.body;
  const INTERACTIVE =
    'button, a[href], input:not([type="hidden"]), select, textarea, [role="button"], [role="radio"], [role="switch"], [role="tab"], [role="checkbox"], [role="link"], summary';
  const interactive = [...scope.querySelectorAll(INTERACTIVE)].filter(
    (el) => rendered(el) && el.closest('[aria-hidden="true"]') === null && !(el.parentElement?.closest(INTERACTIVE) ?? null),
  );
  // Toasts sit above a dialog's scrim too.
  if (dialog !== null) for (const t of toasts) interactive.push(...[...t.querySelectorAll(INTERACTIVE)].filter(rendered));

  const inBody = (el: Element) => body !== null && body.contains(el);
  const zoneTop = opts.insetTop;
  const zoneBottom = vh - opts.insetBottom;

  // (b) ride: key elements fully visible without scrolling.
  if (opts.ride && dialog === null) {
    const KEY =
      '[data-banner], .mk-ptt, [data-testid="mute-toggle"], [data-testid="talk-control"] [role="radiogroup"], [data-testid="ride-status-chip"], [data-testid="heard-by"], [data-testid="talker-chip"], [data-testid="open-public-notice"]';
    const keys = new Set<Element>([...document.querySelectorAll(KEY)].filter(rendered));
    for (const el of interactive) keys.add(el);
    for (const el of keys) {
      const full = el.getBoundingClientRect();
      const vis = clipChain(el).rect;
      const fullA = area(full);
      const visA = area(vis);
      const outOfZone = full.top < zoneTop - TOL || full.bottom > zoneBottom + TOL;
      if (fullA > 0 && (visA < fullA - 2 * Math.max(full.width, full.height) || outOfZone)) {
        const pct = Math.round((visA / fullA) * 100);
        add(
          "hiddenKey",
          "error",
          el,
          `${pct}% visible without scrolling${outOfZone ? " (part in a safe-area zone)" : ""}; box ${Math.round(full.top)}-${Math.round(full.bottom)} of viewport ${vh}`,
          100 - pct,
        );
      }
    }
    const ptt = document.querySelector('.mk-ptt, [data-testid="mute-toggle"]');
    if (ptt !== null && rendered(ptt)) {
      const r = ptt.getBoundingClientRect();
      const size = Math.min(r.width, r.height);
      if (size < 120) add("pttSmall", "error", ptt, `PTT/mute button ${Math.round(r.width)}x${Math.round(r.height)} < 120 px`, 120 - size);
    }
  }

  // Content of an inner scroller other than the screen body (a sheet's body): reachable by
  // scrolling that scroller, so its own box is what must be on screen and clear of the
  // safe-area zones; the element must fit inside it (height and width).
  const innerScroller = (el: Element): Element | null => {
    const sc = clipChain(el).scroller;
    return sc !== null && sc !== body && !(body?.contains(sc) ?? false) ? sc : null;
  };

  // Pinned interactive elements (header, footer, toasts, sheet): fully inside the viewport.
  for (const el of interactive) {
    if (inBody(el)) continue;
    const r = el.getBoundingClientRect();
    const sc = innerScroller(el);
    if (sc !== null) {
      const sb = sc.getBoundingClientRect();
      const boxVis = area(inter(sb, viewport));
      const fitsV = r.height <= (sc as HTMLElement).clientHeight + TOL;
      const fitsH = r.left >= sb.left - TOL && r.right <= sb.right + TOL;
      if (!fitsV || !fitsH || boxVis < area(sb) - 2 * Math.max(sb.width, sb.height)) {
        add("hiddenPinned", "error", el, `control in a scrolling panel cannot be fully revealed (${fitsH ? "" : "wider than the panel; "}${fitsV ? "" : "taller than the panel; "}panel ${Math.round((boxVis / Math.max(1, area(sb))) * 100)}% on screen)`, 100);
      }
      continue;
    }
    const vis = clipChain(el).rect;
    if (area(vis) < area(r) - 2 * Math.max(r.width, r.height)) {
      add("hiddenPinned", "error", el, `pinned control only ${Math.round((area(vis) / area(r)) * 100)}% inside the viewport`, 100 - (area(vis) / area(r)) * 100);
    }
  }

  // (e) safe-area zones: pinned controls where they are; body controls at both scroll ends.
  for (const el of interactive) {
    const sc = inBody(el) ? null : innerScroller(el);
    // Inner scroller content: the scroller's box stands in for it (it scrolls within it).
    const r = sc === null ? el.getBoundingClientRect() : sc.getBoundingClientRect();
    if (!inBody(el)) {
      if (r.top < zoneTop - TOL && r.bottom > 0) {
        add("underSafeArea", "error", el, `top ${Math.round(r.top)} px is under the top inset (${zoneTop} px: status bar / notch)`, zoneTop - r.top);
      } else if (r.bottom > zoneBottom + TOL && r.top < vh) {
        add("underSafeArea", "error", el, `bottom ${Math.round(r.bottom)} px is under the bottom inset (from ${zoneBottom} px: home indicator / nav bar)`, r.bottom - zoneBottom);
      }
      continue;
    }
    // Body content: at scroll top nothing may sit in the top zone; at the scroll end the last
    // controls must clear the bottom zone (otherwise they can never be fully revealed).
    const topAtStart = r.top + (body?.scrollTop ?? 0);
    if (topAtStart < zoneTop - TOL) {
      add("underSafeArea", "error", el, `at scroll top it starts at ${Math.round(topAtStart)} px, under the top inset (${zoneTop} px)`, zoneTop - topAtStart);
    }
    const bottomAtEnd = r.bottom - scrollLeft;
    if (footer === null && bottomAtEnd > zoneBottom + TOL && bottomAtEnd - r.height < vh) {
      add(
        "underSafeArea",
        "error",
        el,
        `at the end of scrolling it ends at ${Math.round(bottomAtEnd)} px, under the bottom inset (from ${zoneBottom} px)`,
        bottomAtEnd - zoneBottom,
      );
    }
  }
  // Visible text in a safe-area zone (header text under the status bar, content under the nav bar).
  const textEls: Element[] = [];
  {
    const walker = document.createTreeWalker(scope, NodeFilter.SHOW_TEXT);
    const seen = new Set<Element>();
    for (let n = walker.nextNode(); n !== null; n = walker.nextNode()) {
      if ((n.textContent ?? "").trim() === "") continue;
      const parent = n.parentElement;
      if (parent === null || seen.has(parent)) continue;
      seen.add(parent);
      if (parent.closest("script, style, svg, [aria-hidden=\"true\"]") !== null) continue;
      if (rendered(parent)) textEls.push(parent);
    }
  }
  for (const el of textEls) {
    if (el.closest(INTERACTIVE) !== null) continue;
    const vis = clipChain(el).rect;
    if (area(vis) < 4) continue;
    if (vis.top < zoneTop - TOL) {
      add("textUnderSafeArea", "warn", el, `visible text at ${Math.round(vis.top)} px under the top inset (${zoneTop} px)`, zoneTop - vis.top);
    } else if (!inBody(el) && vis.bottom > zoneBottom + TOL) {
      add("textUnderSafeArea", "warn", el, `visible text down to ${Math.round(vis.bottom)} px under the bottom inset (from ${zoneBottom} px)`, vis.bottom - zoneBottom);
    }
  }

  // (a) elements sticking out horizontally.
  for (const el of [...interactive, ...textEls]) {
    const r = el.getBoundingClientRect();
    if (r.right <= vw + TOL && r.left >= -TOL) continue;
    const vis = clipChain(el).rect;
    // Clipped by a scroller/hidden ancestor inside the viewport: textClipped covers it.
    if (vis.right < vw - TOL && vis.left > TOL && area(vis) > 0 && r.right > vw) continue;
    add("offscreenX", "error", el, `box ${Math.round(r.left)}..${Math.round(r.right)} outside the ${vw} px viewport`, Math.max(r.right - vw, -r.left));
  }

  // (e) occlusion at the centre, only where the element is meant to be visible now.
  for (const el of interactive) {
    const vis = clipChain(el).rect;
    if (area(vis) < 16) continue;
    const r = el.getBoundingClientRect();
    const cx = (r.left + r.right) / 2;
    const cy = (r.top + r.bottom) / 2;
    if (cx < vis.left || cx > vis.right || cy < vis.top || cy > vis.bottom) continue;
    const hit = document.elementFromPoint(cx, cy);
    if (hit === null || el === hit || el.contains(hit) || hit.contains(el)) continue;
    const cover = hit.closest('.mk-toast, header, footer, [role="dialog"], [data-banner], button') ?? hit;
    add("occluded", opts.ride ? "error" : "warn", el, `centre covered by ${describe(cover)}`, area(r));
  }

  // (e) overlaps between interactive elements.
  const visRects = interactive.map((el) => ({ el, r: clipChain(el).rect }));
  for (let i = 0; i < visRects.length; i += 1) {
    const a = visRects[i];
    if (a === undefined || area(a.r) < 4) continue;
    for (let j = i + 1; j < visRects.length; j += 1) {
      const b = visRects[j];
      if (b === undefined || area(b.r) < 4) continue;
      if (a.el.contains(b.el) || b.el.contains(a.el)) continue;
      const ov = area(inter(a.r, b.r));
      if (ov > 4) add("overlap", "warn", a.el, `overlaps ${describe(b.el)} "${textOf(b.el)}" by ${Math.round(ov)} px²`, ov);
    }
  }

  // (f) touch targets.
  for (const el of interactive) {
    const r = el.getBoundingClientRect();
    if (r.width >= 44 - 0.5 && r.height >= 44 - 0.5) continue;
    if ((el as HTMLInputElement).type === "hidden") continue;
    add("smallTarget", "warn", el, `touch target ${Math.round(r.width)}x${Math.round(r.height)} px < 44x44`, 44 - Math.min(r.width, r.height));
  }

  // (c) text overflow / truncation, (d) wraps.
  const KEY_TRUNC =
    'button, h1, h2, h3, [role="radio"], .mk-badge, .mk-banner, [data-testid="talker-chip"], [data-member], [data-testid="heard-by"], [data-testid="ride-status-chip"], li, .mk-toast';
  const lineCount = (el: Element): { lines: number; lastLineWords: number; words: number } => {
    const rects: DOMRect[] = [];
    const words: { top: number }[] = [];
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    const range = document.createRange();
    for (let n = walker.nextNode(); n !== null; n = walker.nextNode()) {
      const text = n.textContent ?? "";
      if (text.trim() === "") continue;
      if (n.parentElement !== null && isSrOnly(n.parentElement)) continue;
      range.selectNodeContents(n);
      for (const r of range.getClientRects()) if (r.width > 0.5) rects.push(r);
      const re = /[\p{L}\p{N}]+/gu;
      for (let m = re.exec(text); m !== null; m = re.exec(text)) {
        range.setStart(n, m.index);
        range.setEnd(n, m.index + m[0].length);
        const wr = [...range.getClientRects()].filter((x) => x.width > 0.5);
        if (wr[0] !== undefined) words.push({ top: wr[wr.length - 1]?.top ?? wr[0].top });
      }
    }
    rects.sort((x, y) => x.top - y.top);
    let lines = 0;
    let lastTop = -Infinity;
    let lastH = 0;
    for (const r of rects) {
      if (r.top > lastTop + Math.max(4, lastH * 0.5)) {
        lines += 1;
        lastTop = r.top;
        lastH = r.height;
      }
    }
    const maxTop = words.reduce((m, w) => Math.max(m, w.top), -Infinity);
    const lastLineWords = words.filter((w) => w.top > maxTop - 4).length;
    return { lines, lastLineWords, words: words.length };
  };

  for (const el of textEls) {
    const s = cs(el);
    const he = el as HTMLElement;
    if (s.display === "inline" || he.clientWidth === 0) continue;
    const overX = he.scrollWidth - he.clientWidth;
    const overY = he.scrollHeight - he.clientHeight;
    const hiddenX = s.overflowX !== "visible";
    const ellipsis = s.textOverflow === "ellipsis";
    if (overX > 1 && ellipsis && hiddenX) {
      const key = el.closest(KEY_TRUNC) !== null;
      add(key ? "truncatedKey" : "truncated", key ? "warn" : "info", el, `ellipsis hides ${overX} px of "${(el.textContent ?? "").trim().slice(0, 60)}"`, overX);
    } else if (overX > 1) {
      add(hiddenX ? "textClipped" : "textOverflow", "warn", el, `text ${hiddenX ? "clipped" : "spills"} by ${overX} px horizontally`, overX);
    } else if (overY > 2 && s.overflowY !== "visible" && s.overflowY !== "auto" && s.overflowY !== "scroll") {
      add("textClipped", "warn", el, `text clipped by ${overY} px vertically`, overY);
    }
  }

  // Wraps of short labels: the label itself, or the title line of a tile/row (its first
  // text leaf; hints and subtitles under it may wrap). The big round PTT may use 2 lines.
  const LABELS = 'button, [role="radio"], [role="tab"], .mk-badge, .mk-segment, [data-testid="route-chip"]';
  const leavesOf = (el: Element): Element[] => {
    const out: Element[] = [];
    const visit = (e: Element) => {
      if (!rendered(e)) return;
      const own = [...e.childNodes].some((n) => n.nodeType === Node.TEXT_NODE && (n.textContent ?? "").trim() !== "");
      if (own) out.push(e);
      for (const c of e.children) if (!own || cs(c).display !== "inline") visit(c);
    };
    visit(el);
    return out;
  };
  const labelEls = [...scope.querySelectorAll(LABELS)].filter(rendered);
  for (const el of labelEls) {
    const leaves = leavesOf(el);
    const leaf = leaves[0];
    if (leaf === undefined) continue;
    const { lines } = lineCount(leaves.length === 1 ? el : leaf);
    if (lines < 2) continue;
    const ptt = el.closest('.mk-ptt, [data-testid="mute-toggle"]') !== null;
    if (ptt && lines <= 2) continue;
    add("wrap", ptt ? "info" : "warn", leaves.length === 1 ? el : leaf, `label wraps to ${lines} lines`, lines);
  }
  // Squeezed text: 3+ lines of about one word each (a column crushed by its neighbours).
  for (const el of textEls) {
    if (cs(el).display === "inline" && el.parentElement !== null && textEls.includes(el.parentElement)) continue;
    const { lines, words } = lineCount(el);
    if (lines >= 3 && words / lines < 1.5) {
      add("squeezed", "warn", el, `${words} words on ${lines} lines (${Math.round((el as HTMLElement).clientWidth)} px wide column)`, lines);
    }
  }
  const HEADINGS = 'h1, h2, h3, [data-testid="heard-by"], .mk-banner-title, .mk-sheet-title';
  for (const el of [...scope.querySelectorAll(HEADINGS)].filter(rendered)) {
    const { lines, lastLineWords } = lineCount(el);
    if (lines >= 2 && lastLineWords === 1) add("orphan", "info", el, `heading of ${lines} lines ends with a single-word line`, lines);
  }
  {
    const range = document.createRange();
    const reported = new Set<Element>();
    for (const el of textEls) {
      for (const n of el.childNodes) {
        if (n.nodeType !== Node.TEXT_NODE) continue;
        const text = n.textContent ?? "";
        const re = /[\p{L}\p{N}]{3,}/gu;
        for (let m = re.exec(text); m !== null; m = re.exec(text)) {
          range.setStart(n, m.index);
          range.setEnd(n, m.index + m[0].length);
          const rs = [...range.getClientRects()].filter((x) => x.width > 0.5);
          if (rs.length < 2) continue;
          const tops = rs.map((x) => x.top);
          if (Math.max(...tops) - Math.min(...tops) > 4 && !reported.has(el)) {
            reported.add(el);
            add("midWordBreak", "warn", el, `"${m[0]}" is broken across ${rs.length} lines`, m[0].length);
          }
        }
      }
    }
  }

  let envTop: number | null = null;
  let envBottom: number | null = null;
  if (opts.probe === true) {
    const probe = document.createElement("div");
    probe.style.cssText =
      "position:fixed;left:0;top:0;width:1px;visibility:hidden;padding-top:env(safe-area-inset-top,0px);padding-bottom:env(safe-area-inset-bottom,0px)";
    document.body.appendChild(probe);
    const ps = getComputedStyle(probe);
    envTop = parseFloat(ps.paddingTop);
    envBottom = parseFloat(ps.paddingBottom);
    probe.remove();
  }

  return {
    problems,
    metrics: {
      vw,
      vh,
      docScrollWidth,
      docScrollHeight,
      bodyScrollHeight: body?.scrollHeight ?? null,
      bodyClientHeight: body?.clientHeight ?? null,
      envTop,
      envBottom,
      rootFontSize: parseFloat(getComputedStyle(doc).fontSize),
    },
  };
}

/**
 * Text whose font size does not follow the root font size (px sizes): it escapes the
 * textZoom emulation of this harness, while Android WebView textZoom scales it anyway.
 * Only the outermost escaping element of a subtree is listed.
 */
export function findPxFonts(): { selector: string; text: string; px: number }[] {
  const doc = document.documentElement;
  const els: Element[] = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  const seen = new Set<Element>();
  for (let n = walker.nextNode(); n !== null; n = walker.nextNode()) {
    const p = n.parentElement;
    if ((n.textContent ?? "").trim() === "" || p === null || seen.has(p)) continue;
    seen.add(p);
    const r = p.getBoundingClientRect();
    if (r.width > 0 && r.height > 0) els.push(p);
  }
  const before = els.map((e) => parseFloat(getComputedStyle(e).fontSize));
  const prev = doc.style.getPropertyValue("font-size");
  const prevPrio = doc.style.getPropertyPriority("font-size");
  doc.style.setProperty("font-size", "32px", "important");
  const after = els.map((e) => parseFloat(getComputedStyle(e).fontSize));
  doc.style.setProperty("font-size", prev, prevPrio);
  const escaping = new Set<Element>();
  els.forEach((e, i) => {
    const b = before[i] ?? 0;
    const a = after[i] ?? 0;
    if (b > 0 && a / b < 1.5) escaping.add(e);
  });
  const out: { selector: string; text: string; px: number }[] = [];
  els.forEach((e, i) => {
    if (!escaping.has(e)) return;
    for (let a = e.parentElement; a !== null; a = a.parentElement) if (escaping.has(a)) return;
    const cls = [...e.classList].slice(0, 3).join(".");
    const tid = e.closest("[data-testid]")?.getAttribute("data-testid");
    out.push({
      selector: `${tid !== undefined && tid !== null ? `[data-testid="${tid}"] ` : ""}${e.tagName.toLowerCase()}${cls !== "" ? "." + cls : ""}`,
      text: (e.textContent ?? "").trim().slice(0, 40),
      px: before[i] ?? 0,
    });
  });
  return out;
}
