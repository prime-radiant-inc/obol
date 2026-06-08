import { dlopen, FFIType, ptr, CString } from "bun:ffi";
import type { FfiBackend, RawResult } from "./ffi.ts";

export function createBackend(libPath: string): FfiBackend {
  const { symbols } = dlopen(libPath, {
    obol_version:         { args: [],                                                          returns: FFIType.ptr },
    obol_string_free:     { args: [FFIType.ptr],                                               returns: FFIType.void },
    obol_estimate_path:   { args: [FFIType.cstring, FFIType.cstring, FFIType.ptr],             returns: FFIType.i32 },
    obol_refresh_pricing: { args: [FFIType.cstring, FFIType.ptr],                              returns: FFIType.i32 },
  });

  const cstr = (s: string | null) => (s === null ? null : Buffer.from(s + "\0"));

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
    refresh(asOf) {
      const out = new BigUint64Array(1);
      const code = symbols.obol_refresh_pricing(cstr(asOf), ptr(out));
      return drain(code, out);
    },
  };
}
