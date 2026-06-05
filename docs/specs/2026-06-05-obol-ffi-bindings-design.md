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

### The JSON shape bindings re-type (so they mirror it exactly)

From `CostEstimate` (`model.rs`): `total_usd: f64`, `per_model: [{model: string, provider:
string, tokens: {input,output,cache_read,cache_write: u64}, subtotal_usd: f64}]`, `tokens:
{…same four…}`, `unpriced_models: [string]`, `approximations: [...]`, `pricing_as_of: string`.
Two shape subtleties the hand-typed structs must honor:

- **`provider` is a lowercase string**, not an object — `Provider` has a custom `Serialize`
  emitting its label (`"anthropic"`, `"openai"`, `"openrouter"`, or the raw `Other` string).
- **`approximations` is an internally-tagged union**: `#[serde(tag="kind", content="detail")]`
  → `{"kind":"UnpricedModel","detail":"…"}` or `{"kind":"AssumedStandardTier"}` (no detail).
  Bindings type this as a small tagged record, not a bare string.

`RefreshReport` JSON: `{models: u64, as_of: string, written_to: string}` — `written_to` is a
path-as-string.

## The C ABI surface

Five functions. Signatures below show the **C types cbindgen will actually emit** — note
`uintptr_t` for the Rust `usize` (cbindgen maps `usize → uintptr_t`, not `size_t`; ABI-
identical, and we avoid a `libc` dependency just to rename it). The committed header is
whatever cbindgen produces, and the drift test enforces exactly that, so this sample is kept
honest rather than idealized.

