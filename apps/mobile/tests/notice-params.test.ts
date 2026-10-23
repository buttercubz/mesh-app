// Every notice template interpolates with exactly the params mesh-core sends (WP-22
// report: toasts showed raw "{{name}} joined"). The table mirrors the core call sites:
// session/view.rs notice_n {n, names}; session/ride.rs {n}; view.rs partition {islands};
// group/run.rs + apply.rs {group} / {name}; lab {labNode}. batteryLow is not emitted by
// mesh-core today; it carries {percent} per the C1 doc (mesh-api keys.rs).
import type { NoticeKey, ParamValue } from "@meshapp/engine-api";
import { NOTICE_KEYS } from "@meshapp/engine-api";
import { describe, expect, it } from "vitest";

import { noticeToast } from "../src/engine/notices";
import { applyLanguage } from "../src/i18n/index";

const CORE_PARAMS: Partial<Record<NoticeKey, Record<string, ParamValue>>> = {
  memberJoined: { n: 2, names: "Ana, Luis" },
  memberLeft: { n: 1, names: "Ana" },
  memberLost: { n: 2, names: "Ana, Luis" },
  memberBack: { n: 1, names: "Luis" },
  partition: { islands: 2 },
  removed: { group: "Sierra" },
  groupClosed: { group: "Sierra" },
  inviteLeaderMismatch: { group: "Sierra" },
  rideStarted: { n: 4 },
  leaderChanged: { name: "Ana" },
  batteryLow: { percent: 14 },
  labPeerJoined: { labNode: 3 },
  labPeerLost: { labNode: 3 },
};

/** Notices about the current group whose text names it implicitly ("the group"). */
const GROUP_IMPLICIT: ReadonlySet<NoticeKey> = new Set<NoticeKey>(["groupClosed", "inviteLeaderMismatch"]);

describe("notice templates match the params mesh-core sends", () => {
  for (const lang of ["en", "es"] as const) {
    it(`${lang}: no raw placeholder remains in any notice`, () => {
      applyLanguage(lang);
      for (const key of NOTICE_KEYS) {
        const title = noticeToast(key, CORE_PARAMS[key] ?? {}).title;
        expect(title, key).not.toContain("{{");
        expect(title, key).not.toBe(key);
        // Names travel as strings: every name the core sends must reach the rider.
        for (const v of Object.values(CORE_PARAMS[key] ?? {})) {
          if (typeof v === "string" && !GROUP_IMPLICIT.has(key)) expect(title, key).toContain(v);
        }
      }
      applyLanguage("en");
    });
  }
});
