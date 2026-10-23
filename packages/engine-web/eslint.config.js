// @meshapp/engine-web lint: the shared base (WP-01). wasm/ is ignored by the base config
// (written only by `cargo xtask build-wasm`).
import { defineConfig } from "eslint/config";
import globals from "globals";
import base from "../../eslint.config.js";

export default defineConfig([
  ...base,
  {
    files: ["src/**/*.ts"],
    languageOptions: { globals: { ...globals.browser } },
  },
]);