**ABI constraints (what makes cbindgen emit a correct header):** every exported function is
`#[no_mangle] pub extern "C"`; param types are `*const c_char` (`std::os::raw::c_char`),
`*const u8`, `usize`, the out-param `*mut *mut c_char`, and an `i32` return. No Rust structs
cross the boundary — only C strings and integers — so there are no `repr(C)` concerns.

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
int32_t obol_estimate_bytes(const uint8_t *data, uintptr_t len,
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

`obol_version` returns a true `'static` pointer — implemented as
`concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr()` (a `&'static CStr`), **not**
`CString::into_raw` (which would leak on every call and contradict "do NOT free").

`int32_t`/`uint8_t`/`uintptr_t` via `<stdint.h>` (cbindgen emits the includes).

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

So a binding can either switch on the int or parse the envelope — both agree.

**Mapping:** codes 1–6 are produced by matching the returned `ObolError` variant; the
envelope `message` is `err.to_string()` (thiserror's `Display`, e.g. "malformed transcript at
line 12: …"). Codes 7 (invalid argument) and 8 (panic) have **no** `ObolError` behind them —
their envelope is synthesized directly in the FFI.

**The NULL-init invariant (closes the panic double-free hazard):** the *very first* action of
every estimate/refresh function (when `out_json != NULL`) is `*out_json = ptr::null_mut()`.
Then the body runs inside `catch_unwind`. Consequences:

- Success → `*out_json` = the result JSON (obol-owned).
- Handled error (1–7) → `*out_json` = the error envelope.
- Panic (8) → the unwind is caught; the FFI writes a best-effort envelope, but even if *that*
  allocation fails, `*out_json` is still the clean NULL from the first action — never garbage.

So `out_json` is **always a well-defined pointer** (a valid string or NULL) on every
non-crashing return, and `obol_string_free(NULL)` is a documented no-op. The caller's
free-path is therefore uniform and safe: one `obol_string_free(*out_json)` regardless of
outcome, with no risk of freeing an uninitialized pointer.

## Ownership & safety contract (the load-bearing section)

This is where FFI bindings live or die. The rules, stated once, enforced everywhere:

1. **Inputs are borrowed.** `path`, `dialect`, `data`, `as_of` are read during the call
   only. obol copies whatever it needs before returning. The caller may free them
   immediately after the call returns. obol never retains a pointer to caller memory.

2. **Outputs are obol-owned.** Every `*out_json` is allocated by Rust via
   `CString::new(json)?.into_raw()`. The `new` step is handled as a `Result` (never
   unwrapped): serde_json output can't contain an interior NUL — it escapes NUL as
   ` ` — so this cannot fail in practice, but a failure maps to an error rather than a
   panic. The caller **must** return the pointer via `obol_string_free`
   (`CString::from_raw` + drop), which uses Rust's allocator. Freeing it with libc `free`
   or any other allocator is undefined behavior. `obol_string_free(NULL)` is a safe no-op.

3. **No unwinding across the boundary.** Every extern function body runs inside
   `std::panic::catch_unwind(AssertUnwindSafe(|| …))`. `AssertUnwindSafe` is required because
   the closure captures the raw out-pointer and is **sound here** for a concrete reason: the
   obol-core call path holds no `Mutex`/lock across the boundary and shares no mutable state,
   so there is no poison/broken-invariant concern — the only thing mutated on a panic is
   local. A caught panic becomes status 8; it never propagates into C (which would be UB).
   `catch_unwind` is the outermost layer of each function, *after* the NULL-init of `out_json`
   (see Status codes) so the panic path leaves a freeable NULL, not garbage.

4. **NULL handling.** NULL `out_json` → status 7, nothing written (no pointer to write to).
   NULL `path`/`data`/`as_of` where required → status 7 with an envelope written only if
   `out_json` is non-NULL. `data == NULL` → status 7 regardless of `len`; `len == 0` with a
   non-NULL `data` is *valid* and flows to a normal `UnknownDialect`/malformed error from
   core (not a crash). NULL `dialect` → auto-detect (the `Option<Dialect>::None` path).

7. **Dialect-string parsing is the FFI's own, and validating.** The FFI matches `dialect`
   exhaustively: `"claude"`/`"codex"`/`"pi"` → the variant, NULL → auto-detect, **anything
   else → status 7**. It does *not* reuse the CLI's `_ => Dialect::Pi` fallthrough
   (`obol-cli/src/main.rs`), which is only safe there because clap's `value_parser` rejects
   unknown strings first — there is no such gate in the FFI, so an unknown string must error,
   not silently become Pi. obol-core exposes no shared `Dialect` string parser, and we do not
   add one (it'd be an unused core API — YAGNI); the small match lives in the FFI.

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
- Drift guard: a test in obol-ffi (`header_matches_source`) regenerates the header via the
  `cbindgen` *library* (a dev-dependency) and asserts it byte-equals the committed
  `include/obol.h`. **The test loads the same `cbindgen.toml` the script uses**
  (`cbindgen::Config::from_file`), so it is a true mirror of `gen-header.sh`, not a
  reconstructed config that could drift from it. This fails if someone changes an extern
  signature without regenerating — cheap insurance, no build.rs writing into the source tree.
  (If the `cbindgen` dev-dep proves too heavy, the documented fallback is: commit the header,
  keep the script, drop the test. Decision: keep the test; it is the simple-but-quality choice
  for a correctness-critical ABI.)

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

Added to workspace `members`. **Artifact name is determined, not open:** Cargo derives the
lib name from the package name with hyphens→underscores, so package `obol-ffi` → lib
`obol_ffi` → `target/{debug,release}/libobol_ffi.{dylib,so,a}`. **No `[lib] name` override
needed.** cgo links `-lobol_ffi`; ctypes loads `libobol_ffi.<ext>`. Both bindings locate the
artifact by that fixed name. `staticlib` is kept (one word, real future value for cgo static
builds) but adds no test surface in this cut — the bindings use the cdylib.

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
  binding run over the *same* transcript with the *same* pricing snapshot must surface the
  *same* `total_usd`. Don't diff *formatted* floats (Python `repr` vs Go `%v` vs shell
  `printf` can format the same f64 differently and produce false failures). Instead the gate
  extracts the **raw `total_usd` JSON token** each language emits and asserts the three tokens
  are **byte-identical** — which they will be, because all three deserialize the *same* JSON
  string produced by the *same* serde_json f64 serializer (and an f64 round-trips exactly
  through Python `float` / Go `float64`). That byte-equality is the true proof the seam is
  faithful. One script, `scripts/validate_bindings.sh`, seeds a snapshot, runs all three, and
  compares the tokens. (Per-language unit tests separately assert `total_usd > 0` as the
  "the binding actually executed" signal.)

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

- Go in-tree dev linking ergonomics (LDFLAGS pointing at `target/debug`) — make the test
  hermetic via env (e.g. `CGO_LDFLAGS`/an env-driven build) so `go test` works from a fresh
  checkout after a `cargo build`, without hand-editing paths. This is the one genuinely open
  implementation detail; the plan should pick a concrete mechanism.

> Resolved during spec review (Bender@af68a79d): artifact name is `libobol_ffi` (no `[lib]`
> override); keep the cbindgen drift test (load the shared `cbindgen.toml`); `usize` emits as
> `uintptr_t`; equivalence gate compares raw JSON tokens, not formatted floats; FFI owns its
> own validating dialect match (no `_ => Pi` fallthrough); `out_json` is NULL-initialized
> first so the caught-panic path is free-safe; `catch_unwind` uses `AssertUnwindSafe` (sound:
> no locks held across the boundary).
