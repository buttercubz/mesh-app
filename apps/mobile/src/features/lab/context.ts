// Engine used by the Field test components: the app engine by default, a fake-plugin
// engine in component tests (same typed client, same gesture routing).
import { createContext, useContext } from "react";

import { engine } from "../../engine/client";
import type { LabEngine } from "./actions";

export const LabEngineContext = createContext<LabEngine>(engine);

export function useLabEngine(): LabEngine {
  return useContext(LabEngineContext);
}
