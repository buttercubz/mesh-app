// Group feature (WP-22, DESIGN §11.4): Create group and Join confirm. Also hosts the
// intercom session helpers shared by the other WP-22 features. Loaded by app/features.ts.
import { SCREEN, registerScreen } from "../../app/router";
import { CreateGroupScreen } from "./CreateGroupScreen";
import { CREATE_GROUP_SCREEN } from "./ids";
import { JoinScreen } from "./JoinScreen";

registerScreen(CREATE_GROUP_SCREEN, CreateGroupScreen);
registerScreen(SCREEN.join, JoinScreen);

export { CreateGroupScreen } from "./CreateGroupScreen";
export { JoinScreen } from "./JoinScreen";
export { CREATE_GROUP_SCREEN } from "./ids";
