// WP-22 catalogs (acceptance "ES/EN parity incl. all notice keys"): the onboarding,
// group, ride and settings namespaces are registered with ES/EN parity, every key the
// screens build from engine enums exists in both languages, and every NoticeKey has
// text in both languages.
import type { Member } from "@meshapp/engine-api";
import {
  AUDIO_CHECK_PHASES,
  AUDIO_ROUTE_KINDS,
  MEMBER_FLAGS,
  MEMBER_STATUSES,
  NOTICE_KEYS,
  PEER_PLATFORMS,
  TALK_DENIED_REASONS,
  VOICE_REACHES,
} from "@meshapp/engine-api";
import { describe, expect, it } from "vitest";

import { APP_CATALOGS } from "../../app/i18n";
import type { AppLanguage } from "../../i18n/index";
import { catalogParityErrors, catalogTexts, registeredNamespaces } from "../../i18n/index";
import { statusLabel } from "../members/format";
import { ONBOARDING_PERMISSIONS } from "../onboarding/permissions";
import type { BannerAction, BannerKind } from "../ride/model";
import { compassKey, routeKey } from "../ride/model";

const LANGS: AppLanguage[] = ["en", "es"];
const OWN = ["onboarding", "group", "ride", "settings"] as const;

function expectKeys(namespace: string, keys: readonly string[]): void {
  for (const lang of LANGS) {
    const texts = catalogTexts(namespace, lang);
    const missing = keys.filter((k) => typeof texts[k] !== "string" || texts[k] === "");
    expect(missing, `${namespace}/${lang}`).toEqual([]);
  }
}

describe("WP-22 catalogs", () => {
  it("are registered with ES/EN parity", () => {
    for (const ns of OWN) {
      expect(registeredNamespaces()).toContain(ns);
      const catalog = APP_CATALOGS[ns];
      expect(catalog, ns).toBeDefined();
      if (catalog !== undefined) expect(catalogParityErrors(catalog), ns).toEqual([]);
    }
  });

  it("cover every banner, route and talk-denied variant of the ride screen", () => {
    const banners: BannerKind[] = [
      "bluetoothOff",
      "bluetoothDenied",
      "ladderL1",
      "ladderL2",
      "ladderL3",
      "micPaused",
      "notLive",
      "txMutedNoHeadset",
      "alone",
      "island",
      "busy",
      ...TALK_DENIED_REASONS.filter((r) => r !== "cap"),
    ];
    const actions: BannerAction[] = ["resumeMic", "goLive", "openSettings"];
    expectKeys("ride", [
      ...banners.flatMap((b) => [`banner.${b}.title`, `banner.${b}.message`]),
      ...actions.map((a) => `banner.action.${a}`),
      ...AUDIO_ROUTE_KINDS.map(routeKey),
      routeKey(null),
      ...["vox", "ptt", "muted"].map((m) => `mode.${m}`),
    ]);
  });

  it("cover every member status, voice reach, flag, platform and compass point", () => {
    const base = { hops: 2, lastHeardMs: 40_000 } as Member;
    expectKeys("group", [
      ...MEMBER_STATUSES.map((status) => statusLabel({ ...base, status }).key),
      ...VOICE_REACHES.map((v) => `member.voice.${v}`),
      ...MEMBER_FLAGS.map((f) => `member.flag.${f}`),
      ...PEER_PLATFORMS.map((p) => `member.platform.${p}`),
      ...[0, 45, 90, 135, 180, 225, 270, 315].map(compassKey),
    ]);
  });

  it("cover every audio-check phase and onboarding permission", () => {
    expectKeys("onboarding", [
      ...AUDIO_CHECK_PHASES.map((p) => `check.phase.${p}`),
      ...ONBOARDING_PERMISSIONS.flatMap((p) => [`permissions.${p}.title`, `permissions.${p}.reason`]),
    ]);
  });

  it("every engine notice has text in both languages", () => {
    expectKeys("notices", [...NOTICE_KEYS]);
  });

  it("keeps the honest promise in both languages", () => {
    expect(catalogTexts("onboarding", "en")["promise.relays"]).toContain("clear voice up to 2 relays");
    expect(catalogTexts("onboarding", "es")["promise.relays"]).toContain("voz clara hasta 2 retransmisiones");
    for (const lang of LANGS) {
      const all = OWN.flatMap((ns) => Object.values(catalogTexts(ns, lang))).join("\n");
      // No helmet-brand trademarks in product copy (DESIGN §0.2).
      expect(all).not.toMatch(/\b(Sena|Cardo)\b/);
    }
  });
});
