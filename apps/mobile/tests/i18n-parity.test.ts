// i18n infrastructure and catalog parity (WP-18, DESIGN §11.5, §13.1): every catalog
// has ES/EN parity, the engine key sets (ErrorCode, NoticeKey, PromptKey incl.
// demo.line1-8) are complete, and interpolation placeholders match across languages.
import { ERROR_CODES, NOTICE_KEYS, PROMPT_KEYS } from "@meshapp/engine-api";
import { describe, expect, it } from "vitest";

import { APP_CATALOGS, SHELL_NAMESPACES, catalogOf, namespaceOf } from "../src/app/i18n";
import {
  applyLanguage,
  catalogParityErrors,
  defineCatalog,
  isCatalog,
  normalizeLanguage,
  registeredNamespaces,
  translate,
} from "../src/i18n/index";
import { promptsCatalog } from "../src/i18n/catalogs/prompts";

const placeholders = (text: string) => [...text.matchAll(/\{\{\s*([\w.]+)\s*\}\}/g)].map((m) => m[1]).sort();

describe("defineCatalog", () => {
  it("accepts both call forms and brands the result", () => {
    const a = defineCatalog({ en: { hi: "Hi" }, es: { hi: "Hola" } });
    const b = defineCatalog({ hi: "Hi" }, { hi: "Hola" });
    expect(a.en.hi).toBe("Hi");
    expect(b.es.hi).toBe("Hola");
    expect(isCatalog(a)).toBe(true);
    expect(isCatalog(b)).toBe(true);
    expect(isCatalog({ en: {}, es: {} })).toBe(false);
  });

  it("rejects missing and extra Spanish keys at compile time", () => {
    // @ts-expect-error -- Spanish is missing "bye".
    defineCatalog({ en: { hi: "Hi", bye: "Bye" }, es: { hi: "Hola" } });
    // @ts-expect-error -- Spanish has a key English does not have.
    defineCatalog({ en: { hi: "Hi" }, es: { hi: "Hola", extra: "x" } });
    expect(true).toBe(true);
  });

  it("reports runtime parity problems", () => {
    const broken = { en: { a: "A", b: "B" }, es: { a: "", c: "C" } } as never;
    expect(catalogParityErrors(broken)).toEqual(['es "a" is empty', 'es is missing "b"', 'es has extra key "c"']);
  });

  it("finds exactly one catalog per file", () => {
    const c = defineCatalog({ en: { x: "X" }, es: { x: "X" } });
    expect(catalogOf("f.ts", { c, alias: c, other: 1 })).toBe(c);
    expect(() => catalogOf("f.ts", { other: 1 })).toThrow(/exactly one/);
    expect(namespaceOf("../i18n/catalogs/ride.ts")).toBe("ride");
  });
});

describe("registered catalogs", () => {
  it("registers the shell namespaces (and any feature namespace found)", () => {
    for (const ns of SHELL_NAMESPACES) expect(registeredNamespaces()).toContain(ns);
    expect(Object.keys(APP_CATALOGS).sort()).toEqual(registeredNamespaces());
  });

  for (const [ns, catalog] of Object.entries(APP_CATALOGS)) {
    it(`${ns}: ES/EN parity, no empty strings, same placeholders`, () => {
      expect(catalogParityErrors(catalog)).toEqual([]);
      for (const [key, en] of Object.entries(catalog.en)) {
        expect(placeholders(catalog.es[key] ?? ""), `${ns}.${key}`).toEqual(placeholders(en));
      }
    });
  }

  it("errors covers every ErrorCode plus `unknown`", () => {
    const errors = APP_CATALOGS["errors"];
    for (const code of [...ERROR_CODES, "unknown"]) {
      expect(Object.hasOwn(errors?.en ?? {}, code), code).toBe(true);
      expect(Object.hasOwn(errors?.es ?? {}, code), code).toBe(true);
    }
  });

  it("notices covers every NoticeKey", () => {
    const notices = APP_CATALOGS["notices"];
    expect(Object.keys(notices?.en ?? {}).sort()).toEqual([...NOTICE_KEYS].sort());
  });

  it("prompts holds exactly the PromptKeys, including demo.line1-8", () => {
    expect(Object.keys(promptsCatalog.en).sort()).toEqual([...PROMPT_KEYS].sort());
    expect(Object.keys(promptsCatalog.es).sort()).toEqual([...PROMPT_KEYS].sort());
    for (let i = 1; i <= 8; i += 1) {
      expect(promptsCatalog.en[`demo.line${i}` as keyof typeof promptsCatalog.en]).toBeTruthy();
      expect(promptsCatalog.es[`demo.line${i}` as keyof typeof promptsCatalog.es]).toBeTruthy();
    }
  });

  it("prompts use the params the engine fills ({n}, {name})", () => {
    expect(placeholders(promptsCatalog.en.intercomLive)).toEqual(["n"]);
    expect(placeholders(promptsCatalog.en.ridersOutOfRange)).toEqual(["n"]);
    expect(placeholders(promptsCatalog.en.memberJoined)).toEqual(["name"]);
    expect(placeholders(promptsCatalog.en.leaderChanged)).toEqual(["name"]);
  });
});

describe("translation", () => {
  it("translates with dotted keys, params and the current language", () => {
    applyLanguage("es");
    expect(translate("notices", "memberJoined", { n: 1, names: "Ana" })).toBe("Se unió a la rodada: Ana");
    expect(translate("prompts", "demo.line1")).toMatch(/motorista de prueba/);
    applyLanguage("en");
    expect(translate("notices", "memberJoined", { n: 1, names: "Ana" })).toBe("Joined: Ana");
    expect(translate("common", "action.back")).toBe("Back");
    expect(translate("notices", "memberJoined", { n: 1, names: "Ana" }, "es")).toBe("Se unió a la rodada: Ana");
  });

  it("does not HTML-escape params (React escapes)", () => {
    expect(translate("notices", "memberJoined", { n: 1, names: "A&B" }, "en")).toBe("Joined: A&B");
  });

  it("normalises languages", () => {
    expect(normalizeLanguage("es-DO")).toBe("es");
    expect(normalizeLanguage("ES")).toBe("es");
    expect(normalizeLanguage("fr")).toBe("en");
    expect(normalizeLanguage(undefined)).toBe("en");
  });
});
