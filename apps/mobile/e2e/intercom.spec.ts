// Intercom e2e (WP-22, DESIGN §11.4, §1.3, §13.1): the product screens against the
// browser simulator (the real Rust core in mesh_wasm.wasm, through the plugin's web
// implementation). Create → QR → the simulated riders join → Go live → Start ride →
// PTT talking → leader actions → busy banner as a third talker → a rider drops out
// (island banner) and rejoins (banner clears); the Join path in Spanish; onboarding.
import type { ConsoleMessage, Page } from "@playwright/test";
import { expect, test } from "@playwright/test";

/** A small world that forms fast: this phone plus two simulated riders, all in range. */
const SIM = "/?sim=pair&nodes=3&botsTalk=0&seed=7&simDebug=1";

function collectErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
  page.on("console", (m: ConsoleMessage) => {
    if (m.type() === "error") errors.push(`console: ${m.text()}`);
  });
  return errors;
}

/** Runs a simulator command (`sim.*` or C1) and returns its JSON result. */
async function sim(page: Page, command: Record<string, unknown>): Promise<Record<string, unknown>> {
  return page.evaluate((c) => {
    // `?simDebug=1`: the plugin's supported dev handle on the simulator backend.
    const backend = (globalThis as { __meshSim?: { convoy?: { run(x: unknown): unknown } } }).__meshSim;
    if (backend?.convoy === undefined) throw new Error("simulator handle missing (?simDebug=1)");
    return backend.convoy.run(c) as Record<string, unknown>;
  }, command);
}

interface BotInfo {
  index: number;
  nodeId: number | null;
  name: string;
}

async function bots(page: Page): Promise<BotInfo[]> {
  return (await sim(page, { type: "sim.info" }))["bots"] as BotInfo[];
}

/** A rider who already finished onboarding (name, vehicle, language). */
async function seedRider(page: Page, language: "en" | "es", vehicle: "moto" | "car" = "moto"): Promise<void> {
  await page.addInitScript(
    ({ language, vehicle }) => {
      if (localStorage.getItem("meshapp.settings") !== null) return;
      const state = { language, vehicle, displayName: "Me", onboardingDone: true, theme: "dark" };
      localStorage.setItem("meshapp.settings", JSON.stringify({ state, version: 1 }));
    },
    { language, vehicle },
  );
}

const screen = (page: Page) => page.locator("[data-screen]");

