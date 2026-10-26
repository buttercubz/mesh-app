// Every screen and state the layout audit visits, in visiting order. A state's `setup`
// brings the app there from any other state (it never relies on the previous state), so a
// failing state does not cascade. Real flows (simulator) where they are quick and
// deterministic: onboarding, Join (the simulated riders' invite), Create → lobby with the
// natively rendered QR; crafted snapshots through the dev-only hook for the rest.
import type { Page } from "@playwright/test";
import type { Snapshot, TalkDeniedReason } from "@meshapp/engine-api";

import type { LayoutDevice, Locale } from "./devices";
import type { Hook } from "./page";
import { settle } from "./page";
import type { Craft } from "./snapshots";
import { DEFAULT_OTHERS, MANY_OTHERS, RIDER_NAMES, craft } from "./snapshots";

export interface StateContext {
  readonly page: Page;
  readonly hook: Hook;
  readonly device: LayoutDevice;
  readonly locale: Locale;
  /** Real snapshot of this phone's own group (set by the "create" state). */
  base: Snapshot | null;
  groupId: string | null;
}

export interface LayoutState {
  readonly screen: string;
  readonly state: string;
  /** The ride screen rules apply (no scroll, every key element visible). */
  readonly ride?: boolean;
  /** Part of the CI regression subset. */
  readonly ci?: boolean;
  /** Only on this platform (e.g. the Android-only Wi-Fi Aware section). */
  readonly platform?: LayoutDevice["platform"];
  setup(ctx: StateContext): Promise<void>;
  /** Re-run after every zoom change (scroll positions). */
  after?(ctx: StateContext): Promise<void>;
}

const LONG_GROUP = "Rodada a la Sierra Madre 2026";

const screenIs = async (ctx: StateContext, id: string) => {
  await ctx.page.locator(`[data-screen="${id}"]`).waitFor({ timeout: 15_000 });
};

/**
 * Creates this phone's group once per page (real createGroup → lobby) and keeps its
 * snapshot as the base of every crafted one.
 */
async function ensureGroup(ctx: StateContext): Promise<Snapshot> {
  if (ctx.base !== null) return ctx.base;
  const { hook, page } = ctx;
  await hook.unfreeze();
  await hook.go("home", {}, [["createGroup", {}]]);
  await screenIs(ctx, "createGroup");
  await page.getByLabel(await hook.t("group", "create.name"), { exact: true }).fill(LONG_GROUP);
  await page.getByTestId("create-confirm").click();
  await screenIs(ctx, "lobby");
  await page.getByTestId("lobby-screen").waitFor({ timeout: 20_000 });
  const snap = await hook.liveSnapshot();
  if (snap === null || snap.session === null) throw new Error("no session after createGroup");
  ctx.base = snap;
  ctx.groupId = snap.session.groupId;
  return snap;
}

/** Freezes a crafted snapshot and shows `stack` (home first). */
async function frozen(
  ctx: StateContext,
  c: Craft,
  stack: string[],
  opts: { denied?: TalkDeniedReason | null; status?: "idle" | "running"; compat?: "ok" | "mismatch"; settings?: Record<string, unknown> } = {},
) {
  const { hook } = ctx;
  const base = await ensureGroup(ctx);
  await hook.clearToasts();
  await hook.settings({ pttStyle: "hold", ...(opts.settings ?? {}) });
  await hook.freeze(craft(base, c), { status: opts.status ?? "running", compat: opts.compat ?? "ok" });
  await hook.talkDenied(opts.denied ?? null);
  const params = ctx.groupId === null ? {} : { groupId: ctx.groupId };
  const [first, ...rest] = stack;
  await hook.go(
    first ?? "home",
    {},
    rest.map((id) => [id, params] as [string, Record<string, string>]),
  );
  await screenIs(ctx, stack[stack.length - 1] ?? "home");
}

const ride = (c: Craft, opts: Parameters<typeof frozen>[3] = {}) => async (ctx: StateContext) => frozen(ctx, c, ["home", "ride"], opts);
const lobby = (c: Craft, opts: Parameters<typeof frozen>[3] = {}) => async (ctx: StateContext) =>
  frozen(ctx, { ride: "lobby", live: false, ...c }, ["home", "lobby"], opts);

