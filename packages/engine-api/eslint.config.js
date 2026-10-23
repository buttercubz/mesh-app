// @meshapp/engine-api lint: the shared base (WP-01). src/generated/ is ignored by the
// base config (written only by `cargo xtask gen-ts`).
import { defineConfig } from "eslint/config";
import base from "../../eslint.config.js";

export default defineConfig([...base]);