test.describe("Intercom ride (English)", () => {
  test.use({ locale: "en-US" });

  test("create → QR → riders join → Go live → ride → PTT → busy → island clears", async ({ page }) => {
    test.setTimeout(180_000);
    const errors = collectErrors(page);
    await seedRider(page, "en");
    await page.goto(SIM);
    await expect(screen(page)).toHaveAttribute("data-screen", "home");
    await expect(screen(page)).toHaveAttribute("data-engine-kind", "full");

    // Create group → lobby.
    await page.getByRole("button", { name: "Create group" }).click();
    await page.getByLabel("Group name").fill("Sunday ride");
    await page.getByTestId("create-confirm").click();
    await expect(screen(page)).toHaveAttribute("data-screen", "lobby", { timeout: 15_000 });
    await expect(page.getByRole("heading", { level: 1 })).toHaveText("Sunday ride");

    // The natively rendered QR, with its warning; hidden again on demand.
    await page.getByRole("button", { name: "Show QR code" }).click();
    const qr = page.getByTestId("invite-qr-image");
    await expect(qr).toHaveAttribute("src", /^data:image\/png;base64,/);
    await expect(page.getByText("A screenshot of this code lets anyone join.")).toBeVisible();
    await page.getByRole("button", { name: "Hide" }).click();
    await expect(qr).toBeHidden();

    // The simulated riders scan the invite and show up with ready checks.
    await expect(page.getByTestId("lobby-reach")).toHaveText("Voice reaches 2 of 2", { timeout: 40_000 });
    await expect(page.getByTestId("ready-list").locator("[data-member]")).toHaveCount(3);

    // Go live (★, from the tap): capture starts muted and the screen says so.
    await page.getByTestId("go-live").click();
    await expect(page.getByTestId("mic-muted-status")).toHaveText(/Mic ready, muted until the ride starts/);

    await page.getByTestId("start-ride").click();
    await expect(screen(page)).toHaveAttribute("data-screen", "ride", { timeout: 15_000 });
    await expect(page.getByTestId("heard-by")).toHaveText(/Heard by \d of 2/);

    // No VOX on the phone loudspeaker (the simulator's route): the ride runs in PTT.
    const control = page.getByTestId("talk-control");
    await expect(control).toHaveAttribute("data-mode", "ptt", { timeout: 10_000 });
    await expect(page.getByTestId("route-chip")).toHaveText("Phone speaker");
    await expect(control.getByRole("radio", { name: "VOX" })).toBeDisabled();

    // PTT: hold to talk.
    const ptt = page.getByRole("button", { name: "Push to talk" });
    await ptt.hover();
    await page.mouse.down();
    await expect(ptt).toHaveAttribute("data-status", "active", { timeout: 5_000 });
    await expect(page.getByTestId("talker-chip")).toContainText("You");
    await page.mouse.up();
    await expect(ptt).toHaveAttribute("data-status", "idle", { timeout: 5_000 });

    // Members: the leader locks the group (access-loss preview first), then hands over.
    await page.getByRole("button", { name: "Riders" }).click();
    await expect(screen(page)).toHaveAttribute("data-screen", "members");
    await page.getByRole("button", { name: "Lock group" }).click();
    const lock = page.getByTestId("leader-sheet-lock");
    await expect(lock).toContainText("Every rider is confirmed");
    await lock.getByTestId("leader-confirm").click();
    await expect(lock).toBeHidden();
    const [first] = await bots(page);
    await page.locator(`[data-member="${first?.name ?? ""}"] button`).click();
    await page.getByRole("button", { name: "Make leader" }).click();
    await page.getByTestId("leader-sheet-transfer").getByTestId("leader-confirm").click();
    await expect(page.getByText("Only the leader can remove riders")).toBeVisible({ timeout: 10_000 });
    await page.getByRole("button", { name: "Back" }).click();
    await expect(screen(page)).toHaveAttribute("data-screen", "ride");

    // Third talker: the other riders hold the channel, PTT is refused with the busy banner.
    const riders = await bots(page);
    await sim(page, { type: "sim.botTalk", bot: riders[0]?.index, ms: 6_000 });
    await sim(page, { type: "sim.botTalk", bot: riders[1]?.index, ms: 6_000 });
    await expect(page.getByTestId("talker-chip").first()).toBeVisible({ timeout: 5_000 });
    await ptt.hover();
    await page.mouse.down();
    const busy = page.locator('[data-banner="busy"]');
    await expect(busy).toContainText(/Channel full — \d talking/, { timeout: 5_000 });
    await page.mouse.up();

    // A rider rides out of range: island banner; back in range: it clears by itself.
    const away = riders[riders.length - 1];
    await sim(page, { type: "sim.botAway", bot: away?.index, away: true });
    const island = page.locator('[data-banner="island"], [data-banner="alone"]');
    await expect(island).toBeVisible({ timeout: 30_000 });
    await sim(page, { type: "sim.botAway", bot: away?.index, away: false });
    await expect(island).toBeHidden({ timeout: 60_000 });

    expect(errors).toEqual([]);
  });
});

