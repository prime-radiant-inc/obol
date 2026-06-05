# obol — TypeScript/Node/Bun binding (design spec)

> 2026-06-05 · Shevek@7998e83e · draft for Bob review · Linear PRI-2085
> Builds on the C-ABI spine (`2026-06-05-obol-ffi-bindings-design.md`, PRI-2084). The fourth
> consumer of `obol-ffi`, after Python (ctypes) and Go (cgo). Same loop: spec → plan → tests →
> validate, under **both Bun and Node**.

## Goal

A TypeScript binding that estimates transcript cost by calling `obol-ffi` directly, working
under **both Bun and Node** behind one typed API. It re-types the JSON seam exactly like Python
and Go — the Rust core stays the single source of truth; the binding never re-implements
accounting.

**Mechanism: direct FFI over the C-ABI spine** (decided with Matt). Not napi-rs (a parallel
Rust binding that bypasses the spine + a per-platform native build), not spawn-CLI (doesn't
exercise the spine). The spine was built so languages can `dlopen` the cdylib; JS does exactly
that — Bun via built-in `bun:ffi`, Node via `koffi`.

## Both backends are proven (probed against the real cdylib before writing this)

The two mechanisms below were each run against `target/debug/libobol_ffi.dylib` with a seeded
pricing dir and returned `total_usd = 0.000995` (matching Rust/Python/Go), plus code 7 on a bad
dialect. This spec encodes what actually worked, not an API guess.

### Bun (`bun:ffi`, built-in, zero deps)

```ts
import { dlopen, FFIType, ptr, CString } from "bun:ffi";
const lib = dlopen(libPath, {
  obol_version:        { args: [],                                              returns: FFIType.ptr },
  obol_string_free:    { args: [FFIType.ptr],                                   returns: FFIType.void },
  obol_estimate_path:  { args: [FFIType.cstring, FFIType.cstring, FFIType.ptr], returns: FFIType.i32 },
  obol_estimate_bytes: { args: [FFIType.ptr, FFIType.u64, FFIType.cstring, FFIType.ptr], returns: FFIType.i32 },
  obol_refresh_pricing:{ args: [FFIType.cstring, FFIType.ptr],                  returns: FFIType.i32 },
});
```

- **C-string args** (`path`/`dialect`/`as_of`): pass `Buffer.from(s + "\0")`; pass `null` for the
  absent dialect (auto-detect). (`FFIType.cstring` also accepts a JS string, but the explicit
  NUL-terminated buffer is unambiguous.)
- **bytes** (`const uint8_t*`, `uintptr_t len`): `ptr(uint8Array)` as `FFIType.ptr`, length as
  `BigInt(len)` for `FFIType.u64` (`usize` is 64-bit on our targets).
- **`char**` out-param**: allocate a pointer slot `const out = new BigUint64Array(1)`, pass
  `ptr(out)`. After the call the `char*` is `out[0]` (a `bigint`). NULL-guard with `out[0] !== 0n`;
  copy with `new CString(Number(out[0])).toString()`; free with `obol_string_free(Number(out[0]))`.
- **version**: `new CString(lib.symbols.obol_version()).toString()` — static, **never** freed.

### Node (`koffi`, prebuilt binaries, no node-gyp)

```ts
import koffi from "koffi";   // NOT require() — the package is type:module; a bare require throws
                             // "require is not defined in ES module scope". koffi's exports map
                             // supports the default import in 2.16+ and 3.x.
const lib = koffi.load(libPath);
const obol_string_free   = lib.func("void obol_string_free(void* s)");
const obol_estimate_path = lib.func("int obol_estimate_path(const char* path, const char* dialect, _Out_ void** out)");
const obol_estimate_bytes= lib.func("int obol_estimate_bytes(const uint8_t* data, size_t len, const char* dialect, _Out_ void** out)");
const obol_refresh       = lib.func("int obol_refresh_pricing(const char* as_of, _Out_ void** out)");
const obol_version       = lib.func("const char* obol_version()");
```

- **The load-bearing gotcha (verified):** the out-param must be typed **`_Out_ void**`**, NOT
  `char**`. koffi auto-decodes a `char**` out into a JS string and *discards the pointer* — which
  would **leak** obol's allocation (no pointer left to pass to `obol_string_free`). With `void**`,
  koffi yields a raw pointer in `out[0]`.
- **Call**: `const out = [null]; const code = obol_estimate_path(path, dialect, out);` — pass the
  JS string directly (koffi marshals `const char*`), or `null` for auto-detect.
- **Read + free**: `const json = koffi.decode(out[0], "char", -1)` (the `-1` length reads a
  NUL-terminated string, copying it), then `obol_string_free(out[0])`. NULL-guard `out[0]`.