async function scrollIntoView(page: Page, selector: string): Promise<void> {
  await page.locator(selector).first().evaluate((el) => el.scrollIntoView({ block: "center" }));
}

async function scrollBodyToEnd(page: Page): Promise<void> {
  await page.evaluate(() => {
    const body = (document.querySelector("main [data-screen-body]") ?? document.querySelector("main > div.h-dvh > div.overflow-y-auto"));
    if (body !== null) body.scrollTop = body.scrollHeight;
  });
}

async function onboardingTo(ctx: StateContext, step: number): Promise<void> {
  const { page, hook } = ctx;
  await hook.unfreeze();
  await hook.settings({ onboardingDone: false, displayName: "Maximiliano R." });
  await hook.go("home", {}, [["onboarding", {}]]);
  await page.locator('[data-testid="onboarding"][data-step="profile"]').waitFor();
  if (step >= 1) {
    // The step's submit button may sit in the pinned footer (tied to the form by `form`).
    await page.locator('[data-screen="onboarding"] button[type="submit"]').click();
    await page.locator('[data-testid="onboarding"][data-step="permissions"]').waitFor();
  }
  if (step >= 2) {
    await page.getByRole("button", { name: await hook.t("onboarding", "permissions.later"), exact: true }).click();
    await page.locator('[data-testid="onboarding"][data-step="headset"]').waitFor();
  }
  if (step >= 3) {
    await page.getByRole("button", { name: await hook.t("onboarding", "headset.skip"), exact: true }).click();
    await page.locator('[data-testid="onboarding"][data-step="ready"]').waitFor();
  }
}

const SAVED_GROUPS = [
  { id: "a".repeat(32), name: LONG_GROUP, role: "leader" as const, lastUsed: 3 },
  { id: "b".repeat(32), name: "Moteros del Bajío — salida nocturna", role: "member" as const, lastUsed: 2 },
  { id: "c".repeat(32), name: "Viaje", role: "member" as const, lastUsed: 1 },
];

/** All banner kinds that can be active together: primary + "+4" in the chip. */
const WORST: Craft = { bluetooth: "off", ladder: "L3", live: true, txMutedNoHeadset: true, islands: 3, talk: "blocked" };
const TALKERS_4 = ["me", RIDER_NAMES[1], RIDER_NAMES[2], RIDER_NAMES[4]];
const WEAK_OTHERS = DEFAULT_OTHERS.map((o) => ({ ...o, quality: 40 }));
const LOST_OTHERS = DEFAULT_OTHERS.map((o) => ({ ...o, status: "lost" as const, hearsMe: false, voice: "no" as const }));

async function toastsWithBanner(ctx: StateContext): Promise<void> {
  await ride(WORST, { denied: "cap" })(ctx);
  const t = (k: string, p: Record<string, unknown> = {}) => ctx.hook.t("notices", k, p);
  await ctx.hook.toast({ title: await t("memberLost", { n: 3, names: `${RIDER_NAMES[0]}, ${RIDER_NAMES[1]}, ${RIDER_NAMES[3]}` }), tone: "warn", durationMs: 0, key: "a" });
  await ctx.hook.toast({ title: await t("captureLost"), tone: "danger", durationMs: 0, key: "b" });
  await ctx.hook.toast({ title: await t("linkLimitReached"), tone: "warn", durationMs: 0, key: "c" });
}