test.describe("Join a ride (Spanish)", () => {
  test.use({ locale: "es-ES" });

  test("scan the invite → confirm → lobby → start here → En vivo from the banner", async ({ page }) => {
    test.setTimeout(90_000);
    const errors = collectErrors(page);
    await seedRider(page, "es");
    await page.goto(SIM);
    await expect(screen(page)).toHaveAttribute("data-screen", "home");
    await expect(page.locator("html")).toHaveAttribute("lang", "es");

    // The browser's "scanner" returns the simulated riders' invite.
    await page.getByRole("button", { name: "Unirse a un grupo" }).click();
    await expect(screen(page)).toHaveAttribute("data-screen", "join");
    await expect(page.getByText("¿Unirse a este grupo?")).toBeVisible();
    await expect(page.getByTestId("join-group-name")).not.toBeEmpty();
    await page.getByTestId("join-confirm").click();

    // Lobby as a member: the riders already ride, so this late joiner starts locally,
    // then goes live from the ride banner.
    await expect(screen(page)).toHaveAttribute("data-screen", "lobby", { timeout: 30_000 });
    await expect(page.getByText(/Esperando que el líder|El líder está fuera de alcance/)).toBeVisible();
    await page.getByTestId("start-ride").click();
    await expect(screen(page)).toHaveAttribute("data-screen", "ride", { timeout: 15_000 });
    const notLive = page.locator('[data-banner="notLive"]');
    await expect(notLive).toContainText("Micrófono apagado");
    await notLive.getByRole("button", { name: "En vivo" }).click();
    await expect(notLive).toBeHidden({ timeout: 10_000 });
    await expect(page.getByTestId("talk-control").getByRole("radiogroup", { name: "Modo de habla" })).toBeVisible();
    await expect(page.getByTestId("heard-by")).toHaveText(/Lo oyen \d de 2/, { timeout: 20_000 });

    // Leave from the Android-style back affordance: confirm sheet, back Home.
    await page.getByRole("button", { name: "Atrás" }).click();
    await page.getByRole("button", { name: "Salir de la rodada" }).click();
    await expect(screen(page)).toHaveAttribute("data-screen", "home");
    expect(errors).toEqual([]);
  });
});

test.describe("First launch (English)", () => {
  test.use({ locale: "en-US" });

  test("onboarding: language, name, car, permissions, distance, headset check, honest promise", async ({ page }) => {
    test.setTimeout(60_000);
    const errors = collectErrors(page);
    await page.goto(SIM);
    await expect(page.getByTestId("welcome")).toBeVisible();
    await page.getByRole("button", { name: "Get started" }).click();
    await expect(screen(page)).toHaveAttribute("data-screen", "onboarding");
    await page.getByRole("button", { name: "English" }).click();
    await page.getByLabel("Your name").fill("Erick");
    await page.getByRole("radio", { name: "Car" }).click();
    await page.getByRole("button", { name: "Continue" }).click();
    await page.getByRole("button", { name: "Not now" }).click();
    await page.getByRole("button", { name: "Not now" }).click();

    // Headset check: a ★ audioCheck from the tap; the browser has no engine clock
    // before a group starts, so only the request is checked here (samples: Vitest).
    await page.getByTestId("check-tone").click();
    await expect(page.getByTestId("check-tone")).toHaveText("Stop");
    await page.getByTestId("check-tone").click();
    await page.getByRole("button", { name: /Skip for now|Continue/ }).click();

    await expect(page.getByTestId("honest-promise")).toContainText("clear voice up to 2 relays, further is best effort");
    await expect(page.getByText("Do not swipe the app away during a ride")).toBeVisible();
    await page.getByTestId("onboarding-finish").click();
    await expect(page.getByTestId("home-screen")).toBeVisible();

    // Car profile: PTT by default and the screen kept on while mounted.
    await page.getByRole("button", { name: "Settings" }).click();
    await expect(page.getByRole("switch", { name: "Keep screen on while mounted" })).toHaveAttribute("aria-checked", "true");
    await expect(
      page.getByRole("radiogroup", { name: "Mode when the ride starts" }).getByRole("radio", { name: "Push to talk" }),
    ).toHaveAttribute("aria-checked", "true");
    expect(errors).toEqual([]);
  });
});
