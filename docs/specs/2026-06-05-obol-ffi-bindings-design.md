# obol — C-ABI spine + language bindings (design spec)

> 2026-06-05 · Shevek@7998e83e · draft for Bob review · Linear PRI-2084
> Builds on v1 (`2026-06-04-obol-design.md`) and Pi (`2026-06-05-obol-pi-design.md`).
> Adds the multi-language face: one C ABI (the spine) plus two bindings (Python, Go)
> that exercise it. Same disciplined loop: spec → plan → TDD → validate.

## Goal

Let programs written in languages other than Rust get an obol cost estimate without
re-implementing obol. The deliverable is **one stable C ABI** over obol-core's existing
entry points, and **two thin bindings** (Python, Go) that prove the ABI works for both
styles of consumer — dynamic-load (Python/ctypes) and compile-link (Go/cgo).

Non-goal: re-typing obol's domain logic in each language. Bindings parse JSON and present
idiomatic types. The Rust core stays the **single source of truth** for all accounting.

## Why a C ABI (and not something else)

C is the universal FFI substrate. Go *requires* it (cgo speaks C). Python reaches it for
free (ctypes, no build step). Every other language we might add later (Ruby, Node via
`ffi-napi`, Zig, …) can stand on the same floor. A C ABI built once is reused N times; a
per-language native module (napi-rs, PyO3) would be built N times. We pick the spine.

**The seam carries JSON.** `CostEstimate` already derives `serde::Serialize` and the CLI
already emits it with `--json`. The FFI hands back that exact JSON string; each binding
deserializes into its own idiomatic structs. Rejected alternatives, with reasons:

- **Protobuf / FlatBuffers at the seam** — adds a schema compiler and a wire format to
  every binding's build for a payload that is small, read-once, and human-debuggable. The
  estimate is produced once per transcript and read once; there is no hot loop. YAGNI.
- **JSON-Schema → per-language codegen** — a real future option once there are 3+ bindings
  and the hand-written structs become a maintenance tax. Today, two small structs per
  language hand-written is less total machinery than a codegen pipeline. Defer; revisit
  when the third binding lands.

So: **JSON at the seam, hand-typed per language.** Simple, debuggable, proven by the CLI.

## Architecture

```
crates/obol-core   (unchanged logic; one tiny additive derive — see below)
crates/obol-cli    (unchanged)
crates/obol-ffi    (NEW)  cdylib + staticlib; thin C-ABI wrapper over obol-core
   └── include/obol.h     (NEW, committed) cbindgen-generated header
bindings/python/          (NEW) ctypes wrapper → dataclasses
bindings/go/              (NEW) cgo wrapper → structs
```

`obol-ffi` is the only new Rust. It does no accounting — it marshals C arguments into
`obol-core` calls and marshals results back out as JSON C-strings. If a feature isn't in
obol-core, it isn't in the FFI.

### One additive change to obol-core

`RefreshReport` is not currently `Serialize`. Add `#[derive(serde::Serialize)]` to it
(`PathBuf` serializes as a string; harmless, and lets the CLI gain `refresh --json` later).
That is the *only* change to obol-core. Everything else is new code in `obol-ffi`.

## The C ABI surface

Six functions. Exact signatures (as they will appear in `obol.h`):

```c
/* Estimate cost from a transcript file on disk.
 *   path     : NUL-terminated UTF-8 path. Must be non-NULL.
 *   dialect  : "claude" | "codex" | "pi", or NULL to auto-detect.
 *   out_json : receives a heap-allocated NUL-terminated UTF-8 JSON string,
 *              owned by obol. Free with obol_string_free. Always written
 *              (success → CostEstimate JSON; error → error-envelope JSON).
 * Returns 0 on success, a positive obol_status code on error. */
int32_t obol_estimate_path(const char *path, const char *dialect, char **out_json);

/* Estimate cost from transcript bytes already in memory.
 *   data : pointer to len bytes (borrowed; obol copies what it needs). Non-NULL.
 *   len  : length in bytes.
 *   dialect / out_json : as above. */
int32_t obol_estimate_bytes(const uint8_t *data, size_t len,
                            const char *dialect, char **out_json);

/* Refresh pricing tables (network: pulls LiteLLM + OpenRouter sheets).
 *   as_of    : NUL-terminated date string the caller supplies (obol has no clock).
 *   out_json : RefreshReport JSON on success, error-envelope on failure. */
int32_t obol_refresh_pricing(const char *as_of, char **out_json);

/* Free a string previously returned in an out_json out-parameter.
 * Passing NULL is a no-op. Never free obol strings any other way. */
void obol_string_free(char *s);

/* Library version, e.g. "0.1.0". Static storage — do NOT free. */
const char *obol_version(void);
```

`int32_t`/`size_t`/`uint8_t` via `<stdint.h>`/`<stddef.h>` (cbindgen emits the includes).

### Status codes

`obol_estimate_*` / `obol_refresh_pricing` return an `int32_t`:

| code | meaning | maps to |
|---|---|---|
| 0 | success | `Ok` |
| 1 | pricing tables missing | `ObolError::PricingTablesMissing` |
| 2 | unknown / undetectable dialect | `ObolError::UnknownDialect` |
| 3 | malformed transcript | `ObolError::MalformedTranscript` |
| 4 | network error during refresh | `ObolError::Network` |
| 5 | io error | `ObolError::Io` |
| 6 | json error | `ObolError::Json` |
| 7 | invalid argument (FFI-level: NULL where required, bad UTF-8, unknown dialect string) | — |
| 8 | internal panic (caught at the boundary) | — |

The integer is the fast path. **Detail always travels in `out_json`** as an error envelope:

```json
{ "error": { "code": 3, "kind": "MalformedTranscript", "message": "malformed transcript at line 12: ..." } }
```

So a binding can either switch on the int or parse the envelope — both agree. `out_json`
is *always* written on every non-crashing return, success or error, so the caller's
free-path is uniform (one `obol_string_free` regardless of outcome).

## Ownership & safety contract (the load-bearing section)

This is where FFI bindings live or die. The rules, stated once, enforced everywhere:

1. **Inputs are borrowed.** `path`, `dialect`, `data`, `as_of` are read during the call
   only. obol copies whatever it needs before returning. The caller may free them
   immediately after the call returns. obol never retains a pointer to caller memory.

2. **Outputs are obol-owned.** Every `*out_json` is allocated by Rust
   (`CString::into_raw`). The caller **must** return it via `obol_string_free`
   (`CString::from_raw` + drop), which uses Rust's allocator. Freeing it with libc `free`
   or any other allocator is undefined behavior. `obol_string_free(NULL)` is a safe no-op.

3. **No unwinding across the boundary.** Every extern function body is wrapped in
   `std::panic::catch_unwind`. A panic is converted to status 8 with a generic envelope —
   it never propagates into C (which would be UB). The catch is the outermost layer of
   each function.

4. **NULL handling.** NULL `out_json` → status 7, nothing written (no pointer to write to).
   NULL `path`/`data`/`as_of` where required → status 7 with an envelope written only if
   `out_json` is non-NULL. NULL `dialect` → auto-detect (the `Option<Dialect>::None` path).

5. **Thread-safety.** `obol_estimate_*` is reentrant and `Send`-safe: it holds no shared
   mutable state, loads the price snapshot fresh, and touches only borrowed/owned memory.
   Concurrent estimates are fine. `obol_refresh_pricing` writes the on-disk snapshot;
   concurrent refresh-vs-refresh or refresh-vs-estimate is the caller's concern, exactly as
   it already is for the Rust library (no new contract).

6. **UTF-8.** Input strings must be valid UTF-8 (they come from `CStr` → `str`); invalid
   UTF-8 → status 7. Output JSON is always valid UTF-8.

These six rules go verbatim into a doc-comment block at the top of the FFI crate AND into
each binding's README, because the binding author is the one who has to honor them.

## Header generation

`obol.h` is generated by **cbindgen** and **committed** to `crates/obol-ffi/include/`.

- Config: `crates/obol-ffi/cbindgen.toml` (C output, `obol_`-prefixed, include guard,
  `#include <stdint.h>`/`<stddef.h>`, the ownership contract as a file header comment).
- Regeneration: `scripts/gen-header.sh` runs `cbindgen --config … --output include/obol.h`.
- Drift guard: a test in obol-ffi (`header_matches_source`) regenerates the header to a
  temp file via the `cbindgen` *library* (a build/dev-dependency) and asserts it byte-equals
  the committed `include/obol.h`. This fails CI-or-local if someone changes an extern
  signature without regenerating — cheap insurance, no build.rs writing into the source tree.
  (If `cbindgen` as a dependency proves heavy, the fallback is: commit the header, document
  the script, drop the test. Decision: keep the test; it is the simple-but-quality choice.)

Committing the header means binding builds (Go especially) never need cbindgen installed.

## Crate setup (`obol-ffi`)

```toml
[package]
name = "obol-ffi"
# version/edition/license from workspace

[lib]
crate-type = ["cdylib", "staticlib"]   # cdylib for ctypes/cgo dynamic; staticlib for static link

[dependencies]
obol-core = { path = "../obol-core" }
serde_json = { workspace = true }

[dev-dependencies]
cbindgen = "0.27"   # for the header-drift test only
```

Added to workspace `members`. Builds to `target/{debug,release}/libobol_ffi.{dylib,so,a}`
(note: cdylib name is `libobol_ffi` from crate `obol-ffi`; bindings locate it by that name,
or we set a `[lib] name = "obol_ffi"` explicitly and document the artifact path).

## Binding: Python (ctypes)

`bindings/python/` — pure Python, no build step, stdlib only.

```
bindings/python/
  obol/
    __init__.py      # public API: estimate_path, estimate_bytes, refresh; dataclasses; ObolError
    _lib.py          # ctypes CDLL load + function prototypes + obol_string_free wrapper
  README.md          # ownership contract note + how to point at the built dylib
  tests/test_obol.py # exercises estimate over a fixture; asserts total_usd, error path
```

