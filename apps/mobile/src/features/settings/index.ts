// Settings feature (WP-22, DESIGN §11.4). Loaded by app/features.ts after the Field test
// feature, so it replaces the Field test's Phase 0 Settings stand-in.
import { SCREEN, registerScreen } from "../../app/router";
import { SettingsScreen } from "./SettingsScreen";

registerScreen(SCREEN.settings, SettingsScreen);

export { SettingsScreen } from "./SettingsScreen";
