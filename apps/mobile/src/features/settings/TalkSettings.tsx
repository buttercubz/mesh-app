// Settings → Talk and audio (WP-22, DESIGN §11.4, §1.4): default mode, PTT style, VOX
// sensitivity with the VOX test meter, built-in mic TX, route preference, keep screen on
// while mounted (car), haptics. Each change is saved and sent to the engine.
import type { PttStyle, RoutePreference, TalkMode, VoxSensitivity } from "@meshapp/engine-api";
import { SegmentedControl } from "@meshapp/ui-kit";

import { useT } from "../../i18n/index";
import { useSettingsStore } from "../../store/useSettingsStore";
import { useIntercomEngine } from "../group/engine";
import { runCommand } from "../group/session";
import { SettingRow, SettingsGroup, Switch } from "./controls";
import { VoxMeter } from "./VoxMeter";

export function TalkSettings() {
  const { t } = useT("settings");
  const { t: tc } = useT("common");
  const engine = useIntercomEngine();
  const s = useSettingsStore();
  const update = s.update;

  return (
    <SettingsGroup title={t("talk.title")}>
      <SettingRow label={t("talk.defaultMode")} description={t(`talk.defaultModeHint.${s.vehicle}`)} stacked>
        <SegmentedControl<TalkMode>
          label={t("talk.defaultMode")}
          value={s.defaultMode}
          onChange={(mode) => update({ defaultMode: mode })}
          options={(["vox", "ptt", "muted"] as const).map((m) => ({ value: m, label: tc(`mode.${m}`) }))}
        />
      </SettingRow>
      <SettingRow label={t("talk.pttStyle")} description={t("talk.pttStyleHint")} stacked>
        <SegmentedControl<PttStyle>
          label={t("talk.pttStyle")}
          value={s.pttStyle}
          onChange={(style) => {
            update({ pttStyle: style });
            void runCommand(engine, { type: "setPttStyle", style }, true);
          }}
          options={[
            { value: "hold", label: t("talk.hold") },
            { value: "latch", label: t("talk.latch") },
          ]}
        />
      </SettingRow>
      <SettingRow label={t("vox.sensitivity")} description={t("vox.sensitivityHint")} stacked>
        <div className="flex flex-col gap-3">
          <SegmentedControl<VoxSensitivity>
            label={t("vox.sensitivity")}
            value={s.voxSensitivity}
            onChange={(level) => {
              update({ voxSensitivity: level });
              void runCommand(engine, { type: "setVoxSensitivity", level }, true);
            }}
            options={(["low", "medium", "high"] as const).map((l) => ({ value: l, label: t(`vox.${l}`) }))}
          />
          <VoxMeter />
        </div>
      </SettingRow>
      <SettingRow label={t("talk.builtinMic")} description={t(`talk.builtinMicHint.${s.vehicle}`)}>
        <Switch
          label={t("talk.builtinMic")}
          checked={s.builtinMicTx}
          onChange={(allowed) => {
            update({ builtinMicTx: allowed });
            void runCommand(engine, { type: "setBuiltinMicTx", allowed }, true);
          }}
        />
      </SettingRow>
      <SettingRow label={t("talk.route")} description={t("talk.routeHint")} stacked>
        <SegmentedControl<RoutePreference>
          label={t("talk.route")}
          value={s.routePreference}
          onChange={(pref) => {
            update({ routePreference: pref });
            void runCommand(engine, { type: "setRoutePreference", pref }, true);
          }}
          options={(["auto", "headset", "speaker"] as const).map((p) => ({ value: p, label: t(`talk.route.${p}`) }))}
        />
      </SettingRow>
      {s.vehicle === "car" && (
        <SettingRow label={t("talk.keepScreenOn")} description={t("talk.keepScreenOnHint")} testId="keep-screen-on-row">
          <Switch
            label={t("talk.keepScreenOn")}
            checked={s.keepScreenOnMounted}
            onChange={(on) => update({ keepScreenOnMounted: on })}
          />
        </SettingRow>
      )}
      <SettingRow label={t("talk.haptics")} description={t("talk.hapticsHint")}>
        <Switch label={t("talk.haptics")} checked={s.haptics} onChange={(on) => update({ haptics: on })} />
      </SettingRow>
    </SettingsGroup>
  );
}
