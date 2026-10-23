// The app lint rule meshapp/no-gesture-command-via-send (WP-18 acceptance: "no ★
// command through send"): exercised with ESLint's RuleTester, then against the real
// config so the rule is proven to be wired for src/.
import { ESLint, RuleTester } from "eslint";
import tseslint from "typescript-eslint";
import { describe, expect, it } from "vitest";
import { fileURLToPath } from "node:url";

// @ts-expect-error -- eslint.config.js is plain JS without type declarations.
import { noGestureCommandViaSend } from "../eslint.config.js";

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const tester = new RuleTester({ languageOptions: { parser: tseslint.parser, ecmaVersion: 2024, sourceType: "module" } });

tester.run("no-gesture-command-via-send", noGestureCommandViaSend, {
  valid: [
    'engine.command({ type: "goLive" });',
    'plugin.gestureCommand({ command: JSON.stringify({ type: "goLive" }) });',
    'plugin.send({ command: JSON.stringify({ type: "labStop" }) });',
    'plugin.send({ command: JSON.stringify({ type: "labStart", bench: "T0", params: { audio: "off", anchor: "none", gps: false } }) });',
    'plugin.send({ command: JSON.stringify({ type: "audioCheck", op: "stop" }) });',
    "plugin.send({ command: '{\"type\":\"audioCheck\",\"op\":\"stop\"}' });",
    'socket.emit({ type: "goLive" });',
  ],
  invalid: [
    { code: 'plugin.send({ command: JSON.stringify({ type: "goLive" }) });', errors: [{ messageId: "viaSend" }] },
    { code: 'MeshIntercom.send({ command: JSON.stringify({ type: "resumeMic" }) });', errors: [{ messageId: "viaSend" }] },
    {
      code: 'plugin.send({ command: JSON.stringify({ type: "audioCheck", op: "start", mode: "tone" }) });',
      errors: [{ messageId: "viaSend" }],
    },
    {
      code: 'plugin.send({ command: JSON.stringify({ type: "labStart", bench: "T0", params: { audio: "full", anchor: "audio", gps: false } }) });',
      errors: [{ messageId: "viaSend" }],
    },
    {
      code: 'const audio = pick(); plugin.send({ command: JSON.stringify({ type: "labStart", bench: "T0", params: { audio, anchor: "audio", gps: false } }) });',
      errors: [{ messageId: "viaSend" }],
    },
    { code: "plugin.send({ command: '{\"type\":\"goLive\"}' });", errors: [{ messageId: "viaSend" }] },
    { code: "send({ command: `{\"type\":\"resumeMic\"}` });", errors: [{ messageId: "viaSend" }] },
  ],
});

describe("app lint config", () => {
  it("applies the rule and the plugin-import restriction to src/", async () => {
    const cwd = fileURLToPath(new URL("..", import.meta.url));
    const eslint = new ESLint({ cwd });
    const [result] = await eslint.lintText(
      [
        'import { MeshIntercom } from "@meshapp/capacitor-intercom";',
        'void MeshIntercom.send({ command: JSON.stringify({ type: "goLive" }) });',
        "",
      ].join("\n"),
      { filePath: `${cwd}src/features/demo/probe.ts` },
    );
    const rules = (result?.messages ?? []).map((m) => m.ruleId);
    expect(rules).toContain("meshapp/no-gesture-command-via-send");
    expect(rules).toContain("no-restricted-imports");
  });
});
