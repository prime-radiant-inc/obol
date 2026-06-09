// The exported `extern "C"` entry points take and dereference raw C pointers but keep a
// safe (non-`unsafe fn`) signature on purpose: they are the C-callable boundary, and each
// upholds the pointer contract internally (NULL checks + documented ownership rules below).
// clippy's `not_unsafe_ptr_arg_deref` flags this idiomatic FFI shape; allow it crate-wide.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

//! obol-ffi: a C ABI over obol-core. JSON at the seam; the Rust core owns all accounting.
//!
//! OWNERSHIP & SAFETY CONTRACT (honor in every binding):
//!  1. Inputs (path/dialect/as_of) are borrowed; obol copies what it needs before
//!     returning. Caller may free them immediately after the call.
//!  2. Every `*out_json` is obol-owned (Rust allocator). Free ONLY via `obol_string_free`.
//!     Freeing any other way is undefined behavior. `obol_string_free(NULL)` is a no-op.
//!  3. Each function NULL-inits `*out_json` first, then runs inside catch_unwind: a caught
//!     panic yields status 8 and leaves a freeable string-or-NULL, never garbage.
//!  4. NULL required pointer -> status 7. NULL `dialect` -> status 7 (dialect is required).
//!  5. `obol_estimate_path` is reentrant/stateless. `obol_refresh_pricing` writes the on-disk
//!     snapshot; concurrent refresh is the caller's concern (same as the Rust lib).
//!  6. Input strings must be valid UTF-8; output JSON always is.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::ptr;

use obol_core::{estimate_cost, refresh_pricing_tables, Dialect, ObolError};

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

fn code_and_kind(e: &ObolError) -> (i32, &'static str) {
    match e {
        ObolError::PricingTablesMissing(_) => (ERR_PRICING_MISSING, "PricingTablesMissing"),
        ObolError::UnknownDialect => (ERR_UNKNOWN_DIALECT, "UnknownDialect"),
        ObolError::MalformedTranscript { .. } => (ERR_MALFORMED, "MalformedTranscript"),
        ObolError::InvalidAsOf(_) => (ERR_INVALID_ARG, "InvalidArgument"),
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

/// NULL/unknown/invalid-UTF-8 dialect -> Err(()). Known string -> Ok(Dialect).
fn parse_dialect(dialect: *const c_char) -> Result<Dialect, ()> {
    if dialect.is_null() {
        return Err(());
    }
    let s = unsafe { CStr::from_ptr(dialect) }
        .to_str()
        .map_err(|_| ())?;
    match s {
        "claude" => Ok(Dialect::Claude),
        "codex" => Ok(Dialect::Codex),
        "pi" => Ok(Dialect::Pi),
        "gemini" => Ok(Dialect::Gemini),
        "opencode" => Ok(Dialect::Opencode),
        "copilot" => Ok(Dialect::Copilot),
        "kimi" => Ok(Dialect::Kimi),
        "obol" => Ok(Dialect::Obol),
        _ => Err(()),
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
            return fail(
                out_json,
                ERR_INVALID_ARG,
                "InvalidArgument",
                "path pointer is NULL",
            );
        }
        let path = match CStr::from_ptr(path).to_str() {
            Ok(s) => s,
            Err(_) => {
                return fail(
                    out_json,
                    ERR_INVALID_ARG,
                    "InvalidArgument",
                    "path is not valid UTF-8",
                )
            }
        };
        let dialect = match parse_dialect(dialect) {
            Ok(d) => d,
            Err(()) => {
                return fail(
                    out_json,
                    ERR_INVALID_ARG,
                    "InvalidArgument",
                    "dialect is required and must be a known value (NULL/unknown rejected)",
                );
            }
        };
        finish(out_json, estimate_cost(Path::new(path), dialect))
    }));
    match result {
        Ok(code) => code,
        Err(_) => unsafe {
            fail(
                out_json,
                ERR_PANIC,
                "Panic",
                "internal panic caught at FFI boundary",
            )
        },
    }
}

