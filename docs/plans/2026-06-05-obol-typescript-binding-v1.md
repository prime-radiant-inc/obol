# obol TypeScript/Node/Bun binding — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to
> implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A TypeScript binding for obol that runs under **both Bun and Node**, reaching the Rust
core through the `obol-ffi` C ABI (Bun via `bun:ffi`, Node via `koffi`), producing the same
`total_usd` as the Rust CLI, Python, and Go.

**Architecture:** One package, runtime-detected. A lazy `import()` keyed on `typeof Bun` loads
exactly one adapter; both adapters expose the same `FfiBackend` seam (`version`/`estimatePath`/
`estimateBytes`/`refresh` → `{code, json}`). `index.ts` re-types the JSON into TS interfaces
(snake_case, 1:1 with the wire) and throws `ObolError`. The Rust core stays the single source of
truth; the binding re-types, never re-implements. Spec:
`docs/specs/2026-06-05-obol-typescript-binding-design.md` — both FFI dances in it were probed
against the real cdylib before this plan was written.

**Tech stack:** Node 26 (native TS type-stripping — runs `.ts` directly, `node --test` needs no
flag), Bun 1.3 (`bun:ffi` built-in), `koffi` 2.16.2 (Node FFI; probed). Rust 1.96 via
`mise exec rust@1.96.0 -- cargo …`. The cdylib must be built first: `cargo build -p obol-ffi`.

**Three tasks** (Linux container verification + merge are done by the orchestrator after):
- Task 1: the Bun-working binding (scaffold + shared core + `bun:ffi` adapter + public API + tests under Bun).
- Task 2: the Node adapter (`koffi`) + the SAME tests under Node + README.
- Task 3: `total.ts` + the four-language equivalence gate (`rust==py==go==ts_bun==ts_node`) + validation doc.

**House values:** simple but high-quality; no over-engineering, no crufty shortcuts; pre-1.0 zero
users (no back-compat shims). One commit per task.

---

## Critical facts (verified against the real cdylib — do not regress)

- **Bun out-param:** `char**` is read via a pointer slot `new BigUint64Array(1)` + `ptr(out)`;
  the `char*` is `out[0]` (a `bigint`), copied with `new CString(Number(out[0]))`, freed with
  `obol_string_free(Number(out[0]))`. `usize` len is `BigInt(len)` (`FFIType.u64`).
- **Node (koffi) out-param MUST be `_Out_ void**`, NOT `char**`.** `char**` makes koffi
  auto-decode to a string and *discards the pointer* → leak. With `void**`, `out[0]` is a raw
  pointer; copy with `koffi.decode(out[0], "char", -1)`, free with `obol_string_free(out[0])`
  (declared `void obol_string_free(void* s)`).
- **Module resolution:** the package is `type:module`. Node adapter uses `import koffi from "koffi"`
  (NOT `require`). Dynamic imports use **`.ts`** specifiers (`./ffi-bun.ts`), not `.js` — there is
  no build step and Node won't rewrite `.js`→`.ts`.
- **Bad dialect string → code 7** (`InvalidArgument`), not code 2. The unknown-dialect test
  asserts 7; that is correct.
- Fixtures already exist: `bindings/testdata/prices.json` (prices `claude-opus-4-8`) and
  `bindings/testdata/claude-mini.jsonl`. Seed `OBOL_PRICING_DIR` by copying `prices.json` →
  `$DIR/current.json`. Expected `total_usd = 0.000995`, `pricing_as_of = "2026-06-05"`.
- **Bun does NOT propagate runtime `process.env` writes to the native environment** that
  `bun:ffi`'s C calls see — so Rust's `getenv("OBOL_PRICING_DIR")` (read per-call) ignores a
  `process.env.OBOL_PRICING_DIR = …` set *after* startup under Bun (Node propagates fine). Tests
  that change the pricing dir at runtime therefore MUST route through a cross-runtime helper that,
  under Bun, calls libc `setenv`/`unsetenv` via FFI (Task 1 Step 9). Setting the env in the shell
  *before* launching (as the equivalence gate does) is unaffected — only runtime JS mutation needs
  the helper. This is the single non-obvious trap in this binding.

