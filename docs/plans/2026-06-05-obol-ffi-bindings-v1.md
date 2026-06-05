# obol C-ABI spine + Python/Go bindings — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to
> implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose obol-core through a stable C ABI (`obol-ffi` cdylib) and exercise it with two
bindings — Python (ctypes) and Go (cgo) — that produce the same cost estimate as the Rust CLI.

**Architecture:** JSON at the FFI seam. `obol-ffi` marshals C args into `obol-core`'s
`estimate_cost`/`refresh_pricing_tables` and returns the serde_json of `CostEstimate`/
`RefreshReport`. Each binding deserializes that JSON into idiomatic types. The Rust core stays
the single source of truth; bindings re-type, never re-implement. Spec:
`docs/specs/2026-06-05-obol-ffi-bindings-design.md` (read it — the ownership/safety contract is
load-bearing).

**Tech stack:** Rust 1.96 (`mise exec rust@1.96.0 -- cargo …`); cbindgen 0.27; Python 3
(stdlib ctypes); Go 1.21+ (cgo). Tests that mutate `OBOL_PRICING_DIR` must run with
`-- --test-threads=1`.

**Three slices, eight tasks:**
- Slice 1 (Tasks 1–5): the `obol-ffi` crate + header. The correctness-critical core.
- Slice 2 (Task 6): Python binding + tests.
- Slice 3 (Tasks 7–8): Go binding + tests + cross-language equivalence gate.

**House values:** simple but high-quality; no over-engineering, no crufty shortcuts; zero users
(pre-1.0, no back-compat shims). Frequent commits — one per task.

---

## Shared test fixtures (created in Task 1, used throughout)

Two committed fixtures that every layer's tests reuse:

- `crates/obol-ffi/tests/fixtures/claude-mini.jsonl` — a copy of
  `crates/obol-core/tests/fixtures/claude-mini.jsonl` (model `claude-opus-4-8`).
- `bindings/testdata/prices.json` — a minimal real `PriceStore` snapshot pricing that model.
  `ModelPrice` uses `#[serde(default)]` on every field except `input`/`output`, so the minimal
  form is valid:

```json
{"as_of":"2026-06-05","namespaces":{"litellm":{"claude-opus-4-8":{"input":5.0,"output":25.0,"cache_read":0.5,"cache_write":6.25}}}}
```