/// Refresh pricing tables (network). `as_of` is the caller's date string. See the contract.
#[no_mangle]
pub extern "C" fn obol_refresh_pricing(as_of: *const c_char, out_json: *mut *mut c_char) -> i32 {
    if out_json.is_null() {
        return ERR_INVALID_ARG;
    }
    unsafe { *out_json = ptr::null_mut() };
    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        if as_of.is_null() {
            return fail(
                out_json,
                ERR_INVALID_ARG,
                "InvalidArgument",
                "as_of pointer is NULL",
            );
        }
        let as_of = match CStr::from_ptr(as_of).to_str() {
            Ok(s) => s,
            Err(_) => {
                return fail(
                    out_json,
                    ERR_INVALID_ARG,
                    "InvalidArgument",
                    "as_of is not valid UTF-8",
                )
            }
        };
        finish(out_json, refresh_pricing_tables(as_of))
    }));
    match result {
        Ok(code) => code,
        Err(_) => unsafe {
            fail(
                out_json,
                ERR_PANIC,
                "Panic",
                "internal panic caught at FFI boundary",
            )
        },
    }
}

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

    #[test]
    fn maps_obol_errors_to_codes() {
        use obol_core::ObolError;
        assert_eq!(
            code_and_kind(&ObolError::UnknownDialect),
            (ERR_UNKNOWN_DIALECT, "UnknownDialect")
        );
        assert_eq!(
            code_and_kind(&ObolError::MalformedTranscript {
                line: 1,
                msg: "x".into()
            })
            .0,
            ERR_MALFORMED
        );
        assert_eq!(
            code_and_kind(&ObolError::Network("x".into())),
            (ERR_NETWORK, "Network")
        );
    }

    #[test]
    fn envelope_is_valid_json_with_fields() {
        let s = envelope(ERR_MALFORMED, "MalformedTranscript", "bad: \"quote\"");
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["error"]["code"], ERR_MALFORMED);
        assert_eq!(v["error"]["kind"], "MalformedTranscript");
        assert_eq!(v["error"]["message"], "bad: \"quote\"");
    }

    use std::ffi::CString;
    use std::path::PathBuf;

    // Seed a temp pricing dir from the shared prices fixture; returns the dir.
    fn seed_pricing() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "obol-ffi-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("OBOL_PRICING_DIR", &dir);
        let prices = include_bytes!("../../../bindings/testdata/prices.json");
        std::fs::write(dir.join("current.json"), prices).unwrap();
        dir
    }

    fn out_ptr() -> *mut c_char {
        std::ptr::null_mut()
    }

    #[test]
    fn estimate_path_success_with_seeded_store() {
        let dir = seed_pricing();
        let f = dir.join("session.jsonl");
        std::fs::write(
            &f,
            include_bytes!("../tests/fixtures/claude-mini.jsonl").as_slice(),
        )
        .unwrap();
        let cpath = CString::new(f.to_str().unwrap()).unwrap();
        let claude = CString::new("claude").unwrap();
        let mut out = out_ptr();
        let code = obol_estimate_path(cpath.as_ptr(), claude.as_ptr(), &mut out);
        assert_eq!(code, OK, "code={code}");
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        obol_string_free(out);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["total_usd"].as_f64().unwrap() > 0.0, "{json}");
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("OBOL_PRICING_DIR");
    }

    #[test]
    fn estimate_path_success_with_obol_dialect() {
        let dir = seed_pricing();
        let f = dir.join("usage.jsonl");
        std::fs::write(
            &f,
            include_bytes!("../../obol-core/tests/fixtures/obol-usage-mini.jsonl").as_slice(),
        )
        .unwrap();
        let cpath = CString::new(f.to_str().unwrap()).unwrap();
        let obol = CString::new("obol").unwrap();
        let mut out = out_ptr();
        let code = obol_estimate_path(cpath.as_ptr(), obol.as_ptr(), &mut out);
        assert_eq!(code, OK, "code={code}");
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        obol_string_free(out);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["total_usd"].as_f64().unwrap() > 0.0, "{json}");
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("OBOL_PRICING_DIR");
    }

    #[test]
    fn estimate_path_null_dialect_is_invalid_arg() {
        let dir = seed_pricing();
        let f = dir.join("session.jsonl");
        std::fs::write(&f, b"{}").unwrap();
        let cpath = CString::new(f.to_str().unwrap()).unwrap();
        let mut out = out_ptr();
        let code = obol_estimate_path(cpath.as_ptr(), std::ptr::null(), &mut out);
        assert_eq!(code, ERR_INVALID_ARG, "NULL dialect must be rejected");
        obol_string_free(out);
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("OBOL_PRICING_DIR");
    }

    #[test]
    fn estimate_path_bad_path_is_io_error() {
        let dir = seed_pricing();
        let p = CString::new("/nonexistent/obol/transcript.jsonl").unwrap();
        let claude = CString::new("claude").unwrap();
        let mut out = out_ptr();
        let code = obol_estimate_path(p.as_ptr(), claude.as_ptr(), &mut out);
        assert_eq!(code, ERR_IO, "code={code}");
        obol_string_free(out);
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("OBOL_PRICING_DIR");
    }

    #[test]
    fn refresh_null_as_of_is_code_7() {
        let mut out = out_ptr();
        let code = obol_refresh_pricing(std::ptr::null(), &mut out);
        assert_eq!(code, ERR_INVALID_ARG);
        obol_string_free(out);
    }

    #[test]
    fn refresh_garbage_as_of_is_invalid_arg() {
        let as_of = CString::new("Apr-2027").unwrap();
        let mut out = out_ptr();
        let code = obol_refresh_pricing(as_of.as_ptr(), &mut out);
        assert_eq!(code, ERR_INVALID_ARG, "code={code}");
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        obol_string_free(out);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["error"]["kind"], "InvalidArgument", "{json}");
    }

    #[test]
    fn refresh_null_out_is_code_7() {
        let as_of = CString::new("2026-06-05").unwrap();
        let code = obol_refresh_pricing(as_of.as_ptr(), std::ptr::null_mut());
        assert_eq!(code, ERR_INVALID_ARG);
    }

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
}