---

## Task 1: Bun-working binding (scaffold + shared core + bun:ffi adapter + public API)

**Files:**
- Create: `bindings/typescript/package.json`
- Create: `bindings/typescript/tsconfig.json`
- Create: `bindings/typescript/src/types.ts`
- Create: `bindings/typescript/src/lib-path.ts`
- Create: `bindings/typescript/src/ffi.ts`
- Create: `bindings/typescript/src/ffi-bun.ts`
- Create: `bindings/typescript/src/index.ts`
- Create: `bindings/typescript/test/pricing-env.ts`
- Create: `bindings/typescript/test/obol.test.ts`

- [ ] **Step 1: Build the cdylib** (the binding loads it): `mise exec rust@1.96.0 -- cargo build -p obol-ffi`. Confirm `target/debug/libobol_ffi.dylib` exists.

- [ ] **Step 2: Write `bindings/typescript/package.json`:**

```json
{
  "name": "obol",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "description": "Agent-transcript cost estimation — TypeScript binding over the obol C ABI (Bun + Node).",
  "exports": { ".": "./src/index.ts" },
  "dependencies": {
    "koffi": "2.16.2"
  }
}
```

- [ ] **Step 3: Write `bindings/typescript/tsconfig.json`** (editor/typecheck support; `tsc` is
  best-effort, not the gate — the runtime tests under both runtimes are the gate):

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "allowImportingTsExtensions": true,
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true,
    "types": []
  },
  "include": ["src", "test", "total.ts"]
}
```

- [ ] **Step 4: Write `bindings/typescript/src/types.ts`** (the wire types, 1:1 snake_case):

```ts
export interface TokenBuckets {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
}
export interface ModelCost {
  model: string;
  provider: string;
  tokens: TokenBuckets;
  subtotal_usd: number;
}
export interface Approximation {
  kind: string;
  detail?: string;
}
export interface CostEstimate {
  total_usd: number;
  per_model: ModelCost[];
  tokens: TokenBuckets;
  unpriced_models: string[];
  approximations: Approximation[];
  pricing_as_of: string;
}
export interface RefreshReport {
  models: number;
  as_of: string;
  written_to: string;
}
export type Dialect = "claude" | "codex" | "pi";

export class ObolError extends Error {
  code: number;
  kind: string;
  constructor(code: number, kind: string, message: string) {
    super(`obol: ${kind} (code ${code}): ${message}`);
    this.name = "ObolError";
    this.code = code;
    this.kind = kind;
  }
}
```

- [ ] **Step 5: Write `bindings/typescript/src/lib-path.ts`** (dylib discovery, mirrors Python):

```ts
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

function libFilename(): string {
  switch (process.platform) {
    case "darwin": return "libobol_ffi.dylib";
    case "win32": return "obol_ffi.dll";
    default: return "libobol_ffi.so";
  }
}

export function resolveLibPath(): string {
  const tried: string[] = [];
  const env = process.env.OBOL_LIB;
  if (env) {
    tried.push(env);
    if (existsSync(env)) return env;
  }
  const name = libFilename();
  // this file: bindings/typescript/src/lib-path.ts — repo root is three up from src/
  const here = dirname(fileURLToPath(import.meta.url));
  const repo = join(here, "..", "..", ".."); // src -> typescript -> bindings -> repo
  for (const profile of ["release", "debug"]) {
    const p = join(repo, "target", profile, name);
    tried.push(p);
    if (existsSync(p)) return p;
  }
  throw new Error(
    "obol_ffi shared library not found. Set OBOL_LIB or run `cargo build -p obol-ffi`. Tried:\n  " +
      tried.join("\n  "),
  );
}
```

- [ ] **Step 6: Write `bindings/typescript/src/ffi.ts`** (the backend seam + memoized lazy load):

```ts
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
```

- [ ] **Step 7: Write `bindings/typescript/src/ffi-bun.ts`** (the `bun:ffi` adapter — verbatim
  from the proven probe):

```ts
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

  // Copy the obol-owned string out, then free it. Always frees when out[0] is non-NULL.
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
      const code = symbols.obol_estimate_bytes(ptr(data), BigInt(data.length), cstr(dialect), ptr(out));
      return drain(code, out);
    },
    refresh(asOf) {
      const out = new BigUint64Array(1);
      const code = symbols.obol_refresh_pricing(cstr(asOf), ptr(out));
      return drain(code, out);
    },
  };
}
```

- [ ] **Step 8: Write `bindings/typescript/src/index.ts`** (the public typed API):

```ts
import { backend, type RawResult } from "./ffi.ts";
import { ObolError } from "./types.ts";
import type { CostEstimate, RefreshReport, Dialect } from "./types.ts";

