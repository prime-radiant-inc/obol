import { dlopen, FFIType, ptr, CString } from "bun:ffi";
import type { FfiBackend, RawResult } from "./ffi.ts";

export function createBackend(libPath: string): FfiBackend {
  const { symbols } = dlopen(libPath, {
    obol_version:         { args: [],                                                          returns: FFIType.ptr },
    obol_string_free:     { args: [FFIType.ptr],                                               returns: FFIType.void },
    obol_estimate_path:   { args: [FFIType.cstring, FFIType.cstring, FFIType.ptr],             returns: FFIType.i32 },
    obol_estimate_bytes:  { args: [FFIType.ptr, FFIType.u64, FFIType.cstring, FFIType.ptr],    returns: FFIType.i32 },
    obol_refresh_pricing: { args: [FFIType.cstring, FFIType.ptr],                              returns: FFIType.i32 },
  });

  const cstr = (s: string | null) => (s === null ? null : Buffer.from(s + "\0"));

  // bun:ffi's ptr() rejects a zero-length view, so use a 1-byte sentinel for empty input and
  // pass the real length (0). The FFI sees a non-NULL data pointer with len 0 — matching the Go
  // binding — instead of bun throwing a raw TypeError before the call.
  const nonEmpty = (data: Uint8Array) => (data.length === 0 ? new Uint8Array(1) : data);

  // Copy the obol-owned string out, then free it. Always frees when out[0] is non-NULL.
  // out[0] is a bigint; Number() narrows it — exact for all real user-space pointers (< 2^53).
  const drain = (code: number, out: BigUint64Array): RawResult => {
    const p = out[0];
    if (p === 0n) return { code, json: null };
    const json = new CString(Number(p)).toString();
    symbols.obol_string_free(Number(p));
    return { code, json };
  };

  return {
    version: () => new CString(symbols.obol_version()).toString(), // static; never freed
    estimatePath(path, dialect) {
      const out = new BigUint64Array(1);
      const code = symbols.obol_estimate_path(cstr(path), cstr(dialect), ptr(out));
      return drain(code, out);
    },
    estimateBytes(data, dialect) {
      const out = new BigUint64Array(1);
      const code = symbols.obol_estimate_bytes(ptr(nonEmpty(data)), BigInt(data.length), cstr(dialect), ptr(out));
      return drain(code, out);
    },
    refresh(asOf) {
      const out = new BigUint64Array(1);
      const code = symbols.obol_refresh_pricing(cstr(asOf), ptr(out));
      return drain(code, out);
    },
  };
}