- **bytes**: pass a `Buffer`/`Uint8Array` for `data`, `len` as a JS number. koffi's `size_t`
  prototype is ABI-identical to the header's `uintptr_t`.
- **version**: koffi marshals the `const char*` return directly to a JS string; never freed.

## Architecture

```
bindings/typescript/
  src/
    types.ts      # TS interfaces mirroring the JSON 1:1 (snake_case) + ObolError
    ffi.ts        # runtime detection -> lazily import ONE adapter; defines the FfiBackend interface
    ffi-bun.ts    # bun:ffi adapter (only imported under Bun)
    ffi-node.ts   # koffi adapter   (only imported under Node)
    lib-path.ts   # locate libobol_ffi.<ext> (OBOL_LIB or target/{release,debug})
    index.ts      # public typed API: estimatePath/estimateBytes/refresh/version
  test/
    obol.test.ts  # the matrix, runnable under BOTH `bun test` and `node --test`
  total.ts        # prints total_usd for a transcript (the equivalence gate calls this)
  package.json    # private, type:module, dep: koffi (Bun never imports it)
  tsconfig.json
  README.md
```

### Runtime detection + lazy backend load

`ffi.ts` picks the backend by `typeof Bun !== "undefined"` (equivalently `process.versions.bun`)
and **dynamically `import()`s exactly one adapter**. This is load-bearing: a static
`import "bun:ffi"` would throw under Node (unresolvable specifier), and importing `koffi` under
Bun is needless. The detection + lazy import means Node never touches `bun:ffi` and Bun never
loads `koffi`.

```ts
export interface FfiBackend {
  version(): string;
  estimatePath(path: string, dialect: string | null): { code: number; json: string | null };
  estimateBytes(data: Uint8Array, dialect: string | null): { code: number; json: string | null };
  refresh(asOf: string): { code: number; json: string | null };
}
export async function loadBackend(): Promise<FfiBackend> {
  const isBun = typeof (globalThis as any).Bun !== "undefined";
  // NOTE: specifiers are `.ts`, not `.js`. We run sources directly (no build), and Node 26 does
  // NOT rewrite `.js`→`.ts` — an `./ffi-node.js` import throws ERR_MODULE_NOT_FOUND when only
  // `.ts` exists on disk. Bun tolerates either; Node requires the real `.ts` name.
  const mod = isBun ? await import("./ffi-bun.ts") : await import("./ffi-node.ts");
  return mod.createBackend(resolveLibPath());
}
```

Each adapter owns its copy-then-free dance internally (the contract below) and returns the JSON
string already copied + the pointer already freed. The adapter is the single chokepoint for
memory correctness, so no call site can get it wrong — same discipline as Python's
`_decode_and_free` and Go's `drain`.

### Ownership & safety contract (per backend)

Same contract as every other binding; the JS-specific obligations:
- **Copy before free.** `new CString(p).toString()` (Bun) / `koffi.decode(p, "char", -1)` (Node)
  copy the bytes into a JS string; only then is `obol_string_free` called. The obol-owned pointer
  never outlives the copy.
