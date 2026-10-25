// Bottom of the ride screen (WP-22, DESIGN §11.4): the 200 px PTT / Mute button (hold or
// latch in PTT; tap toggles mute in VOX; the ring shows the input level) and the
// glove-sized talk-mode control (VOX is never offered on the phone loudspeaker, §1.4).
// The audio route (and why VOX is off on the loudspeaker) lives in the ride status chip.
import type { TalkMode } from "@meshapp/engine-api";
import { BigPttButton, SegmentedControl } from "@meshapp/ui-kit";
import type { PttStatus } from "@meshapp/ui-kit";
import { Hand, Mic, MicOff } from "lucide-react";

import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { useSettingsStore } from "../../store/useSettingsStore";
import { useIntercomEngine, useSession } from "../group/engine";
import { runCommand } from "../group/session";
import { voxAllowed } from "./model";

function buzz(): void {
  if (!useSettingsStore.getState().haptics) return;
  // Haptics are a comfort: browsers without the Vibration API simply skip it.
  if (typeof navigator !== "undefined" && typeof navigator.vibrate === "function") navigator.vibrate(12);
}

function pttStatus(talk: string | undefined): PttStatus {
  if (talk === "talking") return "active";
  if (talk === "blocked") return "blocked";
  return "idle";
}

export function TalkControl() {
  const { t } = useT("ride");
  const engine = useIntercomEngine();
  const session = useSession();
  const route = useEngineStore((s) => s.snapshot?.audio.route ?? null);
  const inputLevel = useEngineStore((s) => s.snapshot?.audio.inputLevel ?? 0);
  const pttStyle = useSettingsStore((s) => s.pttStyle);
  const defaultMode = useSettingsStore((s) => s.defaultMode);

  if (session === null) return null;
  const mode = session.mode;
  const live = session.live;
  const canVox = voxAllowed(route);

  const setMode = (next: TalkMode) => {
    if (next === mode) return;
    void runCommand(engine, { type: "setTalkMode", mode: next });
  };

  const options = (["vox", "ptt", "muted"] as const).map((value) => ({
    value,
    label: t(`mode.${value}`),
    disabled: value === "vox" && !canVox,
  }));

  let button;
  if (mode === "ptt") {
    const status = pttStatus(session.talk);
    const label =
      status === "active" ? t("ptt.talking") : status === "blocked" ? t("ptt.blocked") : t(`ptt.${pttStyle}`);
    button = (
      <BigPttButton
        mode={pttStyle}
        status={status}
        level={inputLevel}
        disabled={!live}
        label={label}
        icon={status === "blocked" ? <Hand /> : <Mic />}
        ariaLabel={t("ptt.aria")}
        onDown={() => {
          buzz();
          engine.ptt(true).catch((e: unknown) => {
            // The engine stopped under the finger: the banners show why.
            console.warn("[ride] ptt down failed", e);
          });
        }}
        onUp={() => {
          buzz();
          engine.ptt(false).catch((e: unknown) => {
            // Same as above; release is idempotent in the engine.
            console.warn("[ride] ptt up failed", e);
          });
        }}
      />
    );
  } else {
    const muted = mode === "muted";
    // Unmuting returns to the rider's default mode (VOX unless the route forbids it).
    const preferred: TalkMode = defaultMode === "muted" ? "vox" : defaultMode;
    const unmuteTo: TalkMode = preferred === "vox" && !canVox ? "ptt" : preferred;
    const level = Math.min(1, Math.max(0, muted ? 0 : inputLevel));
    button = (
      <button
        type="button"
        className="mk-ptt"
        disabled={!live}
        aria-pressed={muted}
        aria-label={muted ? t("mute.unmuteAria") : t("mute.muteAria")}
        data-status={muted ? "muted" : session.talk === "talking" ? "active" : "idle"}
        data-testid="mute-toggle"
        onClick={() => {
          buzz();
          setMode(muted ? unmuteTo : "muted");
        }}
      >
        <svg className="mk-ptt-ring" viewBox="0 0 100 100" aria-hidden="true" focusable="false">
          <circle className="mk-ptt-ring-track" cx="50" cy="50" r="46" strokeWidth="8" />
          <circle
            className="mk-ptt-ring-level"
            cx="50"
            cy="50"
            r="46"
            strokeWidth="8"
            strokeDasharray={2 * Math.PI * 46}
            strokeDashoffset={2 * Math.PI * 46 * (1 - level)}
          />
        </svg>
        <span className="mk-ptt-label">
          <span className="mk-ptt-icon" aria-hidden="true">
            {muted ? <MicOff /> : <Mic />}
          </span>
          <span>{muted ? t("mute.muted") : t("mute.vox")}</span>
        </span>
      </button>
    );
  }

  return (
    <div className="mx-auto flex w-full max-w-md flex-col items-center gap-4" data-testid="talk-control" data-mode={mode}>
      {button}
      <SegmentedControl
        className="mk-segmented--ride w-full"
        label={t("mode.label")}
        options={options}
        value={mode}
        onChange={setMode}
        disabled={!live}
      />
    </div>
  );
}
