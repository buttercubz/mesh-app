// Create group (WP-22, DESIGN §1.3 F1, §11.4): name + vehicle, advanced invite expiry.
// `createGroup` makes the group id, epoch secret and the leader identity natively (never
// in JS); then the lobby opens.
import type { Vehicle } from "@meshapp/engine-api";
import { Button, SegmentedControl } from "@meshapp/ui-kit";
import { useId, useState } from "react";

import type { ScreenProps } from "../../app/router";
import { ScreenLayout } from "../../components/ScreenLayout";
import { useT } from "../../i18n/index";
import { clampDisplayName, useSettingsStore } from "../../store/useSettingsStore";
import { useIntercomEngine } from "./engine";
import { NameField } from "./NameField";
import { openSession, runCommand, showError } from "./session";

/** Invite expiry choices in hours (`null` = never, the default). */
export const EXPIRY_CHOICES = [null, 1, 4, 12, 24] as const;
export const GROUP_NAME_MAX = 32;

export function CreateGroupScreen(_props: ScreenProps) {
  const { t } = useT("group");
  const { t: tc } = useT("common");
  const engine = useIntercomEngine();
  const savedName = useSettingsStore((s) => s.displayName);
  const [name, setName] = useState("");
  const [rider, setRider] = useState(savedName);
  const [vehicle, setVehicle] = useState<Vehicle>(useSettingsStore.getState().vehicle);
  const [expiry, setExpiry] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const nameId = useId();
  const expiryId = useId();

  const groupName = name.trim().slice(0, GROUP_NAME_MAX);
  const riderName = clampDisplayName(rider);
  const ready = groupName !== "" && riderName !== "" && !busy;

  const create = async () => {
    setBusy(true);
    const settings = useSettingsStore.getState();
    settings.setDisplayName(riderName);
    if (settings.vehicle !== vehicle) settings.setVehicle(vehicle);
    try {
      const result = await engine.command({ type: "createGroup", name: groupName, vehicle });
      if (!result.ok) {
        showError(result.code);
        return;
      }
      const opened = await openSession(engine, { groupId: result.groupId, name: groupName, role: "leader" });
      if (opened && expiry !== null) await runCommand(engine, { type: "setInviteExpiry", hours: expiry });
    } catch (e) {
      // The plugin failed outside the contract: say so, keep the form.
      console.warn("[group] createGroup failed", e);
      showError(undefined);
    } finally {
      setBusy(false);
    }
  };

  return (
    <ScreenLayout
      title={t("create.title")}
      footer={
        <Button variant="primary" size="lg" block disabled={!ready} onClick={() => void create()} data-testid="create-confirm">
          {busy ? t("create.creating") : t("create.confirm")}
        </Button>
      }
    >
      <form
        className="mx-auto flex max-w-xl flex-col gap-5"
        onSubmit={(e) => {
          e.preventDefault();
          if (ready) void create();
        }}
      >
        <div className="flex flex-col gap-2">
          <label htmlFor={nameId} className="text-sm font-semibold">
            {t("create.name")}
          </label>
          <input
            id={nameId}
            className="min-h-target rounded-lg border border-border bg-card px-3 text-lg text-card-foreground"
            value={name}
            maxLength={GROUP_NAME_MAX}
            autoComplete="off"
            placeholder={t("create.namePlaceholder")}
            onChange={(e) => setName(e.target.value)}
          />
        </div>
        <NameField value={rider} onChange={setRider} />
        <div className="flex flex-col gap-2">
          <span className="text-sm font-semibold">{t("create.vehicle")}</span>
          <SegmentedControl
            label={t("create.vehicle")}
            value={vehicle}
            onChange={setVehicle}
            options={[
              { value: "moto", label: tc("vehicle.moto") },
              { value: "car", label: tc("vehicle.car") },
            ]}
          />
          <p className="text-xs text-muted-foreground">{t(`create.vehicleHint.${vehicle}`)}</p>
        </div>
        <details className="rounded-lg bg-card p-3 text-card-foreground">
          <summary className="min-h-target cursor-pointer content-center text-sm font-semibold">
            {t("create.advanced")}
          </summary>
          <div className="flex flex-col gap-2 pt-2">
            <label htmlFor={expiryId} className="text-sm">
              {t("create.expiry")}
            </label>
            <select
              id={expiryId}
              className="min-h-target rounded-lg border border-border bg-background px-3 text-base"
              value={expiry === null ? "never" : String(expiry)}
              onChange={(e) => setExpiry(e.target.value === "never" ? null : Number(e.target.value))}
            >
              {EXPIRY_CHOICES.map((h) => (
                <option key={h ?? "never"} value={h === null ? "never" : String(h)}>
                  {h === null ? t("create.expiryNever") : t("create.expiryHours", { n: h })}
                </option>
              ))}
            </select>
            <p className="text-xs text-muted-foreground">{t("create.expiryHint")}</p>
            <p className="text-xs text-muted-foreground">{t("create.leaderPriority")}</p>
          </div>
        </details>
      </form>
    </ScreenLayout>
  );
}
