# obol — TypeScript binding (Bun + Node)

A thin TypeScript binding over obol's C ABI (`obol-ffi`). It runs under **both Bun and Node**:
Bun uses the built-in `bun:ffi` (zero runtime deps), Node uses [`koffi`](https://koffi.dev). The
Rust core does all the accounting; this binding only `dlopen`s the cdylib and re-types the JSON.

## Install

```sh
bun install     # or: npm install
```

Build the native library it loads (once, from the repo root):

```sh
cargo build -p obol-ffi    # produces target/{debug,release}/libobol_ffi.{dylib,so}
```

The binding finds the library via `$OBOL_LIB` (an explicit path) or, failing that, by looking in
`target/release` then `target/debug` relative to the repo. Set `OBOL_LIB` if the library lives
elsewhere.

## Usage

```ts
import { estimatePath, estimateBytes, refresh, version, ObolError } from "obol";

const est = await estimatePath("session.jsonl", "claude"); // dialect optional → auto-detect
console.log(est.total_usd, est.pricing_as_of);

try {
  await estimateBytes(new Uint8Array(/* … */));
} catch (e) {
  if (e instanceof ObolError) console.error(e.code, e.kind, e.message);
}
```

The API is async because the FFI backend is loaded lazily (and cached) on first use.

Pricing tables must exist before estimating — run `obol refresh` (the CLI) or point
`OBOL_PRICING_DIR` at a directory containing a `current.json` snapshot.

## Ownership

You never touch raw pointers. Each call copies obol's returned string into a JS string and then
frees the obol-owned pointer (via `obol_string_free`) inside the adapter — the single place that
can get it right.

## Bun environment caveat

obol's Rust core reads `OBOL_PRICING_DIR` / `OBOL_LIB` from the OS environment via `getenv`, which
is resolved per call. **Under Bun, mutating `process.env` at runtime does not reach the native
library** — set these variables in the environment *before launching* the process (the normal
way). Node propagates `process.env` to `getenv`, so runtime mutation works there; Bun does not.
(The test suite works around this by calling libc `setenv` directly under Bun — see
`test/pricing-env.ts`.)