function unwrap<T>(r: RawResult): T {
  if (r.code !== 0) {
    let kind = "Unknown";
    let message = "no detail";
    let code = r.code;
    if (r.json) {
      try {
        const e = (JSON.parse(r.json) as { error?: { code?: number; kind?: string; message?: string } }).error;
        if (e) {
          kind = e.kind ?? kind;
          message = e.message ?? message;
          code = e.code ?? code;
        }
      } catch {
        /* keep defaults */
      }
    }
    throw new ObolError(code, kind, message);
  }
  return JSON.parse(r.json as string) as T;
}

export async function version(): Promise<string> {
  return (await backend()).version();
}
export async function estimatePath(path: string, dialect: Dialect | null = null): Promise<CostEstimate> {
  return unwrap<CostEstimate>((await backend()).estimatePath(path, dialect));
}
export async function estimateBytes(data: Uint8Array, dialect: Dialect | null = null): Promise<CostEstimate> {
  return unwrap<CostEstimate>((await backend()).estimateBytes(data, dialect));
}
export async function refresh(asOf: string): Promise<RefreshReport> {
  return unwrap<RefreshReport>((await backend()).refresh(asOf));
}

export { ObolError } from "./types.ts";
export type { CostEstimate, ModelCost, TokenBuckets, Approximation, RefreshReport, Dialect } from "./types.ts";
```

- [ ] **Step 9: Write `bindings/typescript/test/pricing-env.ts`** (cross-runtime pricing-dir
  control — the fix for the Bun runtime-env trap in "Critical facts"). Under Bun it also calls libc
  `setenv`/`unsetenv` so the change reaches Rust's `getenv`; under Node `process.env` alone
  suffices. The native `setenv` also overrides any stale global `~/.local/share/obol`, so tests are
  robust regardless of the host's global pricing dir:

```ts
// Cross-runtime control of OBOL_PRICING_DIR for tests.
// Bun does NOT sync runtime `process.env` writes to the native env that bun:ffi's C calls (and
// Rust's getenv) observe, so under Bun we must call libc setenv/unsetenv. Node propagates
// process.env to getenv natively, so there process.env alone is enough.
const KEY = "OBOL_PRICING_DIR";
const isBun = typeof (globalThis as { Bun?: unknown }).Bun !== "undefined";

interface LibcEnv {
  setenv(k: Uint8Array, v: Uint8Array, overwrite: number): number;
  unsetenv(k: Uint8Array): number;
}
let libc: LibcEnv | undefined;
async function nativeEnv(): Promise<LibcEnv> {
  if (libc) return libc;
  const { dlopen, FFIType } = await import("bun:ffi");
  const name = process.platform === "darwin" ? "libSystem.dylib" : "libc.so.6";
  const { symbols } = dlopen(name, {
    setenv: { args: [FFIType.cstring, FFIType.cstring, FFIType.i32], returns: FFIType.i32 },
    unsetenv: { args: [FFIType.cstring], returns: FFIType.i32 },
  });
  libc = symbols as unknown as LibcEnv;
  return libc;
}

export async function setPricingDir(dir: string): Promise<void> {
  process.env[KEY] = dir;
  if (isBun) (await nativeEnv()).setenv(Buffer.from(KEY + "\0"), Buffer.from(dir + "\0"), 1);
}

