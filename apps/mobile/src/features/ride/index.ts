// Ride feature (WP-22, DESIGN §11.4). Loaded by app/features.ts. Back never pops the
// ride screen by accident: its own back handler opens the Leave sheet.
import { SCREEN, registerScreen } from "../../app/router";
import { RideScreen } from "./RideScreen";

registerScreen(SCREEN.ride, RideScreen, { blockBack: true });

export { RideScreen } from "./RideScreen";
export { RideBanners } from "./RideBanners";
