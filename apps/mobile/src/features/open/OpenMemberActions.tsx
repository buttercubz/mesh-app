// Local mute / block of a rider on an open channel (never sent on air).
import type { Member } from "@meshapp/engine-api";
import { Button } from "@meshapp/ui-kit";
import { Ban, VolumeX } from "lucide-react";

import { useT } from "../../i18n/index";
import { useIntercomEngine } from "../group/engine";
import { setBlocked, setMuted } from "./actions";

export function OpenMemberActions({ member: m }: { readonly member: Member }) {
  const { t } = useT("group");
  const engine = useIntercomEngine();
  const muted = m.flags.includes("muted");
  const blocked = m.flags.includes("blocked");
  return (
    <div className="flex gap-2 px-4 pb-3.5" data-testid="open-member-actions">
      <Button
        variant="secondary"
        size="md"
        aria-label={t(muted ? "open.unmuteName" : "open.muteName", { name: m.name })}
        onClick={() => void setMuted(engine, m, !muted)}
      >
        <VolumeX aria-hidden="true" />
        {t(muted ? "open.unmute" : "open.mute")}
      </Button>
      <Button
        variant={blocked ? "secondary" : "danger"}
        size="md"
        aria-label={t(blocked ? "open.unblockName" : "open.blockName", { name: m.name })}
        onClick={() => void setBlocked(engine, m, !blocked)}
      >
        <Ban aria-hidden="true" />
        {t(blocked ? "open.unblock" : "open.block")}
      </Button>
    </div>
  );
}
