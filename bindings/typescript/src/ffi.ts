import { resolveLibPath } from "./lib-path.ts";

export interface RawResult {
  code: number;
  json: string | null;
}
export interface FfiBackend {
  version(): string;
  estimatePath(path: string, dialect: string | null): RawResult;
  estimateBytes(data: Uint8Array, dialect: string | null): RawResult;
  refresh(asOf: string): RawResult;
}

let cached: Promise<FfiBackend> | undefined;

/** Resolve the backend once; concurrent first calls await the same import. */
export function backend(): Promise<FfiBackend> {
  return (cached ??= load());
}

async function load(): Promise<FfiBackend> {
  const isBun = typeof (globalThis as { Bun?: unknown }).Bun !== "undefined";
  const libPath = resolveLibPath();
  // .ts specifiers (no build step); only the taken branch is ever imported, so Node never
  // resolves bun:ffi and Bun never loads koffi.
  const mod = isBun ? await import("./ffi-bun.ts") : await import("./ffi-node.ts");
  return (mod as { createBackend(p: string): FfiBackend }).createBackend(libPath);
}
