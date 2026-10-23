// Minimal JSON Schema (draft 2020-12) validator for the labrun/1 schema (contract C7,
// crates/mesh-lab/schema/labrun-1.schema.json). It implements exactly the keywords that
// schema uses: type, enum, const, required, properties, additionalProperties, items,
// prefixItems, minItems, maxItems, minimum, maximum, minLength, maxLength, pattern,
// propertyNames, maxProperties, oneOf, anyOf and local $ref (#/$defs/…). The same
// validator checks ridelog/1 (crates/mesh-lab/schema/ridelog-1.schema.json). The Rust contract test validates the same
// lines with a full validator (boon); this one lets lab-analyze reject bad input
// without npm dependencies.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

export const SCHEMA_PATH = fileURLToPath(
  new URL("../../../crates/mesh-lab/schema/labrun-1.schema.json", import.meta.url),
);

export const RIDELOG_SCHEMA_PATH = fileURLToPath(
  new URL("../../../crates/mesh-lab/schema/ridelog-1.schema.json", import.meta.url),
);

/** Loads the ridelog/1 schema from the mesh-lab crate. */
export function loadRideSchema(path = RIDELOG_SCHEMA_PATH) {
  return JSON.parse(readFileSync(path, "utf8"));
}

/** Loads the labrun/1 schema from the mesh-lab crate. */
export function loadSchema(path = SCHEMA_PATH) {
  return JSON.parse(readFileSync(path, "utf8"));
}

const typeOf = (v) => {
  if (v === null) return "null";
  if (Array.isArray(v)) return "array";
  if (typeof v === "number") return Number.isInteger(v) ? "integer" : "number";
  return typeof v;
};

const typeMatches = (want, v) => {
  const got = typeOf(v);
  return want === got || (want === "number" && got === "integer");
};

const deepEqual = (a, b) => JSON.stringify(a) === JSON.stringify(b);

function resolveRef(root, ref) {
  if (!ref.startsWith("#/")) throw new Error(`unsupported $ref ${ref}`);
  return ref
    .slice(2)
    .split("/")
    .reduce((node, key) => {
      if (node === undefined || !(key in node)) throw new Error(`unresolved $ref ${ref}`);
      return node[key];
    }, root);
}

/**
 * Validates `value` against `schema` (the root is `root`). Returns a list of errors
 * (`{ path, message }`), empty when valid.
 */
export function validate(value, schema, root = schema, path = "$") {
  const errors = [];
  const fail = (message) => errors.push({ path, message });
  if (schema === true) return errors;
  if (schema === false) {
    fail("not allowed");
    return errors;
  }
  if (schema.$ref) errors.push(...validate(value, resolveRef(root, schema.$ref), root, path));
  if (schema.type !== undefined) {
    const types = Array.isArray(schema.type) ? schema.type : [schema.type];
    if (!types.some((t) => typeMatches(t, value))) fail(`expected ${types.join("|")}, got ${typeOf(value)}`);
  }
  if (schema.const !== undefined && !deepEqual(schema.const, value)) fail(`expected ${JSON.stringify(schema.const)}`);
  if (schema.enum && !schema.enum.some((e) => deepEqual(e, value))) fail(`not one of ${JSON.stringify(schema.enum)}`);
  if (typeof value === "number") {
    if (schema.minimum !== undefined && value < schema.minimum) fail(`below minimum ${schema.minimum}`);
    if (schema.maximum !== undefined && value > schema.maximum) fail(`above maximum ${schema.maximum}`);
  }
  if (typeof value === "string") {
    const len = [...value].length;
    if (schema.minLength !== undefined && len < schema.minLength) fail(`shorter than ${schema.minLength}`);
    if (schema.maxLength !== undefined && len > schema.maxLength) fail(`longer than ${schema.maxLength}`);
    if (schema.pattern !== undefined && !new RegExp(schema.pattern, "u").test(value)) {
      fail(`does not match ${schema.pattern}`);
    }
  }
  if (Array.isArray(value)) {
    if (schema.minItems !== undefined && value.length < schema.minItems) fail(`fewer than ${schema.minItems} items`);
    if (schema.maxItems !== undefined && value.length > schema.maxItems) fail(`more than ${schema.maxItems} items`);
    const prefix = schema.prefixItems ?? [];
    value.forEach((item, i) => {
      const s = i < prefix.length ? prefix[i] : schema.items;
      if (s !== undefined) errors.push(...validate(item, s, root, `${path}[${i}]`));
    });
  }
  if (typeOf(value) === "object") {
    for (const key of schema.required ?? []) {
      if (!(key in value)) fail(`missing required "${key}"`);
    }
    if (schema.maxProperties !== undefined && Object.keys(value).length > schema.maxProperties) {
      fail(`more than ${schema.maxProperties} properties`);
    }
    if (schema.propertyNames !== undefined) {
      for (const key of Object.keys(value)) {
        errors.push(...validate(key, schema.propertyNames, root, `${path}.${key}(name)`));
      }
    }
    const props = schema.properties ?? {};
    for (const [key, v] of Object.entries(value)) {
      if (key in props) {
        errors.push(...validate(v, props[key], root, `${path}.${key}`));
      } else if (schema.additionalProperties !== undefined) {
        errors.push(...validate(v, schema.additionalProperties, root, `${path}.${key}`));
      }
    }
  }
  if (schema.anyOf && !schema.anyOf.some((s) => validate(value, s, root, path).length === 0)) {
    fail("matches none of anyOf");
  }
  if (schema.oneOf) {
    const results = schema.oneOf.map((s) => validate(value, s, root, path));
    const ok = results.filter((r) => r.length === 0).length;
    if (ok !== 1) {
      // Report the branch that came closest, which is the useful message.
      const closest = results.reduce((a, b) => (b.length < a.length ? b : a));
      if (ok === 0) errors.push(...closest);
      else fail(`matches ${ok} branches of oneOf`);
    }
  }
  return errors;
}