export async function clearPricingDir(): Promise<void> {
  delete process.env[KEY];
  if (isBun) (await nativeEnv()).unsetenv(Buffer.from(KEY + "\0"));
}
```

  (Node never calls `nativeEnv`, so it never imports `bun:ffi`.)

- [ ] **Step 10: Write `bindings/typescript/test/obol.test.ts`** (the matrix; `node:test`/
  `node:assert` so the one file runs under both runtimes; pricing-dir changes go through the
  helper):

```ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, copyFileSync, rmSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import * as obol from "../src/index.ts";
import { setPricingDir, clearPricingDir } from "./pricing-env.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const TESTDATA = join(HERE, "..", "..", "testdata"); // test -> typescript -> bindings, then /testdata
const TRANSCRIPT = join(TESTDATA, "claude-mini.jsonl");

async function seed(): Promise<string> {
  const dir = mkdtempSync(join(tmpdir(), "obol-ts-"));
  copyFileSync(join(TESTDATA, "prices.json"), join(dir, "current.json"));
  await setPricingDir(dir);
  return dir;
}

test("version", async () => {
  assert.equal(await obol.version(), "0.1.0");
});

test("estimatePath success", async () => {
  const dir = await seed();
  try {
    const est = await obol.estimatePath(TRANSCRIPT, "claude");
    assert.ok(est.total_usd > 0, `total_usd=${est.total_usd}`);
    assert.equal(est.pricing_as_of, "2026-06-05");
  } finally {
    rmSync(dir, { recursive: true, force: true });
    await clearPricingDir();
  }
});

test("estimateBytes autodetect", async () => {
  const dir = await seed();
  try {
    const data = readFileSync(TRANSCRIPT); // Buffer is a Uint8Array
    const est = await obol.estimateBytes(data);
    assert.ok(est.total_usd > 0);
  } finally {
    rmSync(dir, { recursive: true, force: true });
    await clearPricingDir();
  }
});

test("missing tables -> ObolError code 1", async () => {
  await setPricingDir("/nonexistent/obol-ts-xyz");
  try {
    const data = readFileSync(TRANSCRIPT);
    await assert.rejects(
      () => obol.estimateBytes(data, "claude"),
      (e: unknown) => e instanceof obol.ObolError && e.code === 1 && e.kind === "PricingTablesMissing",
    );
  } finally {
    await clearPricingDir();
  }
});

