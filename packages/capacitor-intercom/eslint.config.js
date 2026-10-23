// @meshapp/capacitor-intercom lint: the shared base (WP-01) plus browser globals for
// the web implementation.
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
