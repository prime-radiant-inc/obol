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
}
