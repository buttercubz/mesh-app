// Home feature (WP-22, DESIGN §11.4). Loaded by app/features.ts; replaces the shell
// placeholder and the Field test's Phase 0 stand-in, whatever the load order.
import { SCREEN, registerScreen } from "../../app/router";
import { HomeScreen } from "./HomeScreen";

registerScreen(SCREEN.home, HomeScreen);

export { HomeScreen } from "./HomeScreen";
