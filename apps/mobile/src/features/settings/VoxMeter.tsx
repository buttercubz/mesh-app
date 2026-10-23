// VOX test meter of Settings (WP-22, DESIGN §11.4, §7.3): `audioCheck{voxMeter}` (★,
// from the tap) streams the input level and the VAD gate at the current sensitivity, so
// the rider can see whether speech (and not wind) opens VOX.
import { Button } from "@meshapp/ui-kit";

import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { useIntercomEngine } from "../group/engine";
import { useAudioCheck } from "../onboarding/audioCheck";
import { LevelMeter } from "../onboarding/LevelMeter";

export function VoxMeter() {
  const { t } = useT("settings");
  const engine = useIntercomEngine();
  const check = useAudioCheck(engine);
  const reading = useEngineStore((s) => s.audioCheck);
  const running = check.running === "voxMeter";
  return (
    <div className="flex flex-col gap-3" data-testid="vox-meter">
      {running && <LevelMeter reading={reading} label={t("vox.meter")} />}
      <Button
        variant={running ? "secondary" : "primary"}
        block
        // ★ audioCheck start: issued synchronously from this tap.
        onClick={() => (running ? check.stop() : check.start("voxMeter"))}
        data-testid="vox-test"
      >
        {running ? t("vox.stop") : t("vox.test")}
      </Button>
      {running && <p className="text-xs text-muted-foreground">{t("vox.hint")}</p>}
    </div>
  );
}
