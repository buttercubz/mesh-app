// The raw `extern "C"` JSON ABI of mesh_wasm.wasm (DESIGN §11.2), wrapped once:
//
//   uint32_t mw_alloc(uint32_t len);  void mw_free(uint32_t ptr, uint32_t len);
//   uint32_t mw_world_new(uint32_t cfg_ptr, uint32_t cfg_len);
//   uint32_t mw_command(uint32_t world, uint32_t ptr, uint32_t len);
//   void     mw_step(uint32_t world, uint64_t dt_us);
//   uint32_t mw_poll_events(uint32_t world);
//   uint32_t mw_poll_lab(uint32_t world);
//   void     mw_world_free(uint32_t world);
//
// Input buffers are allocated with `mw_alloc`, filled, passed and freed by the caller.
// Returned buffers are `[u32 LE length][UTF-8 bytes]`, owned by the world and valid
// until its next call, so they are decoded at once. Linear memory can grow during any
// call: views are created after the call, never cached.

/** The exported functions, exactly (the module is checked against this list). */
export const ABI_EXPORTS = [
  "mw_alloc",
  "mw_command",
  "mw_free",
  "mw_poll_events",
  "mw_poll_lab",
  "mw_step",
  "mw_world_free",
  "mw_world_new",
] as const;

interface RawExports {
  memory: WebAssembly.Memory;
  mw_alloc(len: number): number;
  mw_free(ptr: number, len: number): void;
  mw_world_new(ptr: number, len: number): number;
  mw_command(world: number, ptr: number, len: number): number;
  mw_step(world: number, dtUs: bigint): void;
  mw_poll_events(world: number): number;
  mw_poll_lab(world: number): number;
  mw_world_free(world: number): void;
}

/** A refused or malformed module, or a call the module could not answer. */
export class MeshWasmError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "MeshWasmError";
  }
}

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8");

function checkExports(exports: WebAssembly.Exports): RawExports {
  const missing: string[] = ABI_EXPORTS.filter((name) => typeof exports[name] !== "function");
  if (!(exports["memory"] instanceof WebAssembly.Memory)) missing.push("memory");
  if (missing.length > 0) {
    throw new MeshWasmError(`mesh_wasm.wasm lacks exports: ${missing.join(", ")}`);
  }
  return exports as unknown as RawExports;
}

/** One instance of mesh_wasm.wasm: worlds are handles into it. */
export class MeshWasm {
  readonly #raw: RawExports;

  constructor(instance: WebAssembly.Instance) {
    this.#raw = checkExports(instance.exports);
  }

  /** Instantiates a compiled module (it imports nothing). */
  static async instantiate(module: WebAssembly.Module): Promise<MeshWasm> {
    const imports = WebAssembly.Module.imports(module);
    if (imports.length > 0) {
      const names = imports.map((i) => `${i.module}.${i.name}`).join(", ");
      throw new MeshWasmError(`mesh_wasm.wasm must not import anything (imports ${names})`);
    }
    return new MeshWasm(await WebAssembly.instantiate(module, {}));
  }

  /** Current size of linear memory, bytes. */
  get memoryBytes(): number {
    return this.#raw.memory.buffer.byteLength;
  }

  /** Copies `text` into a fresh buffer, runs `f(ptr, len)`, frees the buffer. */
  #withInput<T>(text: string, f: (ptr: number, len: number) => T): T {
    const bytes = encoder.encode(text);
    const len = bytes.length;
    const ptr = this.#raw.mw_alloc(len);
    try {
      new Uint8Array(this.#raw.memory.buffer, ptr, len).set(bytes);
      return f(ptr, len);
    } finally {
      this.#raw.mw_free(ptr, len);
    }
  }

  /** Decodes a `[u32 len][bytes]` frame at `ptr`; `undefined` for a null pointer. */
  #frame(ptr: number): string | undefined {
    if (ptr === 0) return undefined;
    const buf = this.#raw.memory.buffer;
    const len = new DataView(buf).getUint32(ptr, true);
    return decoder.decode(new Uint8Array(buf, ptr + 4, len));
  }

  /** Creates a world from the scenario JSON; `0` when the scenario is refused. */
  worldNew(scenarioJson: string): number {
    return this.#withInput(scenarioJson, (p, l) => this.#raw.mw_world_new(p, l));
  }

  /** Runs one JSON command; `undefined` when the world does not exist. */
  command(world: number, json: string): string | undefined {
    const ptr = this.#withInput(json, (p, l) => this.#raw.mw_command(world, p, l));
    return this.#frame(ptr);
  }

  /** Advances simulated time by `dtUs` microseconds. */
  step(world: number, dtUs: number): void {
    const us = Math.max(0, Math.floor(dtUs));
    this.#raw.mw_step(world, BigInt(us));
  }

  /** The JSON array of pending UI events (`undefined`: unknown world). */
  pollEvents(world: number): string | undefined {
    return this.#frame(this.#raw.mw_poll_events(world));
  }

  /** labrun/1 JSONL written since the last poll (`undefined`: unknown world). */
  pollLab(world: number): string | undefined {
    return this.#frame(this.#raw.mw_poll_lab(world));
  }

  /** Frees a world (its handle may be reused). */
  worldFree(world: number): void {
    this.#raw.mw_world_free(world);
  }
}