- `bindings/testdata/claude-mini.jsonl` — another copy of the same transcript (so the bindings
  don't reach into `crates/`).

The binding tests + the equivalence script set `OBOL_PRICING_DIR` to a temp dir and copy
`prices.json` to `$OBOL_PRICING_DIR/current.json` before estimating.

---

## Task 1: Core change + `obol-ffi` scaffold (version, string_free, fixtures)

**Files:**
- Modify: `crates/obol-core/src/lib.rs` (add `Serialize` to `RefreshReport`)
- Modify: `Cargo.toml` (workspace members — add `crates/obol-ffi`)
- Create: `crates/obol-ffi/Cargo.toml`
- Create: `crates/obol-ffi/src/lib.rs`
- Create: `crates/obol-ffi/tests/fixtures/claude-mini.jsonl` (copy)
- Create: `bindings/testdata/prices.json`, `bindings/testdata/claude-mini.jsonl`

- [ ] **Step 1: Add `Serialize` to `RefreshReport`.** In `crates/obol-core/src/lib.rs`, change
  the derive on `RefreshReport`:

```rust
/// Report from a pricing refresh.
#[derive(Debug, serde::Serialize)]
pub struct RefreshReport {
    pub models: usize,
    pub as_of: String,
    pub written_to: PathBuf,
}
```

- [ ] **Step 2: Add a test that `RefreshReport` serializes to the expected shape.** Append to
  the `api_tests` module in `crates/obol-core/src/lib.rs`:

```rust
#[test]
fn refresh_report_serializes() {
    let r = RefreshReport { models: 7, as_of: "2026-06-05".into(), written_to: "/x/current.json".into() };
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["models"], 7);
    assert_eq!(v["as_of"], "2026-06-05");
    assert_eq!(v["written_to"], "/x/current.json");
}
```

- [ ] **Step 3: Run it, expect PASS.** `mise exec rust@1.96.0 -- cargo test -p obol-core refresh_report_serializes -- --test-threads=1`. Expected: 1 passed.

- [ ] **Step 4: Add the crate to the workspace.** In the root `Cargo.toml`, extend members:

```toml
members = ["crates/obol-core", "crates/obol-cli", "crates/obol-ffi"]
```

- [ ] **Step 5: Create `crates/obol-ffi/Cargo.toml`:**

```toml
[package]
name = "obol-ffi"
edition.workspace = true
version.workspace = true
license.workspace = true

[lib]
crate-type = ["cdylib", "staticlib", "rlib"]

[dependencies]
obol-core = { path = "../obol-core" }
serde_json = { workspace = true }

[dev-dependencies]
cbindgen = "0.27"
```

(`rlib` is added so the crate's own `#[test]`s and the header drift test can link it as a
normal Rust lib alongside the C artifacts.)

- [ ] **Step 6: Write the failing test for `obol_version` and `obol_string_free`.** Create
  `crates/obol-ffi/src/lib.rs` with just this test module at first (no impl yet):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn version_is_static_and_correct() {
        let p = obol_version();
        assert!(!p.is_null());
        let s = unsafe { CStr::from_ptr(p) }.to_str().unwrap();
        assert_eq!(s, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn string_free_null_is_noop() {
        obol_string_free(std::ptr::null_mut()); // must not crash
    }
}
```

- [ ] **Step 7: Run it, expect FAIL** (functions not defined):
  `mise exec rust@1.96.0 -- cargo test -p obol-ffi -- --test-threads=1`.

- [ ] **Step 8: Implement `obol_version` + `obol_string_free`** at the top of
  `crates/obol-ffi/src/lib.rs` (above the test module):

```rust
//! obol-ffi: a C ABI over obol-core. JSON at the seam; the Rust core owns all accounting.
//!
//! OWNERSHIP & SAFETY CONTRACT (honor in every binding):
//!  1. Inputs (path/dialect/data/as_of) are borrowed; obol copies what it needs before
//!     returning. Caller may free them immediately after the call.
//!  2. Every `*out_json` is obol-owned (Rust allocator). Free ONLY via `obol_string_free`.
//!     Freeing any other way is undefined behavior. `obol_string_free(NULL)` is a no-op.
//!  3. Each function NULL-inits `*out_json` first, then runs inside catch_unwind: a caught
//!     panic yields status 8 and leaves a freeable string-or-NULL, never garbage.
//!  4. NULL required pointer -> status 7. NULL `dialect` -> auto-detect.
//!  5. `obol_estimate_*` is reentrant/stateless. `obol_refresh_pricing` writes the on-disk
//!     snapshot; concurrent refresh is the caller's concern (same as the Rust lib).
//!  6. Input strings must be valid UTF-8; output JSON always is.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::ptr;

use obol_core::{estimate_cost, refresh_pricing_tables, Dialect, ObolError, Source};

const OK: i32 = 0;
const ERR_PRICING_MISSING: i32 = 1;
const ERR_UNKNOWN_DIALECT: i32 = 2;
const ERR_MALFORMED: i32 = 3;
const ERR_NETWORK: i32 = 4;
const ERR_IO: i32 = 5;
const ERR_JSON: i32 = 6;
const ERR_INVALID_ARG: i32 = 7;
const ERR_PANIC: i32 = 8;

/// Library version as a `'static` NUL-terminated string. Do NOT free.
#[no_mangle]
pub extern "C" fn obol_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

/// Free a string previously returned in an `out_json` out-parameter. NULL is a no-op.
#[no_mangle]
pub extern "C" fn obol_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: `s` was produced by CString::into_raw in this library, or is NULL (handled).
    unsafe { drop(CString::from_raw(s)) };
}
```

- [ ] **Step 9: Run tests, expect PASS.** `mise exec rust@1.96.0 -- cargo test -p obol-ffi -- --test-threads=1`. Expected: 2 passed.

- [ ] **Step 10: Create the fixtures.**

```bash
mkdir -p crates/obol-ffi/tests/fixtures bindings/testdata
cp crates/obol-core/tests/fixtures/claude-mini.jsonl crates/obol-ffi/tests/fixtures/claude-mini.jsonl
cp crates/obol-core/tests/fixtures/claude-mini.jsonl bindings/testdata/claude-mini.jsonl
printf '%s\n' '{"as_of":"2026-06-05","namespaces":{"litellm":{"claude-opus-4-8":{"input":5.0,"output":25.0,"cache_read":0.5,"cache_write":6.25}}}}' > bindings/testdata/prices.json
```

- [ ] **Step 11: Commit.**

```bash
git add crates/obol-core/src/lib.rs Cargo.toml crates/obol-ffi bindings/testdata
git commit -m "feat(ffi): scaffold obol-ffi crate; obol_version + obol_string_free; RefreshReport Serialize"
```

---

## Task 2: Status-code/envelope internals

Internal helpers shared by every estimate/refresh function. Pure functions — unit-test them.

**Files:**
- Modify: `crates/obol-ffi/src/lib.rs`

- [ ] **Step 1: Write failing tests** for the error mapping and envelope. Add to the `tests`
  module:

```rust
#[test]
fn maps_obol_errors_to_codes() {
    use obol_core::ObolError;
    assert_eq!(code_and_kind(&ObolError::UnknownDialect), (ERR_UNKNOWN_DIALECT, "UnknownDialect"));
    assert_eq!(
        code_and_kind(&ObolError::MalformedTranscript { line: 1, msg: "x".into() }).0,
        ERR_MALFORMED
    );
    assert_eq!(code_and_kind(&ObolError::Network("x".into())), (ERR_NETWORK, "Network"));
}

#[test]
fn envelope_is_valid_json_with_fields() {
    let s = envelope(ERR_MALFORMED, "MalformedTranscript", "bad: \"quote\"");
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["error"]["code"], ERR_MALFORMED);
    assert_eq!(v["error"]["kind"], "MalformedTranscript");
    assert_eq!(v["error"]["message"], "bad: \"quote\"");
}
```

- [ ] **Step 2: Run, expect FAIL** (undefined). `mise exec rust@1.96.0 -- cargo test -p obol-ffi -- --test-threads=1`.

- [ ] **Step 3: Implement the helpers** (above the test module):

```rust
fn code_and_kind(e: &ObolError) -> (i32, &'static str) {
    match e {
        ObolError::PricingTablesMissing(_) => (ERR_PRICING_MISSING, "PricingTablesMissing"),
        ObolError::UnknownDialect => (ERR_UNKNOWN_DIALECT, "UnknownDialect"),
        ObolError::MalformedTranscript { .. } => (ERR_MALFORMED, "MalformedTranscript"),
        ObolError::Network(_) => (ERR_NETWORK, "Network"),
        ObolError::Io(_) => (ERR_IO, "Io"),
        ObolError::Json(_) => (ERR_JSON, "Json"),
    }
}

fn envelope(code: i32, kind: &str, message: &str) -> String {
    serde_json::json!({ "error": { "code": code, "kind": kind, "message": message } }).to_string()
}

/// Write `s` into `*out` as an obol-owned C string. Assumes `out` is non-NULL.
/// Returns true on success; false only if `s` contains an interior NUL (impossible for
/// serde_json output, which escapes NUL) — in which case `*out` is left NULL.
unsafe fn write_out(out: *mut *mut c_char, s: String) -> bool {
    match CString::new(s) {
        Ok(c) => {
            *out = c.into_raw();
            true
        }
        Err(_) => {
            *out = ptr::null_mut();
            false
        }
    }
}

/// Write an error envelope and return its code. Assumes `out` is non-NULL.
unsafe fn fail(out: *mut *mut c_char, code: i32, kind: &str, msg: &str) -> i32 {
    write_out(out, envelope(code, kind, msg));
    code
}

/// Turn a core result into (envelope-or-result written to `out`, status code).
/// Assumes `out` is non-NULL.
unsafe fn finish<T: serde::Serialize>(out: *mut *mut c_char, r: Result<T, ObolError>) -> i32 {
    match r {
        Ok(value) => match serde_json::to_string(&value) {
            Ok(json) => {
                if write_out(out, json) {
                    OK
                } else {
                    fail(out, ERR_JSON, "Json", "result contained an interior NUL")
                }
            }
            Err(e) => fail(out, ERR_JSON, "Json", &e.to_string()),
        },
        Err(e) => {
            let (code, kind) = code_and_kind(&e);
            fail(out, code, kind, &e.to_string())
        }
    }
}
```

- [ ] **Step 4: Run, expect PASS.** Then check clippy is clean:
  `mise exec rust@1.96.0 -- cargo clippy -p obol-ffi -- -D warnings`.

- [ ] **Step 5: Commit.**

```bash
git add crates/obol-ffi/src/lib.rs
git commit -m "feat(ffi): status codes, error envelope, and result-writing helpers"
```

---

## Task 3: `obol_estimate_bytes` + `obol_estimate_path`

The heart of the ABI. NULL-init, catch_unwind, validating dialect parse, core call, JSON out.

**Files:**
- Modify: `crates/obol-ffi/src/lib.rs`

- [ ] **Step 1: Write the failing test matrix.** Add to the `tests` module. The success tests
  seed a temp `OBOL_PRICING_DIR` from `bindings/testdata/prices.json` (relative include) and
  estimate the bundled fixture:

```rust
use std::ffi::CString;
use std::path::PathBuf;

