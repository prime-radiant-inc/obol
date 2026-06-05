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