- **Free exactly once, even on error.** The status code can be nonzero, but `out` is still a
  valid pointer-or-NULL (the FFI's NULL-init invariant). The adapter frees whenever `out` is
  non-NULL, in a `finally`, regardless of code.
- **Never free any other way.** Only `obol_string_free`. (Bun: pass `Number(out[0])`; Node: pass
  the raw `out[0]` pointer.)
- **version is static** — read, never freed.
- **Inputs are borrowed** by obol (copied during the call); JS may drop them immediately after.

### Public API (`index.ts`)

```ts
export function version(): Promise<string>;
export function estimatePath(path: string, dialect?: Dialect): Promise<CostEstimate>;
export function estimateBytes(data: Uint8Array, dialect?: Dialect): Promise<CostEstimate>;
export function refresh(asOf: string): Promise<RefreshReport>;
export type Dialect = "claude" | "codex" | "pi";
```

Async because the backend is `import()`ed lazily. Memoize the **promise** at module scope
(`let backend: Promise<FfiBackend> | undefined`), so concurrent first calls all await the same
import rather than double-loading; every call does `await (backend ??= loadBackend())`. Because
the public API is async and awaits the memoized promise on every call, there is no
"called-before-backend-resolved" hazard. On nonzero status, parse the
`{"error":{code,kind,message}}` envelope and `throw new ObolError(...)`.

Error-code note for the implementer: an explicit bad dialect string returns **code 7**
(`InvalidArgument`, "unknown or invalid dialect string"), *not* code 2. Code 2 (`UnknownDialect`)
is reserved for *auto-detect failed to identify any dialect*. The unknown-dialect test asserts 7
— this is correct; do not "fix" it to 2.

### Types (`types.ts`) — snake_case, mirroring the wire 1:1

```ts
export interface TokenBuckets { input: number; output: number; cache_read: number; cache_write: number; }
export interface ModelCost { model: string; provider: string; tokens: TokenBuckets; subtotal_usd: number; }
export interface Approximation { kind: string; detail?: string; }
export interface CostEstimate {
  total_usd: number; per_model: ModelCost[]; tokens: TokenBuckets;
  unpriced_models: string[]; approximations: Approximation[]; pricing_as_of: string;
}
export interface RefreshReport { models: number; as_of: string; written_to: string; }
export class ObolError extends Error { code: number; kind: string; constructor(code:number,kind:string,message:string){…} }
```

No camelCase transform: the JSON keys are the interface keys, so there is no mapping layer to
drift (the same call Go's struct tags make). `provider` is a string; `approximations` is the
tagged `{kind, detail?}` shape; `token` counts are `number` (well within `2^53` for any real
transcript — noted as a deliberate limitation vs `bigint`).

### Library discovery (`lib-path.ts`)

`OBOL_LIB` env (explicit path) → else walk up from the package to the repo root and try
`target/release/` then `target/debug/` for `libobol_ffi.<ext>` (`.dylib`/`.so`/`.dll` by
`process.platform`). Throw a clear error listing what was tried (mirrors Python's loader).

## Testing — under BOTH runtimes

The same acceptance matrix as Python/Go, but each test must pass under **Bun and Node**:
version; `estimatePath` success (seeded `OBOL_PRICING_DIR` from `bindings/testdata/prices.json`)
asserting `total_usd > 0` and `pricing_as_of === "2026-06-05"`; `estimateBytes` auto-detect;
missing-tables → `ObolError` code 1 (`PricingTablesMissing`); unknown-dialect → code 7.

- **Bun:** `bun test` (native TS, `bun:ffi` built-in).
- **Node:** `node --test test/obol.test.ts` — Node 26 strips TS types natively, so the `.ts`
  runs directly; `koffi` is installed via `npm install`.
- The tests use the standard `node:test`/`node:assert` API, which `bun test` also understands, so
  one test file runs under both. (If a divergence forces it, a thin per-runtime shim is allowed,
  but the assertions stay identical.)

Build the dylib first: `cargo build -p obol-ffi`.

## Acceptance — equivalence gate extended to four languages

Extend `scripts/validate_bindings.sh` to add a **`ts`** total via `bindings/typescript/total.ts`,
run under **both Bun and Node**. The existing gate pipes every consumer's total through one
`norm()` = `python3 -c 'repr(float(...))'`, so `total.ts` need only `console.log` the `total_usd`
in any form Python's `float()` round-trips (e.g. the default number-to-string) — it does *not*
need byte-identical formatting; `norm` collapses representations to one canonical f64 repr. The
gate then asserts `rust == python == go == ts_bun == ts_node` (the same normalized IEEE-754
value). A failure in any consumer fails the gate.

## Linux verification

After macOS, re-verify the whole TS path in the `ubuntu:24.04` container (the existing
`/tmp/obol-linux-verify.sh` pattern): install Bun + Node, `npm install` koffi, build
`libobol_ffi.so`, run the TS tests under both runtimes, and run the four-language gate. The
`bun:ffi`/`koffi` `dlopen` of a `.so` is the Linux-specific thing to confirm.

## Out of scope (this cut)

npm publishing (the package is a private in-tree dev artifact like Python/Go); bundling or
shipping prebuilt dylibs; Deno/browser; napi-rs; `bigint` token counts. TypeScript build tooling
beyond `tsc --noEmit` typecheck — we run sources directly (Bun + Node 26 both execute TS).

## Open threads (small)

- koffi prebuilt-binary coverage for **linux-arm64** in the `ubuntu:24.04` container — koffi ships
  prebuilds broadly; if its target is missing it builds from source (the C toolchain is already
  installed for the cgo path). Confirm during Linux verify.

> Resolved during spec review (Calvin@a1ec3dd5, probed under node 26 / bun 1.3.11):
> - Node runs `node --test test/obol.test.ts` on a `.ts` file **with no flag** (type-stripping is
>   unflagged on Node 26); `bun test` runs the same `node:test`/`node:assert` file. One test file,
>   both runtimes — confirmed.
> - koffi pin: **2.16.2 and 3.0.2 both verified** working (load, `void**`, decode, free, default
>   import). The plan picks one and pins it exactly in `package.json` for a reproducible gate.
> - Both FFI dances (bun:ffi pointer-buffer; koffi `void**`+`decode`+free) verified verbatim →
>   `total_usd 0.000995`, bad-dialect → code 7. The `void**`-not-`char**` call is real (char**
>   auto-stringifies and leaks the pointer).
