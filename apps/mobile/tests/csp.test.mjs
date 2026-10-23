// Static contract: Content Security Policy (WP-18, DESIGN §11.3: `default-src 'self'`).
// The app is offline; nothing may load from or connect to another origin, and no inline
// script may run. `vite dev` relaxes the policy (vite.config.ts) — the shipped bundle
// keeps the strict one.
import { describe, expect, it } from "vitest";

import { exists, read } from "./contracts/files.mjs";

export function cspOf(html) {
  const m = /<meta\s+http-equiv="Content-Security-Policy"\s+content="([^"]+)"/i.exec(html);
  if (!m) return null;
  return Object.fromEntries(
    m[1]
      .split(";")
      .map((d) => d.trim().split(/\s+/))
      .filter((parts) => parts[0])
      .map(([name, ...values]) => [name, values]),
  );
}

function expectStrict(csp) {
  expect(csp).not.toBeNull();
  expect(csp["default-src"]).toEqual(["'self'"]);
  expect(csp["script-src"]).toEqual(["'self'", "'wasm-unsafe-eval'"]);
  expect(csp["style-src"]).toEqual(["'self'"]);
  expect(csp["connect-src"]).toEqual(["'self'"]);
  expect(csp["object-src"]).toEqual(["'none'"]);
  expect(csp["base-uri"]).toEqual(["'none'"]);
  expect(csp["form-action"]).toEqual(["'none'"]);
  for (const [name, values] of Object.entries(csp)) {
    for (const v of values) {
      expect(v, `${name} ${v}`).not.toMatch(/^(https?:|wss?:|\*|'unsafe-inline'|'unsafe-eval')/);
    }
  }
  // Only these extra sources: data: images (the native QR PNG) and blob: media/workers.
  expect(csp["img-src"]).toEqual(["'self'", "data:", "blob:"]);
}

describe("index.html CSP", () => {
  const html = read("index.html");

  it("is strict and same-origin only", () => expectStrict(cspOf(html)));

  it("has no inline scripts and no remote resources", () => {
    const scripts = [...html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/g)];
    expect(scripts).toHaveLength(1);
    expect(scripts[0][1]).toContain('src="/src/main.tsx"');
    expect(scripts[0][2].trim()).toBe("");
    expect(html).not.toMatch(/(src|href)="(https?:)?\/\//);
  });

  it("declares the app version of package.json", () => {
    const pkg = JSON.parse(read("package.json"));
    expect(html).toContain(`<meta name="app-version" content="${pkg.version}" />`);
  });

  it("the dev-only relaxation is limited to `vite serve`", () => {
    const config = read("vite.config.ts");
    expect(config).toMatch(/name: "meshapp:dev-csp",\s*apply: "serve"/);
  });
});

describe.runIf(exists("dist/index.html"))("built bundle (dist/)", () => {
  it("ships the strict CSP and no inline script", () => {
    const html = read("dist/index.html");
    expectStrict(cspOf(html));
    for (const m of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/g)) {
      expect(m[1]).toMatch(/src="\.?\/assets\//);
      expect(m[2].trim()).toBe("");
    }
  });
});