test("unknown dialect -> ObolError code 7", async () => {
  const dir = await seed();
  try {
    const data = readFileSync(TRANSCRIPT);
    await assert.rejects(
      () => obol.estimateBytes(data, "banana" as obol.Dialect),
      (e: unknown) => e instanceof obol.ObolError && e.code === 7,
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
    await clearPricingDir();
  }
});
```

  Note the `TESTDATA` path: from `bindings/typescript/test/`, two `..` reach `bindings/`, then
  `testdata` → `bindings/testdata`.

- [ ] **Step 11: Install deps and run the Bun tests.** From `bindings/typescript`:

```bash
cd bindings/typescript && bun install && bun test
```

Expected: **5 pass** (version, estimatePath success, estimateBytes autodetect, missing-tables →
code 1, unknown-dialect → code 7). The loader's `target/debug` fallback finds the dylib with no
env, and the helper's native `setenv` makes the seeded pricing dir reach Rust under Bun (and
overrides any stale global `~/.local/share/obol`). If `pricing_as_of` comes back as anything other
than `2026-06-05`, the helper isn't being used for a seed call.

- [ ] **Step 12: Commit.**

```bash
git add bindings/typescript
git commit -m "feat(bindings): TypeScript binding — bun:ffi adapter + typed API + tests (Bun)"
```

---

## Task 2: Node adapter (koffi) + tests under Node + README

**Files:**
- Create: `bindings/typescript/src/ffi-node.ts`
- Create: `bindings/typescript/README.md`

- [ ] **Step 1: Ensure koffi is installed for Node** (Task 1 added it to `package.json`):

```bash
cd bindings/typescript && npm install
```

(`npm install` resolves `koffi@2.16.2` with its prebuilt binary. `bun install` from Task 1 also
installed it, but Node resolves from `node_modules` the same way.)

- [ ] **Step 2: Run the existing tests under Node — expect FAIL** (ffi-node.ts missing):

```bash
cd bindings/typescript && node --test test/obol.test.ts
```

Expected: error resolving `./ffi-node.ts` (module not found) — Node takes the non-Bun branch.

- [ ] **Step 3: Write `bindings/typescript/src/ffi-node.ts`** (the `koffi` adapter — verbatim from
  the proven probe; note `void**` and `import koffi`):

```ts
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
      const code = obol_estimate_bytes(data, data.length, dialect, out) as number;
      return drain(code, out);
    },
    refresh(asOf) {
      const out: [unknown] = [null];
      const code = obol_refresh(asOf, out) as number;
      return drain(code, out);
    },
  };
}
```

- [ ] **Step 4: Run the tests under Node — expect PASS:**

```bash
cd bindings/typescript && node --test test/obol.test.ts
```

Expected: 5 pass (Node 26 strips TS types; no flag needed). If koffi can't pass the `Uint8Array`
for `data`, wrap as `Buffer.from(data)` in `estimateBytes` — but the probe passed a Buffer
directly and koffi accepts typed arrays for pointer args, so this should be unnecessary.

- [ ] **Step 5: Re-run under Bun to confirm no regression:**

```bash
cd bindings/typescript && bun test
```

Expected: 5 pass (still).

- [ ] **Step 6: Write `bindings/typescript/README.md`** — short: install (`bun install` or
  `npm install`), usage (`import { estimatePath } from "obol"`), that it works under both Bun
  (`bun:ffi`, zero runtime deps) and Node (`koffi`); the ownership note (the adapter copies the
  obol-owned string then frees it — callers never touch pointers); that the cdylib must be built
  (`cargo build -p obol-ffi`) or pointed at via `OBOL_LIB`; and that pricing tables must exist
  (`obol refresh` or a seeded `OBOL_PRICING_DIR`). **Include the Bun env caveat:** set
  `OBOL_PRICING_DIR`/`OBOL_LIB` *in the environment before launching* — under Bun, mutating
  `process.env` at runtime does not reach the native library (Rust reads the OS env via `getenv`),
  so a runtime `process.env.OBOL_PRICING_DIR = …` is silently ignored under Bun (it works under
  Node). One sentence is enough.

- [ ] **Step 7: Commit.**

```bash
git add bindings/typescript/src/ffi-node.ts bindings/typescript/README.md
git commit -m "feat(bindings): TypeScript Node adapter (koffi) + tests under Node + README"
```

---

## Task 3: `total.ts` + four-language equivalence gate + validation doc

**Files:**
- Create: `bindings/typescript/total.ts`
- Modify: `scripts/validate_bindings.sh`
- Modify: `docs/validation-ffi-2026-06-05.md`

- [ ] **Step 1: Write `bindings/typescript/total.ts`** (prints `total_usd`; the gate runs it under
  both runtimes):

```ts
import { estimatePath } from "./src/index.ts";

const path = process.argv[2];
const dialect = (process.argv[3] ?? null) as "claude" | "codex" | "pi" | null;
if (!path) {
  console.error("usage: total <transcript> [dialect]");
  process.exit(2);
}
const est = await estimatePath(path, dialect);
console.log(est.total_usd);
```

(Top-level await is supported in ESM under both Bun and Node 26. The gate normalizes the printed
number through Python `float()`, so plain `console.log(total_usd)` is sufficient.)

- [ ] **Step 2: Extend `scripts/validate_bindings.sh`** to add the two TS consumers and the
  five-way assertion. Find the block that computes `rust_total`/`py_total`/`go_total` and the
  final comparison; replace the comparison and add TS totals. The full updated tail of the script:

```bash
# (after rust_total, py_total, go_total are computed and `norm` is defined)