// Seed a temp pricing dir from the shared prices fixture; returns the dir.
fn seed_pricing() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("obol-ffi-{}-{:?}", std::process::id(), std::thread::current().id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("OBOL_PRICING_DIR", &dir);
    let prices = include_bytes!("../../../bindings/testdata/prices.json");
    std::fs::write(dir.join("current.json"), prices).unwrap();
    dir
}

fn out_ptr() -> *mut c_char { std::ptr::null_mut() }

#[test]
fn estimate_bytes_success_with_seeded_store() {
    let dir = seed_pricing();
    let data = include_bytes!("../tests/fixtures/claude-mini.jsonl");
    let mut out = out_ptr();
    let code = obol_estimate_bytes(data.as_ptr(), data.len(), std::ptr::null(), &mut out);
    assert_eq!(code, OK, "code={code}");
    assert!(!out.is_null());
    let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
    obol_string_free(out);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(v["total_usd"].as_f64().unwrap() > 0.0, "{json}");
    std::fs::remove_dir_all(&dir).ok();
    std::env::remove_var("OBOL_PRICING_DIR");
}

#[test]
fn estimate_bytes_missing_tables_is_code_1() {
    std::env::set_var("OBOL_PRICING_DIR", "/nonexistent/obol-ffi-xyz");
    let data = include_bytes!("../tests/fixtures/claude-mini.jsonl");
    let mut out = out_ptr();
    let code = obol_estimate_bytes(data.as_ptr(), data.len(), std::ptr::null(), &mut out);
    assert_eq!(code, ERR_PRICING_MISSING);
    let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
    obol_string_free(out);
    assert!(json.contains("PricingTablesMissing"));
    std::env::remove_var("OBOL_PRICING_DIR");
}

#[test]
fn estimate_bytes_unknown_dialect_string_is_code_7() {
    let data = b"{}";
    let bad = CString::new("banana").unwrap();
    let mut out = out_ptr();
    let code = obol_estimate_bytes(data.as_ptr(), data.len(), bad.as_ptr(), &mut out);
    assert_eq!(code, ERR_INVALID_ARG);
    obol_string_free(out);
}

#[test]
fn estimate_bytes_null_out_is_code_7() {
    let data = b"{}";
    let code = obol_estimate_bytes(data.as_ptr(), data.len(), std::ptr::null(), std::ptr::null_mut());
    assert_eq!(code, ERR_INVALID_ARG);
}

#[test]
fn estimate_bytes_null_data_is_code_7() {
    let mut out = out_ptr();
    let code = obol_estimate_bytes(std::ptr::null(), 0, std::ptr::null(), &mut out);
    assert_eq!(code, ERR_INVALID_ARG);
    obol_string_free(out);
}

#[test]
fn estimate_path_bad_path_is_io_error() {
    let dir = seed_pricing();
    let p = CString::new("/nonexistent/obol/transcript.jsonl").unwrap();
    let mut out = out_ptr();
    let code = obol_estimate_path(p.as_ptr(), std::ptr::null(), &mut out);
    assert_eq!(code, ERR_IO, "code={code}");
    obol_string_free(out);
    std::fs::remove_dir_all(&dir).ok();
    std::env::remove_var("OBOL_PRICING_DIR");
}
```

Note: these tests set `OBOL_PRICING_DIR`, so the suite must run single-threaded (already
required). `include_bytes!` resolves **relative to the source file** (`crates/obol-ffi/src/lib.rs`),
so `../../../bindings/testdata/prices.json` → repo-root `bindings/testdata/prices.json`, and
`../tests/fixtures/claude-mini.jsonl` → `crates/obol-ffi/tests/fixtures/claude-mini.jsonl`
(the `../` climbs out of `src/`).

- [ ] **Step 2: Run, expect FAIL** (functions undefined).

- [ ] **Step 3: Implement a shared dialect parser + both functions.** Add above the tests:

```rust
/// NULL -> auto-detect (Ok(None)); known string -> Ok(Some). Unknown/invalid UTF-8 -> Err(()).
fn parse_dialect(dialect: *const c_char) -> Result<Option<Dialect>, ()> {
    if dialect.is_null() {
        return Ok(None);
    }
    let s = unsafe { CStr::from_ptr(dialect) }.to_str().map_err(|_| ())?;
    match s {
        "claude" => Ok(Some(Dialect::Claude)),
        "codex" => Ok(Some(Dialect::Codex)),
        "pi" => Ok(Some(Dialect::Pi)),
        _ => Err(()),
    }
}

/// Estimate cost from transcript bytes (borrowed). See the ownership contract.
#[no_mangle]
pub extern "C" fn obol_estimate_bytes(
    data: *const u8,
    len: usize,
    dialect: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    if out_json.is_null() {
        return ERR_INVALID_ARG;
    }
    unsafe { *out_json = ptr::null_mut() };
    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        if data.is_null() {
            return fail(out_json, ERR_INVALID_ARG, "InvalidArgument", "data pointer is NULL");
        }
        let dialect = match parse_dialect(dialect) {
            Ok(d) => d,
            Err(()) => {
                return fail(out_json, ERR_INVALID_ARG, "InvalidArgument", "unknown or invalid dialect string");
            }
        };
        let bytes = std::slice::from_raw_parts(data, len);
        finish(out_json, estimate_cost(Source::Bytes(bytes), dialect))
    }));
    match result {
        Ok(code) => code,
        Err(_) => unsafe { fail(out_json, ERR_PANIC, "Panic", "internal panic caught at FFI boundary") },
    }
}