- **Loading the dylib:** check `$OBOL_LIB` (explicit path) first; else look beside the
  package; else fall back to `target/{release,debug}/libobol_ffi.<ext>` relative to the repo
  (for in-tree dev). Raise a clear `ObolError` if not found.
- **Prototypes:** declare `argtypes`/`restype` for all six functions. `out_json` is a
  `ctypes.c_char_p` by reference (`POINTER(c_char_p)`).
- **The free dance:** after a call, copy the C string into a Python `bytes`/`str`
  *immediately*, then `obol_string_free` the original, then `json.loads` the copy. Never let
  the Rust-owned pointer outlive the copy. This is wrapped in a single helper so call sites
  can't get it wrong.
- **Types:** `@dataclass` `CostEstimate`, `ModelCost`, `TokenBuckets`, `Approximation`,
  built from the parsed JSON (`from_json` classmethods). `ObolError(code:int, kind:str,
  message:str)` raised on nonzero status.
- **API:** `obol.estimate_path(path, dialect=None) -> CostEstimate`,
  `obol.estimate_bytes(data: bytes, dialect=None) -> CostEstimate`,
  `obol.refresh(as_of: str) -> RefreshReport`.

## Binding: Go (cgo)

`bindings/go/` — the consumer that *requires* the C ABI; proves compile-link works.

```
bindings/go/
  obol/
    obol.go          # cgo: #include "obol.h"; wrappers; JSON unmarshal; error type
    obol_test.go     # estimate over a fixture; assert TotalUSD>0; error path
  README.md          # cgo CFLAGS/LDFLAGS, ownership contract, how to point at headers+lib
  go.mod
```

- **cgo preamble:** `// #cgo CFLAGS: -I${SRCDIR}/../../crates/obol-ffi/include` and
  `// #cgo LDFLAGS: -L<target dir> -lobol_ffi` (documented; the test sets them via env or a
  build tag for the in-tree dev path). `#include "obol.h"`.
- **The free dance:** call the C function, `C.GoString` the result into a Go string, then
  `C.obol_string_free` the C pointer, then `json.Unmarshal`. Same discipline as Python.
- **Types:** Go structs with `json:"..."` tags mirroring `CostEstimate` et al. `ObolError`
  implements `error`, carries `Code`, `Kind`, `Message`.
- **API:** `obol.EstimatePath(path string, dialect string) (*CostEstimate, error)`
  (empty `dialect` = auto), `obol.EstimateBytes([]byte, dialect string)`,
  `obol.Refresh(asOf string) (*RefreshReport, error)`.

## Testing & validation

- **Rust (obol-ffi) unit tests:** call each extern fn directly from a `#[test]`:
  success path (seed a temp pricing dir like the existing api_tests, estimate a fixture,
  assert the returned JSON parses and `total_usd > 0`), error paths (missing tables → 1,
  bad dialect string → 7, NULL out → 7, malformed bytes → 3), and the `obol_string_free`
  round-trip (no leak/double-free under a simple loop). Plus the `header_matches_source`
  drift test.
- **Python tests:** `pytest` (or stdlib `unittest`) against the built dylib over the
  existing `claude-mini.jsonl` fixture with a seeded pricing dir; assert the dataclass
  fields; assert `ObolError` raised on a missing-tables run.
- **Go tests:** `go test` over the same fixture; assert `TotalUSD > 0`; assert error type.
- **Cross-language equivalence (acceptance):** the Rust CLI, the Python binding, and the Go
  binding run over the *same* transcript with the *same* pricing snapshot must produce the
  *same* `total_usd` (to JSON float equality). One script, `scripts/validate_bindings.sh`,
  seeds a snapshot, runs all three, and diffs the totals. This is the proof the seam is
  faithful — the same number out of three languages.

## Repo topology

Monorepo, `bindings/<lang>/` alongside `crates/`. The user floated Go-in-its-own-repo for
eventual publishing; that is a packaging decision deferred until we actually publish. For
now everything lives together so the equivalence test can run all three from one checkout.

## Out of scope (this cut)

- **TypeScript** — spawn-CLI vs napi-rs is a real fork, and napi-rs brings the Node-vs-Bun
  N-API question (the user uses a lot of Bun). Its own ticket once these two land.
- **Protobuf / schema-codegen** at the seam (revisit at binding #3).
- **Publishing / packaging** (PyPI wheel, Go module path, prebuilt dylibs per platform) —
  these bindings are in-tree dev artifacts for now; packaging is a later milestone.
- **Windows** — contract is portable, but we validate on macОS/Linux (`.dylib`/`.so`) only.

## Open threads (small)

- Exact cdylib artifact name (`libobol_ffi` vs forcing `[lib] name`): pick whichever yields
  the cleanest, most predictable path for both bindings to locate; document it once.
- Whether the `cbindgen` dev-dependency is worth the build weight for the drift test, or
  whether the lighter "script + documented regen" path is better. Lean: keep the test.
- Go in-tree dev linking ergonomics (LDFLAGS pointing at `target/debug`) — make the test
  hermetic via env so `go test` works from a fresh checkout after a `cargo build`.