export const STATES: readonly LayoutState[] = [
  // ---- first launch ---------------------------------------------------------------
  {
    screen: "welcome",
    state: "default",
    async setup({ hook, page }) {
      await hook.unfreeze();
      await hook.settings({ onboardingDone: false });
      await hook.go("home");
      await page.getByTestId("welcome").waitFor();
    },
  },
  { screen: "onboarding", state: "1-profile", setup: (ctx) => onboardingTo(ctx, 0) },
  { screen: "onboarding", state: "2-permissions", setup: (ctx) => onboardingTo(ctx, 1) },
  { screen: "onboarding", state: "3-headset", setup: (ctx) => onboardingTo(ctx, 2) },
  { screen: "onboarding", state: "4-ready", setup: (ctx) => onboardingTo(ctx, 3) },

  // ---- home -------------------------------------------------------------------------
  {
    screen: "home",
    state: "saved-groups",
    ci: true,
    async setup({ hook, page }) {
      await hook.unfreeze();
      await hook.settings({ onboardingDone: true, displayName: "Maximiliano R." });
      await hook.go("home");
      await page.getByTestId("home-screen").waitFor();
      await hook.groups(SAVED_GROUPS);
    },
  },
  {
    screen: "home",
    state: "open-channel",
    async setup({ hook, page }) {
      await hook.unfreeze();
      await hook.go("home");
      await page.getByTestId("home-screen").waitFor();
      await hook.groups(SAVED_GROUPS);
    },
    after: ({ page }) => scrollIntoView(page, '[data-testid="open-channel"]'),
  },
  {
    screen: "home",
    state: "scrolled-end",
    async setup({ hook, page }) {
      await hook.unfreeze();
      await hook.go("home");
      await page.getByTestId("home-screen").waitFor();
      await hook.groups(SAVED_GROUPS);
    },
    after: ({ page }) => scrollBodyToEnd(page),
  },
  {
    screen: "home",
    state: "empty",
    async setup({ hook, page }) {
      await hook.unfreeze();
      await hook.go("home");
      await page.getByTestId("home-screen").waitFor();
      await hook.groups([]);
    },
  },

  // ---- join (real invite of the simulated riders) --------------------------------------
  {
    screen: "join",
    state: "confirm",
    ci: true,
    async setup(ctx) {
      const { hook, page } = ctx;
      await hook.unfreeze();
      await hook.go("home");
      await page.getByTestId("home-screen").waitFor();
      await page.getByRole("button", { name: await hook.t("group", "home.join"), exact: true }).click();
      await screenIs(ctx, "join");
      await page.locator('[data-testid="join-group-name"]:not(:empty)').waitFor({ timeout: 20_000 });
    },
  },

  // ---- settings and lab (no session yet) ---------------------------------------------------
  {
    screen: "settings",
    state: "top",
    ci: true,
    async setup(ctx) {
      await ctx.hook.unfreeze();
      await ctx.hook.aware(ctx.device.platform === "android" ? { supported: true, enabled: false } : null);
      await ctx.hook.go("home", {}, [["settings", {}]]);
      await ctx.page.getByTestId("settings-screen").waitFor();
    },
  },
  {
    screen: "settings",
    state: "talk",
    async setup(ctx) {
      await ctx.hook.unfreeze();
      await ctx.hook.go("home", {}, [["settings", {}]]);
      await ctx.page.getByTestId("settings-screen").waitFor();
    },
    after: ({ page }) => scrollIntoView(page, '[data-testid="vox-meter"], [data-testid="settings-screen"] section:nth-of-type(2)'),
  },
  {
    screen: "settings",
    state: "aware-beta",
    platform: "android",
    async setup(ctx) {
      await ctx.hook.unfreeze();
      await ctx.hook.aware({ supported: true, enabled: false });
      await ctx.hook.go("home", {}, [["settings", {}]]);
      await ctx.page.getByTestId("aware-row").waitFor({ timeout: 10_000 });
    },
    after: ({ page }) => scrollIntoView(page, '[data-testid="aware-row"]'),
  },
  {
    screen: "settings",
    state: "diagnostics",
    async setup(ctx) {
      await ctx.hook.unfreeze();
      await ctx.hook.go("home", {}, [["settings", {}]]);
      await ctx.page.getByTestId("diag-usage").waitFor({ timeout: 10_000 });
    },
    after: ({ page }) => scrollIntoView(page, '[data-testid="diag-usage"]'),
  },
  {
    screen: "settings",
    state: "scrolled-end",
    async setup(ctx) {
      await ctx.hook.unfreeze();
      await ctx.hook.go("home", {}, [["settings", {}]]);
      await ctx.page.getByTestId("settings-screen").waitFor();
    },
    after: ({ page }) => scrollBodyToEnd(page),
  },
  {
    screen: "fieldTest",
    state: "entry",
    async setup(ctx) {
      await ctx.hook.unfreeze();
      await ctx.hook.go("home", {}, [["settings", {}], ["fieldTest", {}]]);
      await ctx.page.getByTestId("field-test").waitFor();
    },
  },

  // ---- create → lobby (real group, real QR) -------------------------------------------------
  {
    screen: "createGroup",
    state: "filled",
    ci: true,
    async setup(ctx) {
      const { hook, page } = ctx;
      await hook.unfreeze();
      await hook.go("home", {}, [["createGroup", {}]]);
      await screenIs(ctx, "createGroup");
      await page.getByLabel(await hook.t("group", "create.name"), { exact: true }).fill(LONG_GROUP);
    },
  },
  {
    screen: "lobby",
    state: "leader-not-live",
    ci: true,
    async setup(ctx) {
      // Deterministic from here: two riders ready, this phone not live yet.
      await lobby({ others: DEFAULT_OTHERS.slice(0, 2) })(ctx);
    },
  },
  {
    screen: "lobby",
    state: "qr-shown",
    async setup(ctx) {
      await lobby({ others: DEFAULT_OTHERS.slice(0, 2) })(ctx);
      await ctx.page.locator('[data-testid="invite-qr"] button').last().click();
      await ctx.page.getByTestId("invite-qr-image").waitFor({ timeout: 15_000 });
    },
    after: ({ page }) => scrollIntoView(page, '[data-testid="invite-qr-image"]'),
  },
  { screen: "lobby", state: "live-many-riders", ci: true, setup: lobby({ live: true, others: MANY_OTHERS }) },
  {
    screen: "lobby",
    state: "ready-list",
    setup: lobby({ live: true, others: MANY_OTHERS }),
    after: ({ page }) => scrollIntoView(page, '[data-testid="ready-list"]'),
  },
  { screen: "lobby", state: "member-waiting", setup: lobby({ role: "member", others: DEFAULT_OTHERS }) },
  { screen: "lobby", state: "banners", setup: lobby({ bluetooth: "off", others: LOST_OTHERS, live: true, ladder: "L2" }) },

  // ---- ride -------------------------------------------------------------------------------
  { screen: "ride", state: "no-banner", ride: true, ci: true, setup: ride({}) },
  { screen: "ride", state: "speaker-ptt", ride: true, setup: ride({ route: "builtinSpeaker" }) },
  { screen: "ride", state: "banner-bluetoothOff", ride: true, setup: ride({ bluetooth: "off" }) },
  { screen: "ride", state: "banner-bluetoothDenied", ride: true, setup: ride({ bluetooth: "unauthorized" }) },
  { screen: "ride", state: "banner-ladderL1", ride: true, setup: ride({ ladder: "L1" }) },
  { screen: "ride", state: "banner-ladderL2", ride: true, setup: ride({ ladder: "L2" }) },
  { screen: "ride", state: "banner-ladderL3", ride: true, setup: ride({ ladder: "L3" }) },
  { screen: "ride", state: "banner-micPaused", ride: true, setup: ride({ captureOk: false }) },
  { screen: "ride", state: "banner-notLive", ride: true, ci: true, setup: ride({ live: false }) },
  { screen: "ride", state: "banner-txMutedNoHeadset", ride: true, setup: ride({ txMutedNoHeadset: true, route: "builtinReceiver" }) },
  { screen: "ride", state: "banner-alone", ride: true, setup: ride({ others: LOST_OTHERS }) },
  { screen: "ride", state: "banner-island", ride: true, setup: ride({ islands: 2 }) },
  { screen: "ride", state: "banner-busy", ride: true, setup: ride({ talk: "blocked", talkers: TALKERS_4.slice(1, 3) }, { denied: "cap" }) },
  { screen: "ride", state: "banner-preempted", ride: true, setup: ride({ talk: "blocked" }, { denied: "preempted" }) },
  { screen: "ride", state: "banner-congestion", ride: true, setup: ride({ talk: "blocked" }, { denied: "congestion" }) },
  { screen: "ride", state: "banner-weakSignal", ride: true, setup: ride({ talk: "blocked", others: WEAK_OTHERS }, { denied: "congestion" }) },
  { screen: "ride", state: "worst-stacking", ride: true, ci: true, setup: ride(WORST, { denied: "cap" }) },
  {
    screen: "ride",
    state: "worst-details-open",
    ride: true,
    ci: true,
    async setup(ctx) {
      await ride(WORST, { denied: "cap" })(ctx);
      await ctx.page.getByTestId("ride-status-chip").click();
      await ctx.page.getByTestId("ride-status-details").waitFor();
    },
  },
  {
    screen: "ride",
    state: "details-open",
    ride: true,
    async setup(ctx) {
      await ride({ route: "builtinSpeaker" })(ctx);
      await ctx.page.getByTestId("ride-status-chip").click();
      await ctx.page.getByTestId("ride-status-details").waitFor();
    },
  },
  { screen: "ride", state: "talkers-2", ride: true, setup: ride({ talkers: TALKERS_4.slice(1, 3) }) },
  { screen: "ride", state: "talkers-4", ride: true, setup: ride({ talkers: TALKERS_4.slice(1).concat([RIDER_NAMES[0]]) }) },
  { screen: "ride", state: "worst-talkers-4", ride: true, ci: true, setup: ride({ ...WORST, talkers: TALKERS_4 }, { denied: "cap" }) },
  { screen: "ride", state: "vox", ride: true, ci: true, setup: ride({ mode: "vox", talk: "talking", talkers: ["me"] }) },
  { screen: "ride", state: "muted", ride: true, setup: ride({ mode: "muted", talk: "muted" }) },
  {
    screen: "ride",
    state: "ptt-latched-talking",
    ride: true,
    setup: ride({ talk: "talking", talkers: ["me"] }, { settings: { pttStyle: "latch" } }),
  },
  {
    screen: "ride",
    state: "open-channel-worst",
    ride: true,
    setup: ride({ ...WORST, openChannel: 7, others: DEFAULT_OTHERS }, { denied: "cap" }),
  },
  { screen: "ride", state: "toasts-with-banner", ride: true, ci: true, setup: toastsWithBanner },
  {
    screen: "ride",
    state: "leave-sheet",
    ride: true,
    async setup(ctx) {
      await ride({}, {})(ctx);
      await ctx.page.locator("header button").first().click();
      await ctx.page.getByTestId("leave-sheet").waitFor();
    },
  },
  {
    screen: "ride",
    state: "engine-banner",
    ride: true,
    setup: ride({ engine: "crashed" }, { compat: "mismatch" }),
  },
  { screen: "ride", state: "session-gone", setup: ride({}, { status: "idle" }) },

  // ---- members ----------------------------------------------------------------------------
  {
    screen: "members",
    state: "leader-many",
    ci: true,
    setup: (ctx) => frozen(ctx, { others: MANY_OTHERS }, ["home", "ride", "members"]),
  },
  {
    screen: "members",
    state: "leader-tools",
    setup: (ctx) => frozen(ctx, { others: MANY_OTHERS }, ["home", "ride", "members"]),
    after: ({ page }) => scrollBodyToEnd(page),
  },
  {
    screen: "members",
    state: "member-view",
    setup: (ctx) => frozen(ctx, { role: "member", others: MANY_OTHERS, shareDistance: true }, ["home", "ride", "members"]),
  },
  {
    screen: "members",
    state: "open-channel-blocked-muted",
    setup: (ctx) => frozen(ctx, { role: "member", openChannel: 4, others: MANY_OTHERS }, ["home", "ride", "members"]),
  },
  {
    screen: "members",
    state: "leader-sheet",
    async setup(ctx) {
      await frozen(ctx, { others: MANY_OTHERS }, ["home", "ride", "members"]);
      await ctx.page.locator(`[data-member="${RIDER_NAMES[1]}"] button`).first().click();
      await ctx.page.locator('[role="dialog"]').waitFor();
    },
  },

  // ---- home with a session, shell banners -------------------------------------------------------
  { screen: "home", state: "active-ride", setup: (ctx) => frozen(ctx, {}, ["home"]) },
  { screen: "home", state: "engine-crashed", setup: (ctx) => frozen(ctx, { engine: "crashed" }, ["home"]) },
];

/** Waits for the screen after a zoom change and re-applies scroll positions. */
export async function afterZoom(ctx: StateContext, s: LayoutState): Promise<void> {
  await settle(ctx.page, 0);
  if (s.after !== undefined) await s.after(ctx);
  await settle(ctx.page);
}