/// Estimate cost from a transcript file path (borrowed). See the ownership contract.
#[no_mangle]
pub extern "C" fn obol_estimate_path(
    path: *const c_char,
    dialect: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    if out_json.is_null() {
        return ERR_INVALID_ARG;
    }
    unsafe { *out_json = ptr::null_mut() };
    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        if path.is_null() {
            return fail(out_json, ERR_INVALID_ARG, "InvalidArgument", "path pointer is NULL");
        }
        let path = match CStr::from_ptr(path).to_str() {
            Ok(s) => s,
            Err(_) => return fail(out_json, ERR_INVALID_ARG, "InvalidArgument", "path is not valid UTF-8"),
        };
        let dialect = match parse_dialect(dialect) {
            Ok(d) => d,
            Err(()) => {
                return fail(out_json, ERR_INVALID_ARG, "InvalidArgument", "unknown or invalid dialect string");
            }
        };
        finish(out_json, estimate_cost(Source::Path(Path::new(path)), dialect))
    }));
    match result {
        Ok(code) => code,
        Err(_) => unsafe { fail(out_json, ERR_PANIC, "Panic", "internal panic caught at FFI boundary") },
    }
}
```

- [ ] **Step 4: Run, expect PASS** (all estimate tests + the Task 1/2 tests):
  `mise exec rust@1.96.0 -- cargo test -p obol-ffi -- --test-threads=1`. Then clippy:
  `mise exec rust@1.96.0 -- cargo clippy -p obol-ffi -- -D warnings`.

- [ ] **Step 5: Commit.**

```bash
git add crates/obol-ffi/src/lib.rs
git commit -m "feat(ffi): obol_estimate_bytes + obol_estimate_path with panic-safe boundary"
```

---

## Task 4: `obol_refresh_pricing`

Mirrors the estimate functions. The happy path needs network, so unit tests cover only the
arg-validation paths; the network path is exercised by the equivalence script (Task 8) and is
identical to the already-validated `refresh_pricing_tables`.

**Files:**
- Modify: `crates/obol-ffi/src/lib.rs`

- [ ] **Step 1: Write failing tests** (arg validation only):

```rust
#[test]
fn refresh_null_as_of_is_code_7() {
    let mut out = out_ptr();
    let code = obol_refresh_pricing(std::ptr::null(), &mut out);
    assert_eq!(code, ERR_INVALID_ARG);
    obol_string_free(out);
}

#[test]
fn refresh_null_out_is_code_7() {
    let as_of = CString::new("2026-06-05").unwrap();
    let code = obol_refresh_pricing(as_of.as_ptr(), std::ptr::null_mut());
    assert_eq!(code, ERR_INVALID_ARG);
}
```

- [ ] **Step 2: Run, expect FAIL.**

- [ ] **Step 3: Implement:**

```rust
/// Refresh pricing tables (network). `as_of` is the caller's date string. See the contract.
#[no_mangle]
pub extern "C" fn obol_refresh_pricing(as_of: *const c_char, out_json: *mut *mut c_char) -> i32 {
    if out_json.is_null() {
        return ERR_INVALID_ARG;
    }
    unsafe { *out_json = ptr::null_mut() };
    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        if as_of.is_null() {
            return fail(out_json, ERR_INVALID_ARG, "InvalidArgument", "as_of pointer is NULL");
        }
        let as_of = match CStr::from_ptr(as_of).to_str() {
            Ok(s) => s,
            Err(_) => return fail(out_json, ERR_INVALID_ARG, "InvalidArgument", "as_of is not valid UTF-8"),
        };
        finish(out_json, refresh_pricing_tables(as_of))
    }));
    match result {
        Ok(code) => code,
        Err(_) => unsafe { fail(out_json, ERR_PANIC, "Panic", "internal panic caught at FFI boundary") },
    }
}
```

- [ ] **Step 4: Run tests + clippy, expect PASS / clean.**

- [ ] **Step 5: Commit.**

```bash
git add crates/obol-ffi/src/lib.rs
git commit -m "feat(ffi): obol_refresh_pricing"
```

---

## Task 5: cbindgen header + drift test

**Files:**
- Create: `crates/obol-ffi/cbindgen.toml`
- Create: `crates/obol-ffi/examples/gen_header.rs` (writes the committed header)
- Create: `scripts/gen-header.sh` (thin wrapper over the example)
- Create: `crates/obol-ffi/include/obol.h` (generated, committed)
- Modify: `crates/obol-ffi/src/lib.rs` (add the drift test)

**Design (resolves the CLI-vs-library divergence up front):** the committed header and the drift
test are generated by the **same cbindgen version** — the single `cbindgen = "0.27"` dev-dep —
via the **same code path**. dev-dependencies are available to examples, so an
`examples/gen_header.rs` writes the header, and the `#[test]` regenerates with identical calls
and diffs. There is no separately-installed cbindgen CLI (no `cargo install`, no version skew).

- [ ] **Step 1: Write `crates/obol-ffi/cbindgen.toml`:**

```toml
language = "C"
include_guard = "OBOL_H"
tab_width = 4
documentation = true
documentation_style = "c"

[parse]
parse_deps = false

[export]
prefix = ""
```

(No `header`/`autogen_warning` keys with timestamps — keep output deterministic so the drift
test is stable.)

- [ ] **Step 2: Write `crates/obol-ffi/examples/gen_header.rs`** (the single source of truth for
  the committed header):

```rust
//! Regenerate the committed C header. Run via `scripts/gen-header.sh`.
fn main() {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    let config = cbindgen::Config::from_file(format!("{crate_dir}/cbindgen.toml")).unwrap();
    let bindings = cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen generation failed");
    let mut buf: Vec<u8> = Vec::new();
    bindings.write(&mut buf);
    let out = format!("{crate_dir}/include/obol.h");
    std::fs::create_dir_all(format!("{crate_dir}/include")).unwrap();
    std::fs::write(&out, &buf).unwrap();
    println!("wrote {out}");
}
```

