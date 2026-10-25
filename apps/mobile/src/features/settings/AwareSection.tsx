// Settings → Faster link (WP-24 beta): the "Wi-Fi Aware (beta, Android)" switch, off by
// default. Shown only on Android phones that have Wi-Fi Aware; Android↔Android links then
// upgrade from Bluetooth to Wi-Fi Aware, iPhone↔Android stays on Bluetooth. ES + EN
// through the `settings` catalog.
import { useEffect, useState } from "react";

import type { AwareStatus } from "../../engine/client";
import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { useIntercomEngine } from "../group/engine";
import { SettingRow, SettingsGroup, Switch } from "./controls";

export function AwareSection() {
  const { t } = useT("settings");
  const engine = useIntercomEngine();
  const android = useEngineStore((s) => s.info?.platform === "android");
  const [status, setStatus] = useState<AwareStatus | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    if (!android) return;
    engine.awareStatus().then(setStatus, (e: unknown) => {
      // No engine (platform-only build): the section stays hidden.
      console.warn("[settings] Wi-Fi Aware status unavailable", e);
    });
  }, [engine, android]);

  if (!android || status === null || !status.supported) return null;

  const apply = (on: boolean) => {
    setStatus({ ...status, enabled: on });
    engine.awareSetEnabled(on).catch((e: unknown) => {
      console.warn("[settings] Wi-Fi Aware toggle failed", e);
      setStatus({ ...status, enabled: !on });
    });
  };

  const toggle = (on: boolean) => {
    setMessage(null);
    if (!on) {
      apply(false);
      return;
    }
    // Wi-Fi Aware needs NEARBY_WIFI_DEVICES (Android 13+, neverForLocation).
    engine.requestPermissions(["nearbyWifi"]).then(
      (p) => {
        if (p.nearbyWifi === "granted") apply(true);
        else setMessage(t("aware.permission"));
      },
      (e: unknown) => {
        console.warn("[settings] nearby Wi-Fi permission request failed", e);
        setMessage(t("aware.permission"));
      },
    );
  };

  return (
    <SettingsGroup title={t("aware.title")}>
      <SettingRow label={t("aware.toggle")} description={t("aware.hint")} testId="aware-row">
        <Switch label={t("aware.toggle")} checked={status.enabled} onChange={toggle} />
      </SettingRow>
      {message !== null && <p className="px-4 pb-3 text-sm text-muted-foreground">{message}</p>}
    </SettingsGroup>
  );
}
