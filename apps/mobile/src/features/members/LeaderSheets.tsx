// Leader action sheets of the Members screen (WP-22, DESIGN §1.3 F9): the member sheet
// (Remove, Make leader), and one confirmation sheet per action. Remove and Lock first
// show the unconfirmed members who would lose access (`previewAccessLoss`), with the
// choice to wait for them ("Confirm reachable members first") or proceed.
import type { Member, NodeRef } from "@meshapp/engine-api";
import { Button, Sheet } from "@meshapp/ui-kit";

import { useT } from "../../i18n/index";

/** What the leader is about to do. */
export type LeaderAction =
  | { readonly kind: "member"; readonly member: Member }
  | { readonly kind: "remove"; readonly member: Member; readonly unconfirmed: readonly NodeRef[] }
  | { readonly kind: "lock"; readonly unconfirmed: readonly NodeRef[] }
  | { readonly kind: "close" }
  | { readonly kind: "transfer"; readonly member: Member };

export interface LeaderSheetProps {
  readonly action: LeaderAction | null;
  readonly busy: boolean;
  readonly onClose: () => void;
  /** Member sheet: start Remove (fetches the access-loss preview first). */
  readonly onRemove: (member: Member) => void;
  /** Member sheet: start Make leader. */
  readonly onTransfer: (member: Member) => void;
  /** Confirms the action shown. */
  readonly onConfirm: (action: LeaderAction) => void;
}

function AccessLoss({ unconfirmed }: { readonly unconfirmed: readonly NodeRef[] }) {
  const { t } = useT("group");
  if (unconfirmed.length === 0) return <p className="text-sm text-muted-foreground">{t("leader.noAccessLoss")}</p>;
  return (
    <div className="flex flex-col gap-2 rounded-lg bg-ladder-warn/15 p-3" data-testid="access-loss">
      <p className="text-sm font-semibold">{t("leader.accessLossTitle", { n: unconfirmed.length })}</p>
      <ul className="list-inside list-disc text-sm">
        {unconfirmed.map((n) => (
          <li key={n.nodeId}>{n.name}</li>
        ))}
      </ul>
      <p className="text-sm text-muted-foreground">{t("leader.accessLossHint")}</p>
    </div>
  );
}

export function LeaderSheet({ action, busy, onClose, onRemove, onTransfer, onConfirm }: LeaderSheetProps) {
  const { t } = useT("group");
  const { t: tc } = useT("common");
  if (action === null) return null;
  const title = {
    member: () => (action.kind === "member" ? action.member.name : ""),
    remove: () => (action.kind === "remove" ? t("leader.removeTitle", { name: action.member.name }) : ""),
    lock: () => t("leader.lockTitle"),
    close: () => t("leader.closeTitle"),
    transfer: () => (action.kind === "transfer" ? t("leader.transferTitle", { name: action.member.name }) : ""),
  }[action.kind]();

  let body;
  if (action.kind === "member") {
    const present = action.member.status === "direct" || action.member.status === "relayed";
    body = (
      <>
        <Button variant="secondary" size="lg" block disabled={busy || !present} onClick={() => onTransfer(action.member)}>
          {t("leader.makeLeader")}
        </Button>
        {!present && <p className="text-xs text-muted-foreground">{t("leader.makeLeaderAbsent")}</p>}
        <Button variant="danger" size="lg" block disabled={busy} onClick={() => onRemove(action.member)}>
          {t("leader.remove")}
        </Button>
      </>
    );
  } else {
    const waitable = (action.kind === "remove" || action.kind === "lock") && action.unconfirmed.length > 0;
    const message = {
      remove: t("leader.removeMessage"),
      lock: t("leader.lockMessage"),
      close: t("leader.closeMessage"),
      transfer: t("leader.transferMessage"),
    }[action.kind];
    const confirm = {
      remove: waitable ? t("leader.removeAnyway") : t("leader.remove"),
      lock: waitable ? t("leader.lockAnyway") : t("leader.lock"),
      close: t("leader.close"),
      transfer: t("leader.makeLeader"),
    }[action.kind];
    body = (
      <>
        <p className="text-sm text-muted-foreground">{message}</p>
        {(action.kind === "remove" || action.kind === "lock") && <AccessLoss unconfirmed={action.unconfirmed} />}
        {waitable && (
          <Button variant="primary" size="lg" block onClick={onClose}>
            {t("leader.waitFirst")}
          </Button>
        )}
        <Button
          variant={action.kind === "transfer" ? "primary" : "danger"}
          size="lg"
          block
          disabled={busy}
          onClick={() => onConfirm(action)}
          data-testid="leader-confirm"
        >
          {confirm}
        </Button>
      </>
    );
  }

  return (
    <Sheet open onClose={onClose} title={title} closeLabel={tc("action.close")}>
      <div className="flex flex-col gap-3" data-testid={`leader-sheet-${action.kind}`}>
        {body}
        <Button variant="plain" size="lg" block onClick={onClose}>
          {tc("action.cancel")}
        </Button>
      </div>
    </Sheet>
  );
}