- [ ] **Step 3: Write `scripts/gen-header.sh`:**

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mise exec rust@1.96.0 -- cargo run -q -p obol-ffi --example gen_header
```

Make it executable: `chmod +x scripts/gen-header.sh`.

- [ ] **Step 4: Generate the header.** Run `./scripts/gen-header.sh` and open
  `crates/obol-ffi/include/obol.h`. Verify it declares all five functions with
  `int32_t`/`uintptr_t`/`*const char`/`char**`/`const char*` types and the `OBOL_H` guard.
  (cbindgen 0.27 emits `uintptr_t` for `usize` — not `size_t`. That's expected and matches the
  spec; the Go binding's `C.uintptr_t` aligns with it.)

- [ ] **Step 5: Write the drift test** in the `tests` module of `crates/obol-ffi/src/lib.rs`.
  It regenerates via the cbindgen **library** with the same toml and asserts byte-equality —
  the exact same calls the example makes, so they cannot diverge:

```rust
#[test]
fn header_matches_source() {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    let committed = std::fs::read_to_string(format!("{crate_dir}/include/obol.h")).unwrap();
    // cbindgen 0.27 `Bindings` implements neither Display nor ToString — write into a buffer.
    let config = cbindgen::Config::from_file(format!("{crate_dir}/cbindgen.toml")).unwrap();
    let bindings = cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen generation failed");
    let mut buf: Vec<u8> = Vec::new();
    bindings.write(&mut buf);
    let generated = String::from_utf8(buf).unwrap();
    assert_eq!(
        committed, generated,
        "include/obol.h is stale — run ./scripts/gen-header.sh and commit the result"
    );
}
```

  Run it, expect PASS:
  `mise exec rust@1.96.0 -- cargo test -p obol-ffi header_matches_source -- --test-threads=1`.

- [ ] **Step 6: Full crate test + clippy.**
  `mise exec rust@1.96.0 -- cargo test -p obol-ffi -- --test-threads=1` and
  `mise exec rust@1.96.0 -- cargo clippy --all-targets -p obol-ffi -- -D warnings`.

- [ ] **Step 7: Commit.**

```bash
git add crates/obol-ffi/cbindgen.toml crates/obol-ffi/examples/gen_header.rs scripts/gen-header.sh crates/obol-ffi/include/obol.h crates/obol-ffi/src/lib.rs
git commit -m "feat(ffi): cbindgen header generation (example + drift test)"
```

---

## Task 6: Python binding (ctypes)

**Files:**
- Create: `bindings/python/obol/__init__.py`
- Create: `bindings/python/obol/_lib.py`
- Create: `bindings/python/tests/test_obol.py`
- Create: `bindings/python/README.md`

- [ ] **Step 1: Build the dylib first** (needed for the tests to load):
  `mise exec rust@1.96.0 -- cargo build -p obol-ffi`. Artifact:
  `target/debug/libobol_ffi.{dylib,so}`.

- [ ] **Step 2: Write `bindings/python/obol/_lib.py`** (dylib discovery + prototypes + the
  free-dance helper):

```python
"""ctypes loader and low-level prototypes for the obol C ABI."""
import ctypes
import os
import sys
from pathlib import Path


def _lib_filename() -> str:
    if sys.platform == "darwin":
        return "libobol_ffi.dylib"
    if sys.platform == "win32":
        return "obol_ffi.dll"
    return "libobol_ffi.so"


def _candidates():
    if os.environ.get("OBOL_LIB"):
        yield Path(os.environ["OBOL_LIB"])
    here = Path(__file__).resolve()
    name = _lib_filename()
    yield here.parent / name                      # installed beside the package
    repo = here.parents[3]                         # …/obol
    for profile in ("release", "debug"):
        yield repo / "target" / profile / name     # in-tree dev build


def _load():
    tried = []
    for p in _candidates():
        tried.append(str(p))
        if p.exists():
            return ctypes.CDLL(str(p))
    raise OSError(
        "obol_ffi shared library not found. Set OBOL_LIB or run "
        "`cargo build -p obol-ffi`. Looked in:\n  " + "\n  ".join(tried)
    )


_lib = _load()

c_char_pp = ctypes.POINTER(ctypes.POINTER(ctypes.c_char))

_lib.obol_version.restype = ctypes.c_char_p
_lib.obol_version.argtypes = []

_lib.obol_string_free.restype = None
_lib.obol_string_free.argtypes = [ctypes.POINTER(ctypes.c_char)]

_lib.obol_estimate_bytes.restype = ctypes.c_int32
_lib.obol_estimate_bytes.argtypes = [
    ctypes.c_char_p, ctypes.c_size_t, ctypes.c_char_p, c_char_pp,
]
_lib.obol_estimate_path.restype = ctypes.c_int32
_lib.obol_estimate_path.argtypes = [ctypes.c_char_p, ctypes.c_char_p, c_char_pp]

_lib.obol_refresh_pricing.restype = ctypes.c_int32
_lib.obol_refresh_pricing.argtypes = [ctypes.c_char_p, c_char_pp]


def _decode_and_free(out) -> "bytes | None":
    """Copy the obol-owned C string into Python bytes, then free it. Always frees."""
    if not out:
        return None
    try:
        return ctypes.cast(out, ctypes.c_char_p).value  # copies up to the NUL
    finally:
        _lib.obol_string_free(out)


def call(fn, *args) -> "tuple[int, bytes | None]":
    """Invoke an estimate/refresh fn whose last param is the out-pointer. Returns (code, json)."""
    out = ctypes.POINTER(ctypes.c_char)()
    code = fn(*args, ctypes.byref(out))
    return code, _decode_and_free(out)
```

- [ ] **Step 3: Write `bindings/python/obol/__init__.py`** (typed API + error):

```python
"""obol — agent-transcript cost estimation. Thin binding over the Rust core via the C ABI."""
from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

from . import _lib

__all__ = [
    "estimate_path", "estimate_bytes", "refresh", "version",
    "CostEstimate", "ModelCost", "TokenBuckets", "Approximation", "RefreshReport", "ObolError",
]


class ObolError(Exception):
    def __init__(self, code: int, kind: str, message: str):
        super().__init__(f"obol: {kind} (code {code}): {message}")
        self.code, self.kind, self.message = code, kind, message


@dataclass
class TokenBuckets:
    input: int
    output: int
    cache_read: int
    cache_write: int

    @classmethod
    def from_json(cls, d: dict) -> "TokenBuckets":
        return cls(d["input"], d["output"], d["cache_read"], d["cache_write"])


@dataclass
class ModelCost:
    model: str
    provider: str
    tokens: TokenBuckets
    subtotal_usd: float

    @classmethod
    def from_json(cls, d: dict) -> "ModelCost":
        return cls(d["model"], d["provider"], TokenBuckets.from_json(d["tokens"]), d["subtotal_usd"])


@dataclass
class Approximation:
    kind: str
    detail: Optional[str] = None

    @classmethod
    def from_json(cls, d: dict) -> "Approximation":
        return cls(d["kind"], d.get("detail"))


@dataclass
class CostEstimate:
    total_usd: float
    per_model: list[ModelCost]
    tokens: TokenBuckets
    unpriced_models: list[str]
    approximations: list[Approximation]
    pricing_as_of: str

    @classmethod
    def from_json(cls, d: dict) -> "CostEstimate":
        return cls(
            d["total_usd"],
            [ModelCost.from_json(m) for m in d["per_model"]],
            TokenBuckets.from_json(d["tokens"]),
            list(d["unpriced_models"]),
            [Approximation.from_json(a) for a in d["approximations"]],
            d["pricing_as_of"],
        )


