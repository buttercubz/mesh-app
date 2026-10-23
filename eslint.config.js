// Shared ESLint flat config (owned by WP-01, DESIGN §2.1; rules in docs/conventions/typescript.md).
//
// Every package and app extends it from its own eslint.config.js:
//
//   import { defineConfig } from "eslint/config";
//   import base from "../../eslint.config.js";
//   export default defineConfig([...base, { /* package-specific files, globals, plugins */ }]);
//
// React-specific plugins (react-hooks, react-compiler, react-refresh) are added by
// apps/mobile, which owns those dependencies. Running `eslint` at the repository root
// lints the root tooling (scripts/*.mjs and this file).
import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";
import { defineConfig, globalIgnores } from "eslint/config";

/** Rules shared by JS and TS sources (convLink rules carried over). */
const sharedRules = {
  // Every catch block explains why the error is handled or ignored: no silent failures.
  "no-empty": ["error", { allowEmptyCatch: false }],
  eqeqeq: ["error", "always"],
  "no-var": "error",
  "prefer-const": "error",
};

export default defineConfig([
  globalIgnores([
    "**/dist/",
    "**/build/",
    "**/coverage/",
    "**/node_modules/",
    "**/target/",
    "**/playwright-report/",
    "**/test-results/",
    "native/",
    "third_party/",
    "fuzz/",
    "apps/mobile/ios/",
    "apps/mobile/android/",
    "packages/*/wasm/",
    "packages/*/src/generated/",
  ]),
  {
    name: "mesh-app/javascript",
    files: ["**/*.{js,mjs,cjs}"],
    extends: [js.configs.recommended],
    languageOptions: {
      ecmaVersion: 2024,
      sourceType: "module",
      globals: { ...globals.node },
    },
    rules: sharedRules,
  },
  {
    name: "mesh-app/typescript",
    files: ["**/*.{ts,tsx,mts,cts}"],
    extends: [js.configs.recommended, tseslint.configs.recommended],
    languageOptions: {
      ecmaVersion: 2024,
      sourceType: "module",
    },
    rules: {
      ...sharedRules,
      // No `any` in new code: use `unknown` plus guards.
      "@typescript-eslint/no-explicit-any": "error",
      // `@ts-ignore` / `@ts-expect-error` only with a reason.
      "@typescript-eslint/ban-ts-comment": [
        "error",
        { "ts-ignore": "allow-with-description", "ts-expect-error": "allow-with-description" },
      ],
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_", caughtErrorsIgnorePattern: "^_" },
      ],
      "@typescript-eslint/consistent-type-imports": ["error", { fixStyle: "separate-type-imports" }],
      "@typescript-eslint/no-unused-expressions": [
        "error",
        { allowShortCircuit: true, allowTernary: true, allowTaggedTemplates: true },
      ],
      // erasableSyntaxOnly: no enums, namespaces or parameter properties.
      "no-restricted-syntax": [
        "error",
        { selector: "TSEnumDeclaration", message: "Use a union type or an `as const` object (erasableSyntaxOnly)." },
        { selector: "TSModuleDeclaration[kind='namespace']", message: "No namespaces (erasableSyntaxOnly)." },
        { selector: "TSParameterProperty", message: "No parameter properties (erasableSyntaxOnly)." },
      ],
    },
  },
]);
