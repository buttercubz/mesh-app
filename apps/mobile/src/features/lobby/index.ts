// Lobby feature (WP-22, DESIGN §11.4). Loaded by app/features.ts.
import { SCREEN, registerScreen } from "../../app/router";
import { LobbyScreen } from "./LobbyScreen";

registerScreen(SCREEN.lobby, LobbyScreen);

export { LobbyScreen } from "./LobbyScreen";
