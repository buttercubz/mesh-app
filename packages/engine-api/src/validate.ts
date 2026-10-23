// Minimal runtime validators that mirror the serde rules of crates/mesh-api.
//
// A `Validator<T>` checks an `unknown` JSON value and returns the first problem as a
// string with its JSON path, or `undefined` when the value is a valid `T`. Object
// validators are typed against the generated types, so a field added on the Rust side
// fails `tsc` here until the guard is updated.

/** Checks `value` (at `path`) and returns the first problem, or `undefined` if valid. */
export interface Validator<T> {
  readonly check: (value: unknown, path: string) => string | undefined;
  /** Phantom marker carrying the validated type; never set at run time. */
  readonly __type?: T;
}

/** The type a validator accepts. */
export type Infer<V> = V extends Validator<infer T> ? T : never;

/** A field that may be absent, `undefined` or `null` (serde `Option` + `skip_serializing_if`). */
export interface OptionalField<T> {
  readonly optional: Validator<T>;
}

type Field<T> = undefined extends T ? OptionalField<Exclude<T, undefined>> : Validator<T>;

/** One validator per key of `T`; optional keys use `opt(...)`. */
export type Shape<T> = { readonly [K in keyof T]-?: Field<T[K]> };

const describe = (v: unknown): string =>
  v === null ? "null" : Array.isArray(v) ? "array" : typeof v;

const make = <T>(check: Validator<T>["check"]): Validator<T> => ({ check });

const isPlainObject = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v);

/** Any string. */
export const str = (): Validator<string> =>
  make((v, p) => (typeof v === "string" ? undefined : `${p}: expected string, got ${describe(v)}`));

/** Any finite number (Rust `f64`). */
export const num = (): Validator<number> =>
  make((v, p) =>
    typeof v === "number" && Number.isFinite(v)
      ? undefined
      : `${p}: expected a finite number, got ${describe(v)}`,
  );

/** An integer in `[min, max]` (Rust `u8`/`u16`/`u32`/`i16`). */
export const int = (min: number, max: number): Validator<number> =>
  make((v, p) =>
    typeof v === "number" && Number.isInteger(v) && v >= min && v <= max
      ? undefined
      : `${p}: expected an integer in [${min}, ${max}], got ${JSON.stringify(v) ?? describe(v)}`,
  );

/** Integer ranges of the Rust field types. */
export const u8 = () => int(0, 0xff);
export const u16 = () => int(0, 0xffff);
export const u32 = () => int(0, 0xffff_ffff);
export const i16 = () => int(-0x8000, 0x7fff);

/** `true` or `false`. */
export const bool = (): Validator<boolean> =>
  make((v, p) => (typeof v === "boolean" ? undefined : `${p}: expected boolean, got ${describe(v)}`));

/** Exactly `value`. */
export const lit = <const L extends string | boolean>(value: L): Validator<L> =>
  make((v, p) => (v === value ? undefined : `${p}: expected ${JSON.stringify(value)}, got ${JSON.stringify(v) ?? describe(v)}`));

/** One of a closed string set (a generated `*_VALUES`/`*_KEYS` list). */
export const oneOf = <const T extends string>(values: readonly T[]): Validator<T> => {
  const set = new Set<string>(values);
  return make((v, p) =>
    typeof v === "string" && set.has(v)
      ? undefined
      : `${p}: ${JSON.stringify(v) ?? describe(v)} is not one of ${values.length} allowed values`,
  );
};

/** `T | null` (Rust `Option<T>` without `skip_serializing_if`). */
export const nullable = <T>(inner: Validator<T>): Validator<T | null> =>
  make((v, p) => (v === null ? undefined : inner.check(v, p)));

/** An optional object field (`?: T`). */
export const opt = <T>(inner: Validator<T>): OptionalField<T | null> => ({
  optional: nullable(inner),
});

/** `T[]`. */
export const arr = <T>(item: Validator<T>): Validator<T[]> =>
  make((v, p) => {
    if (!Array.isArray(v)) return `${p}: expected array, got ${describe(v)}`;
    for (let i = 0; i < v.length; i++) {
      const err = item.check(v[i], `${p}[${i}]`);
      if (err !== undefined) return err;
    }
    return undefined;
  });

/** `{ [key in K]?: V }` (Rust `BTreeMap<K, V>`); `keys` limits the allowed keys. */
export const record = <K extends string, V>(
  value: Validator<V>,
  keys?: Validator<K>,
): Validator<{ [key in K]?: V }> =>
  make((v, p) => {
    if (!isPlainObject(v)) return `${p}: expected object, got ${describe(v)}`;
    for (const [k, item] of Object.entries(v)) {
      const keyErr = keys?.check(k, `${p} key`);
      if (keyErr !== undefined) return keyErr;
      const err = value.check(item, `${p}.${k}`);
      if (err !== undefined) return err;
    }
    return undefined;
  });

/** An object with exactly the keys of `shape` (serde `deny_unknown_fields`). */
export const obj = <T>(shape: Shape<T>): Validator<T> => {
  const entries = Object.entries(shape) as [string, Validator<unknown> | OptionalField<unknown>][];
  const known = new Set(entries.map(([k]) => k));
  return make((v, p) => {
    if (!isPlainObject(v)) return `${p}: expected object, got ${describe(v)}`;
    for (const k of Object.keys(v)) {
      if (!known.has(k)) return `${p}: unknown field \`${k}\``;
    }
    for (const [k, field] of entries) {
      const fieldPath = `${p}.${k}`;
      if ("optional" in field) {
        if (v[k] === undefined) continue;
        const err = field.optional.check(v[k], fieldPath);
        if (err !== undefined) return err;
      } else {
        if (!(k in v)) return `${p}: missing field \`${k}\``;
        const err = field.check(v[k], fieldPath);
        if (err !== undefined) return err;
      }
    }
    return undefined;
  });
};

/** A union discriminated by the string field `tag`; one validator per tag value. */
export const tagged = <U extends Record<Tag, string>, Tag extends string>(
  tag: Tag,
  variants: { readonly [K in U[Tag]]: Validator<Extract<U, Record<Tag, K>>> },
): Validator<U> => {
  const table = variants as Record<string, Validator<U> | undefined>;
  return make((v, p) => {
    if (!isPlainObject(v)) return `${p}: expected object, got ${describe(v)}`;
    const t = v[tag];
    if (typeof t !== "string") return `${p}: missing string field \`${tag}\``;
    const variant = Object.hasOwn(table, t) ? table[t] : undefined;
    if (variant === undefined) return `${p}: unknown ${tag} ${JSON.stringify(t)}`;
    return variant.check(v, p);
  });
};

/** The first validator that accepts the value (serde `untagged`, tried in order). */
export const anyOf = <T>(label: string, options: readonly Validator<T>[]): Validator<T> =>
  make((v, p) => {
    const errors: string[] = [];
    for (const option of options) {
      const err = option.check(v, p);
      if (err === undefined) return undefined;
      errors.push(err);
    }
    return `${p}: not a valid ${label} (${errors.join("; ")})`;
  });

/** Runs `validator` on `value`; returns the typed value or the first problem. */
export function validate<T>(
  validator: Validator<T>,
  value: unknown,
  root = "$",
): { ok: true; value: T } | { ok: false; error: string } {
  const error = validator.check(value, root);
  return error === undefined ? { ok: true, value: value as T } : { ok: false, error };
}
