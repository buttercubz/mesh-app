// @meshapp/mobile lint (WP-18): the shared base (WP-01) plus React (hooks, compiler,
// refresh) and two app rules:
//   - meshapp/no-gesture-command-via-send: a ★ command (DESIGN §3.2) must never be sent
//     through the plugin's `send`; it goes through `gestureCommand` (engine.command
//     routes it automatically).
//   - no-restricted-imports: UI code reaches the plugin only through src/engine/.
import { defineConfig } from "eslint/config";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import base from "../../eslint.config.js";

import { GESTURE_COMMANDS } from "@meshapp/engine-api";

/** Static value of a literal node (string/number/boolean/null), else `undefined`. */
function literalValue(node) {
  if (node?.type === "Literal") return node.value;
  if (node?.type === "TemplateLiteral" && node.expressions.length === 0) return node.quasis[0]?.value.cooked;
  return undefined;
}

function propertyKey(prop) {
  if (prop.type !== "Property") return undefined;
  if (prop.key.type === "Identifier" && !prop.computed) return prop.key.name;
  const v = literalValue(prop.key);
  return typeof v === "string" ? v : undefined;
}

function findProperty(objectNode, key) {
  return objectNode.properties.find((p) => propertyKey(p) === key);
}

/** Follows `path` through nested object literals; `undefined` when it cannot be resolved. */
function valueAt(objectNode, path) {
  let cur = objectNode;
  for (const key of path) {
    if (cur?.type !== "ObjectExpression") return { known: false };
    const prop = findProperty(cur, key);
    if (prop === undefined) return { known: true, value: undefined };
    cur = prop.value;
  }
  const v = literalValue(cur);
  return v === undefined && cur?.type !== "Identifier" ? { known: false } : { known: v !== undefined, value: v };
}

/** `true` when an object-literal command may be ★ (conservative when not provable). */
function objectMayNeedGesture(objectNode) {
  const typeProp = findProperty(objectNode, "type");
  const type = typeProp ? literalValue(typeProp.value) : undefined;
  if (typeof type !== "string") return undefined;
  for (const rule of GESTURE_COMMANDS.filter((r) => r.type === type)) {
    if (rule.when === null) return type;
    const at = valueAt(objectNode, rule.when.path);
    if (!at.known) return type;
    const equal = at.value === rule.when.value;
    if (rule.when.op === "equals" ? equal : !equal) return type;
  }
  return undefined;
}

/** Same check for a JSON string literal such as '{"type":"goLive"}'. */
function textMayNeedGesture(text) {
  for (const rule of GESTURE_COMMANDS) {
    if (!new RegExp(`"type"\\s*:\\s*"${rule.type}"`).test(text)) continue;
    if (rule.when === null) return rule.type;
    const key = rule.when.path[rule.when.path.length - 1];
    const m = new RegExp(`"${key}"\\s*:\\s*"([^"]*)"`).exec(text);
    if (m === null) return rule.type;
    const equal = m[1] === rule.when.value;
    if (rule.when.op === "equals" ? equal : !equal) return rule.type;
  }
  return undefined;
}

/** ESLint rule: no ★ command (GESTURE_COMMANDS) through the plugin's `send`. */
export const noGestureCommandViaSend = {
  meta: {
    type: "problem",
    docs: { description: "★ commands must go through gestureCommand (engine.command routes them), never send" },
    messages: {
      viaSend:
        "`{{type}}` is a ★ gesture command (DESIGN §3.2): call engine.command(...) or gestureCommand inside the tap, never send().",
    },
    schema: [],
  },
  create(context) {
    return {
      CallExpression(node) {
        const callee = node.callee;
        const name =
          callee.type === "MemberExpression" && !callee.computed && callee.property.type === "Identifier"
            ? callee.property.name
            : callee.type === "Identifier"
              ? callee.name
              : undefined;
        if (name !== "send") return;
        const seen = new Set();
        const visit = (n) => {
          if (n === null || typeof n !== "object" || seen.has(n)) return undefined;
          seen.add(n);
          if (n.type === "ObjectExpression") {
            const hit = objectMayNeedGesture(n);
            if (hit !== undefined) return hit;
          }
          const text = literalValue(n);
          if (typeof text === "string") {
            const hit = textMayNeedGesture(text);
            if (hit !== undefined) return hit;
          }
          for (const [key, child] of Object.entries(n)) {
            if (key === "parent" || key === "loc" || key === "range") continue;
            const children = Array.isArray(child) ? child : [child];
            for (const c of children) {
              if (c && typeof c.type === "string") {
                const hit = visit(c);
                if (hit !== undefined) return hit;
              }
            }
          }
          return undefined;
        };
        for (const arg of node.arguments) {
          const type = visit(arg);
          if (type !== undefined) {
            context.report({ node, messageId: "viaSend", data: { type } });
            return;
          }
        }
      },
    };
  },
};

export default defineConfig([
  ...base,
  {
    name: "mobile/ignores",
    ignores: ["ios/**", "android/**", "dist/**", "playwright-report/**", "test-results/**"],
  },
  {
    name: "mobile/browser",
    files: ["src/**/*.{ts,tsx}", "tests/**/*.{ts,tsx}", "e2e/**/*.ts"],
    languageOptions: { globals: { ...globals.browser, ...globals.node } },
  },
  {
    name: "mobile/react",
    files: ["src/**/*.{ts,tsx}", "tests/**/*.{ts,tsx}"],
    extends: [reactHooks.configs.flat.recommended],
  },
  {
    name: "mobile/react-refresh",
    files: ["src/**/*.tsx"],
    extends: [reactRefresh.configs.vite],
  },
  {
    name: "mobile/intercom-rules",
    files: ["src/**/*.{ts,tsx}", "e2e/**/*.ts"],
    plugins: { meshapp: { rules: { "no-gesture-command-via-send": noGestureCommandViaSend } } },
    rules: {
      "meshapp/no-gesture-command-via-send": "error",
      "no-restricted-imports": [
        "error",
        {
          paths: [
            {
              name: "@meshapp/capacitor-intercom",
              message: "Use `engine` from src/engine/client.ts (typed, validated, gesture-routed).",
            },
          ],
        },
      ],
    },
  },
  {
    name: "mobile/engine-owns-plugin",
    files: ["src/engine/**/*.{ts,tsx}", "src/store/**/*.ts"],
    rules: { "no-restricted-imports": "off" },
  },
]);
