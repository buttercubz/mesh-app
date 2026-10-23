// Field test feature (WP-21, DESIGN §12.5). Loaded by app/features.ts, which registers
// every feature's screens before the first render.
import { SCREEN, getScreen, registerScreen } from "../../app/router";
import { FieldTestScreen } from "./FieldTestScreen";
import { Phase0HomeScreen, Phase0SettingsScreen } from "./entry";

registerScreen(SCREEN.fieldTest, FieldTestScreen);

// Phase 0 stand-ins (see entry.tsx); never override a real Home or Settings screen.
if (getScreen(SCREEN.home) === undefined) registerScreen(SCREEN.home, Phase0HomeScreen);
if (getScreen(SCREEN.settings) === undefined) registerScreen(SCREEN.settings, Phase0SettingsScreen);

export { FieldTestRow } from "./entry";
export { FieldTestScreen } from "./FieldTestScreen";
