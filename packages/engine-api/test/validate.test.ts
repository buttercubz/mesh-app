// Unit tests of the validator combinators (serde-equivalent rules).
import { describe, expect, it } from "vitest";
import {
  anyOf,
  arr,
  i16,
  lit,
  nullable,
  num,
  obj,
  oneOf,
  opt,
  record,
  str,
  tagged,
  u16,
  u8,
  validate,
} from "../src/validate.js";

describe("validators", () => {
  it("integers follow the Rust ranges", () => {
    expect(u8().check(255, "$")).toBeUndefined();
    expect(u8().check(256, "$")).toMatch(/integer in \[0, 255\]/);
    expect(u16().check(1.5, "$")).toBeDefined();
    expect(u16().check(-1, "$")).toBeDefined();
    expect(i16().check(-61, "$")).toBeUndefined();
    expect(num().check(Number.NaN, "$")).toBeDefined();
    expect(num().check("1", "$")).toBeDefined();
  });

  it("objects are exact; required nullable fields must be present", () => {
    type T = { a: string; b: number | null; c?: boolean | null };
    const v = obj<T>({ a: str(), b: nullable(num()), c: opt(lit(true)) });
    expect(v.check({ a: "x", b: null }, "$")).toBeUndefined();
    expect(v.check({ a: "x", b: 1, c: true }, "$")).toBeUndefined();
    expect(v.check({ a: "x", b: 1, c: null }, "$")).toBeUndefined();
    expect(v.check({ a: "x" }, "$")).toBe("$: missing field `b`");
    expect(v.check({ a: "x", b: 1, d: 0 }, "$")).toBe("$: unknown field `d`");
    expect(v.check({ a: "x", b: 1, c: false }, "$.o")).toMatch(/^\$\.o\.c: /);
    expect(v.check([], "$")).toBeDefined();
  });

  it("tagged unions dispatch on the tag and ignore inherited keys", () => {
    type U = { k: "a"; x: number } | { k: "b" };
    const v = tagged<U, "k">("k", {
      a: obj<{ k: "a"; x: number }>({ k: lit("a"), x: num() }),
      b: obj<{ k: "b" }>({ k: lit("b") }),
    });
    expect(v.check({ k: "a", x: 1 }, "$")).toBeUndefined();
    expect(v.check({ k: "b", x: 1 }, "$")).toBeDefined();
    expect(v.check({ k: "toString" }, "$")).toMatch(/unknown k/);
    expect(v.check({ x: 1 }, "$")).toMatch(/missing string field/);
  });

  it("records, arrays, enums and untagged unions", () => {
    const keys = oneOf(["x", "y"] as const);
    expect(record(str(), keys).check({ x: "1" }, "$")).toBeUndefined();
    expect(record(str(), keys).check({ z: "1" }, "$")).toBeDefined();
    expect(arr(u8()).check([1, 2, 300], "$")).toBe("$[2]: expected an integer in [0, 255], got 300");
    const sn = anyOf<string | number>("ParamValue", [str(), num()]);
    expect(sn.check(3, "$")).toBeUndefined();
    expect(sn.check(true, "$")).toMatch(/not a valid ParamValue/);
    expect(validate(sn, "a")).toEqual({ ok: true, value: "a" });
    expect(validate(sn, null).ok).toBe(false);
  });
});
