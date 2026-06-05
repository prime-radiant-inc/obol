import koffi from "koffi";
import type { FfiBackend, RawResult } from "./ffi.ts";

export function createBackend(libPath: string): FfiBackend {
  const lib = koffi.load(libPath);
  const obol_version = lib.func("const char* obol_version()");
  const obol_string_free = lib.func("void obol_string_free(void* s)");
  // out-param is void** (NOT char**): char** makes koffi auto-stringify and lose the pointer → leak.
  const obol_estimate_path = lib.func(
    "int obol_estimate_path(const char* path, const char* dialect, _Out_ void** out)",
  );
  const obol_estimate_bytes = lib.func(
    "int obol_estimate_bytes(const uint8_t* data, size_t len, const char* dialect, _Out_ void** out)",
  );
  const obol_refresh = lib.func("int obol_refresh_pricing(const char* as_of, _Out_ void** out)");

  // Use a 1-byte sentinel for empty input so the FFI sees a non-NULL data pointer with len 0
  // (matching the Go binding and the Bun adapter) rather than koffi passing NULL → code 7.
  const nonEmpty = (data: Uint8Array) => (data.length === 0 ? Buffer.alloc(1) : data);

  // Copy the obol-owned string out, then free it. Always frees when the pointer is non-NULL.
  const drain = (code: number, out: [unknown]): RawResult => {
    const p = out[0];
    if (p === null || p === undefined) return { code, json: null };
    const json = koffi.decode(p, "char", -1) as string;
    obol_string_free(p);
    return { code, json };
  };

  return {
    version: () => obol_version() as string, // koffi marshals const char* return to a JS string
    estimatePath(path, dialect) {
      const out: [unknown] = [null];
      const code = obol_estimate_path(path, dialect, out) as number;
      return drain(code, out);
    },
    estimateBytes(data, dialect) {
      const out: [unknown] = [null];
      const code = obol_estimate_bytes(nonEmpty(data), data.length, dialect, out) as number;
      return drain(code, out);
    },
    refresh(asOf) {
      const out: [unknown] = [null];
      const code = obol_refresh(asOf, out) as number;
      return drain(code, out);
    },
  };
}