@dataclass
class RefreshReport:
    models: int
    as_of: str
    written_to: str

    @classmethod
    def from_json(cls, d: dict) -> "RefreshReport":
        return cls(d["models"], d["as_of"], d["written_to"])


def _raise(code: int, payload: Optional[bytes]):
    kind, message = "Unknown", "no detail"
    if payload:
        try:
            err = json.loads(payload)["error"]
            kind, message = err.get("kind", kind), err.get("message", message)
            code = err.get("code", code)
        except (ValueError, KeyError):
            pass
    raise ObolError(code, kind, message)


def _estimate_result(code: int, payload: Optional[bytes]) -> CostEstimate:
    if code != 0:
        _raise(code, payload)
    return CostEstimate.from_json(json.loads(payload))


def estimate_path(path, dialect: Optional[str] = None) -> CostEstimate:
    d = dialect.encode() if dialect else None
    code, payload = _lib.call(_lib._lib.obol_estimate_path, str(path).encode(), d)
    return _estimate_result(code, payload)


def estimate_bytes(data: bytes, dialect: Optional[str] = None) -> CostEstimate:
    d = dialect.encode() if dialect else None
    code, payload = _lib.call(_lib._lib.obol_estimate_bytes, data, len(data), d)
    return _estimate_result(code, payload)


def refresh(as_of: str) -> RefreshReport:
    code, payload = _lib.call(_lib._lib.obol_refresh_pricing, as_of.encode())
    if code != 0:
        _raise(code, payload)
    return RefreshReport.from_json(json.loads(payload))


def version() -> str:
    return _lib._lib.obol_version().decode()
```

- [ ] **Step 4: Write `bindings/python/tests/test_obol.py`:**

```python
import json
import os
import shutil
import tempfile
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
TESTDATA = REPO / "bindings" / "testdata"


@pytest.fixture()
def seeded(monkeypatch):
    d = Path(tempfile.mkdtemp(prefix="obol-py-"))
    shutil.copy(TESTDATA / "prices.json", d / "current.json")
    monkeypatch.setenv("OBOL_PRICING_DIR", str(d))
    yield d
    shutil.rmtree(d, ignore_errors=True)


def test_version():
    import obol
    assert obol.version() == "0.1.0"


def test_estimate_path_matches_expectations(seeded):
    import obol
    est = obol.estimate_path(TESTDATA / "claude-mini.jsonl", dialect="claude")
    assert est.total_usd > 0.0
    assert est.pricing_as_of == "2026-06-05"
    assert isinstance(est.tokens.input, int)


def test_estimate_bytes_autodetect(seeded):
    import obol
    data = (TESTDATA / "claude-mini.jsonl").read_bytes()
    est = obol.estimate_bytes(data)  # no dialect -> auto-detect
    assert est.total_usd > 0.0


def test_missing_tables_raises(monkeypatch):
    import obol
    monkeypatch.setenv("OBOL_PRICING_DIR", "/nonexistent/obol-py-xyz")
    data = (TESTDATA / "claude-mini.jsonl").read_bytes()
    with pytest.raises(obol.ObolError) as ei:
        obol.estimate_bytes(data, dialect="claude")
    assert ei.value.code == 1
    assert ei.value.kind == "PricingTablesMissing"


def test_unknown_dialect_raises(seeded):
    import obol
    data = (TESTDATA / "claude-mini.jsonl").read_bytes()
    with pytest.raises(obol.ObolError) as ei:
        obol.estimate_bytes(data, dialect="banana")
    assert ei.value.code == 7
```

- [ ] **Step 5: Run the Python tests.** After `cargo build -p obol-ffi` (Step 1), the loader's
  `target/debug` fallback finds the dylib with no extra env:

```bash
cd bindings/python && PYTHONPATH=. python -m pytest tests -q
```

Expected: 5 passed. (Set `OBOL_LIB=/abs/path/to/libobol_ffi.<ext>` only if the dylib lives
somewhere non-standard.)

- [ ] **Step 6: Write `bindings/python/README.md`** — a short doc: install/usage, the ownership
  contract note (obol owns returned strings; the binding copies-then-frees automatically), how
  to point at the dylib (`OBOL_LIB` or `cargo build -p obol-ffi`), and that pricing tables must
  exist (`obol refresh` or a seeded `OBOL_PRICING_DIR`).

- [ ] **Step 7: Commit.**

```bash
git add bindings/python
git commit -m "feat(bindings): Python ctypes binding + tests"
```

---

## Task 7: Go binding (cgo)

**Files:**
- Create: `bindings/go/go.mod`
- Create: `bindings/go/obol/obol.go`
- Create: `bindings/go/obol/obol_test.go`
- Create: `bindings/go/README.md`

- [ ] **Step 1: Build the dylib** (if not already): `mise exec rust@1.96.0 -- cargo build -p obol-ffi`.

- [ ] **Step 2: Write `bindings/go/go.mod`:**

```
module github.com/primeradiant/obol/bindings/go

go 1.21
```

- [ ] **Step 3: Write `bindings/go/obol/obol.go`:**

```go
// Package obol is a thin cgo binding over obol-core's C ABI. The Rust core owns all
// accounting; this package only marshals C strings and unmarshals JSON.
package obol

/*
#cgo CFLAGS: -I${SRCDIR}/../../../crates/obol-ffi/include
#cgo LDFLAGS: -L${SRCDIR}/../../../target/debug -lobol_ffi -Wl,-rpath,${SRCDIR}/../../../target/debug
#include <stdlib.h>
#include "obol.h"
*/
import "C"

import (
	"encoding/json"
	"fmt"
	"unsafe"
)

type TokenBuckets struct {
	Input      uint64 `json:"input"`
	Output     uint64 `json:"output"`
	CacheRead  uint64 `json:"cache_read"`
	CacheWrite uint64 `json:"cache_write"`
}

type ModelCost struct {
	Model       string       `json:"model"`
	Provider    string       `json:"provider"`
	Tokens      TokenBuckets `json:"tokens"`
	SubtotalUSD float64      `json:"subtotal_usd"`
}

type Approximation struct {
	Kind   string `json:"kind"`
	Detail string `json:"detail,omitempty"`
}

type CostEstimate struct {
	TotalUSD       float64         `json:"total_usd"`
	PerModel       []ModelCost     `json:"per_model"`
	Tokens         TokenBuckets    `json:"tokens"`
	UnpricedModels []string        `json:"unpriced_models"`
	Approximations []Approximation `json:"approximations"`
	PricingAsOf    string          `json:"pricing_as_of"`
}

