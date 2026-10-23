// Join confirm (WP-22, DESIGN §1.3 F2, §8.1): the invite URL (in-app scanner or a
// `convlinkvoice://join` deep link, held in memory only) goes to `parseInvite`; the
// sheet shows the group name, an expired warning and a leader-signature warning, and
// asks for the rider's name. Join → `joinGroup` → lobby.
import type { OkInviteInfo } from "@meshapp/engine-api";
import { Banner, Button } from "@meshapp/ui-kit";
import { useEffect, useState } from "react";

import type { ScreenProps } from "../../app/router";
import { ScreenLayout } from "../../components/ScreenLayout";
import { useT } from "../../i18n/index";
import { clampDisplayName, useSettingsStore } from "../../store/useSettingsStore";
import { useIntercomEngine } from "./engine";
import { NameField } from "./NameField";
import { openSession, showError } from "./session";

type Parsed = { readonly state: "loading" } | { readonly state: "invalid" } | { readonly state: "ok"; readonly info: OkInviteInfo };

export function JoinScreen({ params }: ScreenProps) {
  const { t } = useT("group");
  const engine = useIntercomEngine();
  const url = typeof params["url"] === "string" ? params["url"] : "";
  const [parsed, setParsed] = useState<Parsed>(url === "" ? { state: "invalid" } : { state: "loading" });
  const [rider, setRider] = useState(useSettingsStore.getState().displayName);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (url === "") return;
    let alive = true;
    engine.command({ type: "parseInvite", url }).then(
      (r) => {
        if (!alive) return;
        setParsed(r.ok && r.valid ? { state: "ok", info: r } : { state: "invalid" });
      },
      (e: unknown) => {
        console.warn("[join] parseInvite failed", e);
        if (alive) setParsed({ state: "invalid" });
      },
    );
    return () => {
      alive = false;
    };
  }, [engine, url]);

  const riderName = clampDisplayName(rider);
  const info = parsed.state === "ok" ? parsed.info : null;

  const join = async () => {
    if (info === null || info.groupId === null) return;
    setBusy(true);
    useSettingsStore.getState().setDisplayName(riderName);
    try {
      if (info.alreadyMember) {
        await openSession(engine, { groupId: info.groupId, name: info.groupName ?? "", role: "member" });
        return;
      }
      const result = await engine.command({ type: "joinGroup", url, displayName: riderName });
      if (!result.ok) {
        showError(result.code);
        return;
      }
      await openSession(engine, { groupId: result.groupId, name: info.groupName ?? "", role: "member" });
    } catch (e) {
      // Outside the contract (plugin missing): keep the screen, tell the rider.
      console.warn("[join] joinGroup failed", e);
      showError(undefined);
    } finally {
      setBusy(false);
    }
  };

  const canJoin = info !== null && !info.expired && riderName !== "" && !busy;

  return (
    <ScreenLayout
      title={t("join.title")}
      footer={
        info !== null ? (
          <Button variant="primary" size="lg" block disabled={!canJoin} onClick={() => void join()} data-testid="join-confirm">
            {info.alreadyMember ? t("join.open") : info.leaderSigOk ? t("join.confirm") : t("join.confirmAnyway")}
          </Button>
        ) : undefined
      }
    >
      <div className="mx-auto flex max-w-xl flex-col gap-5" data-testid="join-screen">
        {parsed.state === "loading" && (
          <p role="status" className="py-10 text-center text-muted-foreground">
            {t("join.reading")}
          </p>
        )}
        {parsed.state === "invalid" && (
          <Banner tone="danger" title={t("join.invalidTitle")} message={t("join.invalidMessage")} />
        )}
        {info !== null && (
          <>
            <div className="py-4 text-center">
              <p className="text-sm text-muted-foreground">{t("join.question")}</p>
              <h2 className="mt-1 text-3xl font-semibold break-words" data-testid="join-group-name">
                {info.groupName}
              </h2>
            </div>
            {info.expired && <Banner tone="danger" title={t("join.expiredTitle")} message={t("join.expiredMessage")} />}
            {!info.leaderSigOk && (
              <Banner tone="warn" title={t("join.leaderMismatchTitle")} message={t("join.leaderMismatchMessage")} />
            )}
            {info.alreadyMember && <Banner tone="info" title={t("join.alreadyTitle")} message={t("join.alreadyMessage")} />}
            <NameField value={rider} onChange={setRider} />
            <p className="text-xs text-muted-foreground">{t("join.privacy")}</p>
          </>
        )}
      </div>
    </ScreenLayout>
  );
}
