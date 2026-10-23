// Loads mesh_wasm.wasm in a browser (fetch, streaming compile when the server sends
// `application/wasm`) or in Node/Vitest (read from disk). The module ships in the
// package's wasm/ directory, written by `cargo xtask build-wasm`; dist/loader.js finds
// it at ../wasm/mesh_wasm.wasm, which bundlers (Vite) turn into an asset URL.
import { MeshWasm, MeshWasmError } from "./abi.js";

/** Where the module comes from (default: the copy shipped with this package). */
export interface LoadOptions {
  /** Already compiled module (shared between instances, tests). */
  module?: WebAssembly.Module;
  /** Module bytes. */
  bytes?: ArrayBuffer | Uint8Array;
  /** URL (browser) or file URL / path (Node) of mesh_wasm.wasm. */
  url?: string | URL;
}

/** The module shipped with this package. */
export function defaultWasmUrl(): URL {
  return new URL("../wasm/mesh_wasm.wasm", import.meta.url);
}

function isNode(): boolean {
  const p = (globalThis as { process?: { versions?: { node?: string } } }).process;
  return typeof p?.versions?.node === "string";
}

/** Reads a local file in Node without making the browser bundle depend on node:fs. */
async function readNodeFile(url: URL | string): Promise<Uint8Array> {
  const fsName = "node:fs/promises";
  const urlName = "node:url";
  const fs = (await import(/* @vite-ignore */ fsName)) as {
    readFile(path: string): Promise<Uint8Array>;
  };
  const { fileURLToPath } = (await import(/* @vite-ignore */ urlName)) as {
    fileURLToPath(url: URL | string): string;
  };
  const path = typeof url === "string" && !url.startsWith("file:") ? url : fileURLToPath(url);
  return fs.readFile(path);
}

function toArrayBuffer(bytes: ArrayBuffer | Uint8Array): ArrayBuffer {
  if (bytes instanceof ArrayBuffer) return bytes;
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer;
}

/** Compiles mesh_wasm.wasm from the given source. */
export async function compileMeshWasm(opts: LoadOptions = {}): Promise<WebAssembly.Module> {
  if (opts.module !== undefined) return opts.module;
  if (opts.bytes !== undefined) return WebAssembly.compile(toArrayBuffer(opts.bytes));
  const url = opts.url ?? defaultWasmUrl();
  const href = url.toString();
  if (isNode() && (href.startsWith("file:") || !/^[a-z]+:/i.test(href))) {
    return WebAssembly.compile(toArrayBuffer(await readNodeFile(url)));
  }
  const response = await fetch(href);
  if (!response.ok) {
    throw new MeshWasmError(`could not load ${href}: HTTP ${response.status}`);
  }
  const type = response.headers.get("content-type") ?? "";
  if (typeof WebAssembly.compileStreaming === "function" && type.startsWith("application/wasm")) {
    return WebAssembly.compileStreaming(response);
  }
  return WebAssembly.compile(await response.arrayBuffer());
}

let shared: Promise<WebAssembly.Module> | undefined;

/**
 * Loads and instantiates mesh_wasm.wasm. Without options the compiled default module
 * is cached for the page, so every convoy shares one compilation (each gets its own
 * instance and memory).
 */
export async function loadMeshWasm(opts: LoadOptions = {}): Promise<MeshWasm> {
  const custom = opts.module !== undefined || opts.bytes !== undefined || opts.url !== undefined;
  let module: WebAssembly.Module;
  if (custom) {
    module = await compileMeshWasm(opts);
  } else {
    shared ??= compileMeshWasm();
    try {
      module = await shared;
    } catch (e) {
      // Let the next call retry (a transient network error must not stick).
      shared = undefined;
      throw e;
    }
  }
  return MeshWasm.instantiate(module);
}