type RefreshReport struct {
	Models    uint64 `json:"models"`
	AsOf      string `json:"as_of"`
	WrittenTo string `json:"written_to"`
}

// ObolError carries the FFI error envelope.
type ObolError struct {
	Code    int    `json:"code"`
	Kind    string `json:"kind"`
	Message string `json:"message"`
}

func (e *ObolError) Error() string {
	return fmt.Sprintf("obol: %s (code %d): %s", e.Kind, e.Code, e.Message)
}

// drain copies the obol-owned C string into a Go []byte and frees it. Always frees.
func drain(out *C.char) []byte {
	if out == nil {
		return nil
	}
	defer C.obol_string_free(out)
	return []byte(C.GoString(out))
}

func toError(code int, payload []byte) error {
	e := &ObolError{Code: code, Kind: "Unknown", Message: "no detail"}
	if len(payload) > 0 {
		var env struct {
			Error ObolError `json:"error"`
		}
		if json.Unmarshal(payload, &env) == nil && env.Error.Code != 0 {
			*e = env.Error
		}
	}
	return e
}

func decodeEstimate(code C.int32_t, payload []byte) (*CostEstimate, error) {
	if int(code) != 0 {
		return nil, toError(int(code), payload)
	}
	var est CostEstimate
	if err := json.Unmarshal(payload, &est); err != nil {
		return nil, err
	}
	return &est, nil
}

// EstimatePath estimates a transcript file's cost. dialect "" means auto-detect.
func EstimatePath(path, dialect string) (*CostEstimate, error) {
	cPath := C.CString(path)
	defer C.free(unsafe.Pointer(cPath))
	cDialect := dialectArg(dialect)
	defer freeDialect(cDialect)
	var out *C.char
	code := C.obol_estimate_path(cPath, cDialect, &out)
	return decodeEstimate(code, drain(out))
}

// EstimateBytes estimates in-memory transcript bytes. dialect "" means auto-detect.
func EstimateBytes(data []byte, dialect string) (*CostEstimate, error) {
	var dptr *C.uint8_t
	if len(data) > 0 {
		dptr = (*C.uint8_t)(unsafe.Pointer(&data[0]))
	} else {
		dptr = (*C.uint8_t)(unsafe.Pointer(&[]byte{0}[0])) // non-nil for len 0
	}
	cDialect := dialectArg(dialect)
	defer freeDialect(cDialect)
	var out *C.char
	code := C.obol_estimate_bytes(dptr, C.uintptr_t(len(data)), cDialect, &out)
	return decodeEstimate(code, drain(out))
}

// Refresh pulls fresh pricing tables. asOf is the caller's date string.
func Refresh(asOf string) (*RefreshReport, error) {
	cAsOf := C.CString(asOf)
	defer C.free(unsafe.Pointer(cAsOf))
	var out *C.char
	code := C.obol_refresh_pricing(cAsOf, &out)
	payload := drain(out)
	if int(code) != 0 {
		return nil, toError(int(code), payload)
	}
	var r RefreshReport
	if err := json.Unmarshal(payload, &r); err != nil {
		return nil, err
	}
	return &r, nil
}

// Version returns the obol library version (static C string; not freed).
func Version() string {
	return C.GoString(C.obol_version())
}

func dialectArg(dialect string) *C.char {
	if dialect == "" {
		return nil
	}
	return C.CString(dialect)
}

func freeDialect(p *C.char) {
	if p != nil {
		C.free(unsafe.Pointer(p))
	}
}
```

Note on the cbindgen header type for `len`: the spec fixes it as `uintptr_t`, so the cgo call
uses `C.uintptr_t`. If cbindgen emits `size_t` instead in your generated header, change the cast
to `C.size_t` to match `obol.h` exactly (the header is the source of truth here).

- [ ] **Step 4: Write `bindings/go/obol/obol_test.go`:**

```go
package obol

import (
	"os"
	"path/filepath"
	"testing"
)

func repoRoot(t *testing.T) string {
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	return filepath.Join(wd, "..", "..", "..") // bindings/go/obol -> repo root
}

func testdata(t *testing.T) string { return filepath.Join(repoRoot(t), "bindings", "testdata") }

func seed(t *testing.T) {
	dir := t.TempDir()
	src, err := os.ReadFile(filepath.Join(testdata(t), "prices.json"))
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "current.json"), src, 0o644); err != nil {
		t.Fatal(err)
	}
	t.Setenv("OBOL_PRICING_DIR", dir)
}

func TestVersion(t *testing.T) {
	if Version() == "" {
		t.Fatal("empty version")
	}
}

func TestEstimatePath(t *testing.T) {
	seed(t)
	est, err := EstimatePath(filepath.Join(testdata(t), "claude-mini.jsonl"), "claude")
	if err != nil {
		t.Fatal(err)
	}
	if est.TotalUSD <= 0 {
		t.Fatalf("expected positive total, got %v", est.TotalUSD)
	}
	if est.PricingAsOf != "2026-06-05" {
		t.Fatalf("unexpected pricing_as_of %q", est.PricingAsOf)
	}
}

func TestMissingTablesIsError(t *testing.T) {
	t.Setenv("OBOL_PRICING_DIR", "/nonexistent/obol-go-xyz")
	data, _ := os.ReadFile(filepath.Join(testdata(t), "claude-mini.jsonl"))
	_, err := EstimateBytes(data, "claude")
	oe, ok := err.(*ObolError)
	if !ok || oe.Code != 1 {
		t.Fatalf("expected ObolError code 1, got %v", err)
	}
}

func TestUnknownDialectIsError(t *testing.T) {
	seed(t)
	data, _ := os.ReadFile(filepath.Join(testdata(t), "claude-mini.jsonl"))
	_, err := EstimateBytes(data, "banana")
	oe, ok := err.(*ObolError)
	if !ok || oe.Code != 7 {
		t.Fatalf("expected ObolError code 7, got %v", err)
	}
}
```

- [ ] **Step 5: Run the Go tests.** The `-Wl,-rpath` in the LDFLAGS bakes the lib directory
  into the test binary, so no `DYLD_LIBRARY_PATH`/`LD_LIBRARY_PATH` is needed at runtime — just
  build the dylib first:

```bash
mise exec rust@1.96.0 -- cargo build -p obol-ffi
cd bindings/go && CGO_ENABLED=1 go test ./...
```

Expected: ok, all tests pass. If the linker can't find `-lobol_ffi`, confirm
`target/debug/libobol_ffi.{dylib,so}` exists (`cargo build -p obol-ffi`). (On macOS the Rust
cdylib's install-name is the absolute build path, so it would load even without the rpath; the
rpath makes Linux work env-free too.)

- [ ] **Step 6: Write `bindings/go/README.md`** — usage, the `#cgo` CFLAGS/LDFLAGS explanation
  (including the baked `-Wl,-rpath` that makes it run env-free after `cargo build -p obol-ffi`;
  note that linking against a release build means editing the LDFLAGS path or rebuilding debug),
  the ownership-contract note (returned strings copied-then-freed by `drain`), and the pricing
  prerequisite.

