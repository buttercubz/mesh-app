// Leave / End ride confirmation (WP-22, DESIGN §1.3 F8): "Leave" floods LEAVE and stops
// the engine (the group stays saved); the leader can also end the ride for everyone
// (RIDE_CTL end: members are told, never auto-stopped).
import { Button, Sheet } from "@meshapp/ui-kit";

import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { useIntercomEngine } from "../group/engine";
import { leaveSession, runCommand } from "../group/session";

/** On short screens the secondary actions drop to the 48 px target so all three fit. */
const SHORT_SCREEN_BUTTON = "[@media(max-height:700px)]:min-h-target [@media(max-height:700px)]:py-2.5";

export interface LeaveSheetProps {
  readonly open: boolean;
  readonly onClose: () => void;
}

export function LeaveSheet({ open, onClose }: LeaveSheetProps) {
  const { t } = useT("ride");
  const { t: tc } = useT("common");
  const engine = useIntercomEngine();
  const leader = useEngineStore((s) => s.snapshot?.session?.role === "leader");
  const riding = useEngineStore((s) => s.snapshot?.session?.ride === "riding");
  return (
    <Sheet open={open} onClose={onClose} title={t("leave.title")} closeLabel={tc("action.close")}>
      <div className="flex flex-col gap-3" data-testid="leave-sheet">
        <p className="text-sm text-muted-foreground">{t("leave.message")}</p>
        <Button
          variant="danger"
          size="lg"
          block
          onClick={() => {
            onClose();
            void leaveSession(engine);
          }}
        >
          {t("leave.confirm")}
        </Button>
        {leader && riding && (
          <Button
            variant="secondary"
            size="lg"
            block
            className={SHORT_SCREEN_BUTTON}
            onClick={() => {
              onClose();
              void runCommand(engine, { type: "rideControl", op: "end" });
            }}
          >
            {t("leave.endRide")}
          </Button>
        )}
        <Button variant="plain" size="lg" block className={SHORT_SCREEN_BUTTON} onClick={onClose}>
          {tc("action.cancel")}
        </Button>
      </div>
    </Sheet>
  );
}
