// Ways into the Field test (WP-21, DESIGN §11.4, §12.1: Settings → Field test, visible,
// plain copy, no hidden gesture).
//
// - `FieldTestRow`: the row the Settings screen (WP-22) shows; it pushes `fieldTest`.
// - Phase 0 has no Settings or Home screen yet (WP-22 lands in Phase 1), so the Field
//   test registers minimal stand-ins: the shell's start screen with a Settings button,
//   and a Settings screen holding only the Field test row. They are registered only
//   when no feature has registered those ids, and WP-22's registrations replace them
//   (registerScreen replaces, whatever the feature load order).
import { Button, ListRow, ListSection } from "@meshapp/ui-kit";

import type { ScreenProps } from "../../app/router";
import { SCREEN, useRouter } from "../../app/router";
import { ShellScreen } from "../../app/ShellScreen";
import { ScreenLayout } from "../../components/ScreenLayout";
import { useT } from "../../i18n/index";

/** Settings row that opens the Field test. */
export function FieldTestRow() {
  const { t } = useT("lab");
  return (
    <ListRow
      title={t("entry.row")}
      subtitle={t("entry.subtitle")}
      trailing={<span aria-hidden="true">›</span>}
      onClick={() => useRouter.getState().push(SCREEN.fieldTest)}
    />
  );
}

/** Phase 0 start screen: the shell placeholder plus a visible way to Settings. */
export function Phase0HomeScreen(props: ScreenProps) {
  const { t } = useT("lab");
  return (
    <div className="relative">
      <div className="absolute top-[max(env(safe-area-inset-top),0.75rem)] right-3">
        <Button onClick={() => useRouter.getState().push(SCREEN.settings)}>{t("entry.settings")}</Button>
      </div>
      <ShellScreen {...props} />
    </div>
  );
}

/** Phase 0 Settings: only the Field test until WP-22's Settings screen exists. */
export function Phase0SettingsScreen(_props: ScreenProps) {
  const { t } = useT("lab");
  return (
    <ScreenLayout title={t("entry.settingsTitle")}>
      <div className="mx-auto max-w-xl" data-testid="phase0-settings">
        <ListSection>
          <FieldTestRow />
        </ListSection>
      </div>
    </ScreenLayout>
  );
}
