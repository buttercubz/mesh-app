// @meshapp/ui-kit lint: the shared base (WP-01) plus browser globals and the React
// hooks rules (Rules of React, so the app's React Compiler can optimise the primitives).
import { defineConfig } from "eslint/config";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import base from "../../eslint.config.js";

export default defineConfig([
  ...base,
  {
    name: "ui-kit/browser",
    files: ["src/**/*.{ts,tsx}", "test/**/*.{ts,tsx}"],
    languageOptions: { globals: { ...globals.browser } },
  },
  {
    name: "ui-kit/react-hooks",
    files: ["src/**/*.{ts,tsx}", "test/**/*.{ts,tsx}"],
    extends: [reactHooks.configs.flat.recommended],
  },
]);
