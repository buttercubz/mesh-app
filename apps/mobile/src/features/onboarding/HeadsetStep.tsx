// Onboarding headset check + Echo test (WP-22, DESIGN §1.3 F0 step 3), skippable.
// "Turn on and pair your helmet now", then the tap starts `audioCheck{tone}` (★, inside
// the gesture): the engine plays a tone, records 2 s and streams the level and detected
// route. The Echo test (`audioCheck{echo}`) plays the rider's voice back after 1 s.
import { Button } from "@meshapp/ui-kit";

import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { useIntercomEngine } from "../group/engine";
import { useAudioCheck } from "./audioCheck";
import { LevelMeter } from "./LevelMeter";

export interface HeadsetStepProps {
  readonly onNext: () => void;
}

export function HeadsetStep({ onNext }: HeadsetStepProps) {
  const { t } = useT("onboarding");
  const { t: tc } = useT("common");
  const engine = useIntercomEngine();
  const check = useAudioCheck(engine);
  const reading = useEngineStore((s) => s.audioCheck);
  const narrow = reading?.route === "hfpNarrowband";
  const speaker = reading?.route === "builtinSpeaker" || reading?.route === "builtinReceiver";

  const finish = () => {
    check.stop();
    onNext();
  };

  return (
    <div className="flex flex-col gap-4" data-testid="headset-step">
      <p className="text-lg font-semibold">{t("headset.pairNow")}</p>
      <p className="text-sm text-muted-foreground">{t("headset.why")}</p>
      <div className="rounded-lg bg-card p-4 text-card-foreground">
        <LevelMeter reading={check.running === null ? null : reading} label={t("headset.meter")} />
      </div>
      {narrow && <p className="text-sm">{t("headset.tipNarrow")}</p>}
      {speaker && <p className="text-sm">{t("headset.tipSpeaker")}</p>}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Button
          variant={check.running === "tone" ? "secondary" : "primary"}
          size="lg"
          block
          // ★ audioCheck start: issued synchronously from this tap.
          onClick={() => (check.running === "tone" ? check.stop() : check.start("tone"))}
          data-testid="check-tone"
        >
          {check.running === "tone" ? t("headset.stop") : t("headset.start")}
        </Button>
        <Button
          variant="secondary"
          size="lg"
          block
          onClick={() => (check.running === "echo" ? check.stop() : check.start("echo"))}
          data-testid="check-echo"
        >
          {check.running === "echo" ? t("headset.stop") : t("headset.echo")}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">{t("headset.echoHint")}</p>
      <Button variant="primary" size="lg" block onClick={finish}>
        {reading !== null ? tc("action.continue") : t("headset.skip")}
      </Button>
    </div>
  );
}