# Ensure TS deps (koffi) are present for the Node consumer.
( cd bindings/typescript && bun install >/dev/null 2>&1 || npm install >/dev/null 2>&1 )

ts_bun=$(  (cd bindings/typescript && bun  total.ts "$ROOT/$T" claude) | norm )
ts_node=$( (cd bindings/typescript && node total.ts "$ROOT/$T" claude) | norm )

echo "rust    : $rust_total"
echo "py      : $py_total"
echo "go      : $go_total"
echo "ts(bun) : $ts_bun"
echo "ts(node): $ts_node"

if [ "$rust_total" = "$py_total" ] && [ "$py_total" = "$go_total" ] \
   && [ "$go_total" = "$ts_bun" ] && [ "$ts_bun" = "$ts_node" ]; then
  echo "OK: rust == python == go == ts(bun) == ts(node) total_usd ($rust_total)"
else
  echo "MISMATCH: rust=$rust_total py=$py_total go=$go_total ts_bun=$ts_bun ts_node=$ts_node"; exit 1
fi
```

  Remove the old two-line `rust==python==go` comparison that this replaces. Keep everything above
  it (build, seed, `norm()`, the rust/py/go computations) intact. `OBOL_LIB` is already exported
  by the script, which the TS loader honors.

- [ ] **Step 3: Make sure it's executable and run it:**

```bash
chmod +x scripts/validate_bindings.sh && ./scripts/validate_bindings.sh
```

Expected: all five totals equal — `OK: rust == python == go == ts(bun) == ts(node) total_usd (0.000995)`.

- [ ] **Step 4: Update `docs/validation-ffi-2026-06-05.md`** — extend the "Method", "Results", and
  "Per-binding test suites" sections to include the TS consumer (run under both Bun and Node), and
  update the results table/block to the five-way agreement. Mirror the existing style; keep it
  honest (only what was shown).

- [ ] **Step 5: Commit.**

```bash
git add bindings/typescript/total.ts scripts/validate_bindings.sh docs/validation-ffi-2026-06-05.md
git commit -m "test: extend equivalence gate to TypeScript (Bun + Node) — four-language parity"
```

---

## Final verification (orchestrator, after the three tasks)

- [ ] `cd bindings/typescript && bun test` and `node --test test/obol.test.ts` — both green.
- [ ] `./scripts/validate_bindings.sh` — five-way totals agree.
- [ ] Dispatch a final reviewer over the whole branch (both adapters, the free-dance in each, the
  module-resolution choices, docs).
- [ ] **Linux container verification:** extend the `ubuntu:24.04` verification to install Bun + Node
  + `npm install` koffi, build `libobol_ffi.so`, run the TS tests under both runtimes, and run the
  five-language gate — confirming `bun:ffi`/`koffi` `dlopen` of a `.so` works on Linux.
- [ ] Merge to `main` (fast-forward), move PRI-2085 to In Review with a reflective comment.

## Self-review notes (plan author)

- **Spec coverage:** every spec section maps to a task — Bun adapter + types + lib-path + detection
  + public API (Task 1), koffi adapter + dual-runtime tests + README (Task 2), `total.ts` + gate +
  validation (Task 3), Linux verify (final). Both FFI dances are the spec's probed code, verbatim.
- **Type consistency:** `FfiBackend`/`RawResult` are defined once in `ffi.ts` and imported by both
  adapters and `index.ts`; `createBackend(libPath)` signature is identical in both adapters; the
  wire interfaces in `types.ts` match `model.rs` (snake_case) and are reused by `index.ts` and the
  tests. `ObolError` is defined in `types.ts`, re-exported from `index.ts`, asserted in tests.
- **Placeholder scan:** no TBD/"handle errors"/"similar to" — every step has complete code. The one
  inline-comment nit in the test (the `TESTDATA` path comment) is called out with its correction in
  Step 9.
- **Known soft spots flagged for the implementer:** koffi accepting a `Uint8Array` for `data`
  (fallback `Buffer.from`); `tsc` is best-effort not a gate (the dual-runtime tests are the gate);
  the validation-doc edit is descriptive (mirror existing style).