- [ ] **Step 7: Commit.**

```bash
git add bindings/go
git commit -m "feat(bindings): Go cgo binding + tests"
```

---

## Task 8: Cross-language equivalence gate + validation doc

Prove the seam is faithful: Rust CLI, Python, and Go produce a byte-identical `total_usd`.

**Files:**
- Create: `bindings/go/cmd/total/main.go` (prints the Go binding's `total_usd`)
- Create: `scripts/validate_bindings.sh`
- Create: `docs/validation-ffi-2026-06-05.md`

- [ ] **Step 1: Write `bindings/go/cmd/total/main.go`** — a real, required helper (not "if
  present") that estimates the fixture and prints just the total:

```go
package main

import (
	"fmt"
	"os"
	"strconv"

	"github.com/primeradiant/obol/bindings/go/obol"
)

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: total <transcript> [dialect]")
		os.Exit(2)
	}
	dialect := ""
	if len(os.Args) >= 3 {
		dialect = os.Args[2]
	}
	est, err := obol.EstimatePath(os.Args[1], dialect)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Println(strconv.FormatFloat(est.TotalUSD, 'g', -1, 64))
}
```

- [ ] **Step 2: Write `scripts/validate_bindings.sh`** — a true three-way **value** comparison.
  All three totals are normalized through one Python `float()` parse before comparison, so we
  compare IEEE-754 values, never formatting. A Go failure fails the gate (no `|| true`):

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd)"
RUST="mise exec rust@1.96.0 -- cargo"

echo "building obol-ffi + cli…"
$RUST build -p obol-ffi -p obol-cli

LIBDIR="$ROOT/target/debug"
LIBNAME="libobol_ffi.$([ "$(uname)" = Darwin ] && echo dylib || echo so)"
export OBOL_LIB="$LIBDIR/$LIBNAME"

SEED="$(mktemp -d)"; trap 'rm -rf "$SEED"' EXIT
cp bindings/testdata/prices.json "$SEED/current.json"
export OBOL_PRICING_DIR="$SEED"
T="bindings/testdata/claude-mini.jsonl"

# Normalize any numeric string to Python's shortest round-trip repr of its f64 value.
norm() { python3 -c 'import sys; print(repr(float(sys.stdin.read().strip())))'; }

rust_total=$($RUST run -q -p obol-cli -- estimate "$T" --dialect claude --json \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["total_usd"])' | norm)
py_total=$( (cd bindings/python && PYTHONPATH=. python3 -c \
  "import obol; print(obol.estimate_path('$ROOT/$T', dialect='claude').total_usd)") | norm)
go_total=$( (cd bindings/go && go run ./cmd/total "$ROOT/$T" claude) | norm)

echo "rust : $rust_total"
echo "py   : $py_total"
echo "go   : $go_total"

if [ "$rust_total" = "$py_total" ] && [ "$py_total" = "$go_total" ]; then
  echo "OK: rust == python == go total_usd ($rust_total)"
else
  echo "MISMATCH: rust=$rust_total python=$py_total go=$go_total"; exit 1
fi
```

(The `norm()` step re-parses every total — including Go's `FormatFloat` output, which may use a
different exponent style than serde — back to an f64 and prints Python's canonical shortest repr,
so equal values compare equal regardless of source formatting. The comparison is strictly
value-based.)

- [ ] **Step 3: Make it executable and run it.** `chmod +x scripts/validate_bindings.sh && ./scripts/validate_bindings.sh`. Expected: `OK: rust == python == go total_usd (…)`.

- [ ] **Step 4: Write `docs/validation-ffi-2026-06-05.md`** documenting: what was validated
  (three consumers, one transcript, one snapshot → identical `total_usd`), the exact command,
  the observed totals, and the conclusion (the seam is faithful; bindings re-type without
  drift). Mirror the style of `docs/validation-pi-2026-06-05.md`.

- [ ] **Step 5: Commit.**

```bash
git add bindings/go/cmd scripts/validate_bindings.sh docs/validation-ffi-2026-06-05.md
git commit -m "test: cross-language equivalence gate (Rust/Python/Go) + validation doc"
```

---

## Final verification (after all tasks)

- [ ] `mise exec rust@1.96.0 -- cargo test -- --test-threads=1` — whole workspace green.
- [ ] `mise exec rust@1.96.0 -- cargo clippy --all-targets -- -D warnings` — clean.
- [ ] `./scripts/validate_bindings.sh` — three-language totals agree.
- [ ] `cargo build -p obol-ffi` then Python + Go test suites pass against the built dylib.
- [ ] Dispatch a final code reviewer over the whole branch before finishing.

## Self-review notes (plan author)

- **Type consistency:** the JSON field names (`total_usd`, `per_model`, `subtotal_usd`,
  `cache_read`, `cache_write`, `pricing_as_of`, `unpriced_models`, `kind`/`detail`) match
  `model.rs`'s serde output exactly, and are reused identically in the Python dataclasses and Go
  struct tags. `provider` is a string in both bindings (matches `Provider`'s custom Serialize).
- **`len` C type:** the plan uses `uintptr_t`/`C.uintptr_t` per the spec, with an explicit note
  to match whatever `obol.h` actually emits (the header is the source of truth; the drift test
  enforces it).
- **Single-thread tests:** every Rust test that touches `OBOL_PRICING_DIR` runs under
  `--test-threads=1` (already the repo norm). Python/Go use per-process temp dirs via
  `monkeypatch`/`t.Setenv`/`t.TempDir`, so they're isolated without a global flag.
- **Header generation has one source of truth:** the committed header and the drift-test
  expectation are both produced by the single pinned `cbindgen` dev-dep via identical calls
  (the `examples/gen_header.rs` writer and the `header_matches_source` test). No separately-
  installed CLI, so no version-skew path that could leave the test permanently red.
