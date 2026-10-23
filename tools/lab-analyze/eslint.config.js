// @meshapp/lab-analyze lint: the shared base (WP-01). Plain Node ESM, no extra rules.
import { defineConfig } from "eslint/config";
import base from "../../eslint.config.js";

export default defineConfig([...base]);
