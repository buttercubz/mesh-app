// Members feature (WP-22, DESIGN §11.4). Loaded by app/features.ts.
import { SCREEN, registerScreen } from "../../app/router";
import { MembersScreen } from "./MembersScreen";

registerScreen(SCREEN.members, MembersScreen);

export { MembersScreen } from "./MembersScreen";
