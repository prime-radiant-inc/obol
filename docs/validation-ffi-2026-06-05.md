# Cross-language FFI validation: Rust CLI vs Python vs Go (2026-06-05)

PRI-2084 acceptance gate. Confirms the `obol-ffi` C ABI seam is faithful: the Rust
CLI, the Python (ctypes) binding, and the Go (cgo) binding all produce a byte-for-byte
identical `total_usd` for the SAME transcript priced against the SAME on-disk pricing
snapshot. The bindings re-type the core's JSON; they never re-implement accounting, and
this gate proves there is no drift across the seam.

## Method

- Single transcript: `bindings/testdata/claude-mini.jsonl` (model `claude-opus-4-8`,
  Claude dialect).
- Single pricing snapshot: `bindings/testdata/prices.json` copied to
  `$OBOL_PRICING_DIR/current.json` (a temp dir), so all three consumers read the exact
  same rates.
- Three consumers, one fixture:
  - Rust:   `obol-cli estimate <T> --dialect claude --json`, `total_usd` field.
  - Python: `obol.estimate_path(<T>, dialect='claude').total_usd` (ctypes over the dylib).
  - Go:     `bindings/go/cmd/total <T> claude` (cgo over the dylib).
- All three totals are normalized through one Python `float()` parse before comparison
  (`repr(float(x))`), so the comparison is strictly value-based (IEEE-754), never sensitive
  to source formatting (e.g. Go's `FormatFloat` exponent style vs serde's).
- The gate fails loudly on any mismatch — a Go failure is not swallowed.
- Reproducer: `scripts/validate_bindings.sh`.

## Results

```
rust : 0.000995
py   : 0.000995
go   : 0.000995
OK: rust == python == go total_usd (0.000995)
```

| Consumer | path | total_usd |
|---|---|---|
| Rust CLI | `obol-cli estimate --json` | 0.000995 |
| Python (ctypes) | `obol.estimate_path` | 0.000995 |
| Go (cgo) | `cmd/total` | 0.000995 |

All three agree to the full IEEE-754 value. The seam is faithful.

## Per-binding test suites

Beyond the equivalence gate, each binding has its own test suite exercising the success
path, the missing-pricing-tables error (code 1 → `PricingTablesMissing`), and the
unknown-dialect error (code 7), plus version:

- Python: `cd bindings/python && PYTHONPATH=. python -m pytest tests -q` — 5 passed.
- Go:     `cd bindings/go && CGO_ENABLED=1 go test ./...` — ok (Version, EstimatePath,
  MissingTables→code 1, UnknownDialect→code 7).

Both run env-free after `cargo build -p obol-ffi`: the Python loader falls back to
`target/debug`, and the Go binding bakes `-Wl,-rpath,…/target/debug` into the test binary.

## Linux verification (closes the macOS-only risk)

The above was developed on macOS (`.dylib`). The whole stack was then re-verified from a
clean checkout inside a stock `ubuntu:24.04` container (linux/aarch64), with Rust 1.96 via
rustup, Go 1.22, and Python 3.12 freshly installed. Reproducer: `/tmp/obol-linux-verify.sh`
(clone `/src` → install toolchains → run every gate). All passed:

- **Workspace tests:** 38 passed (1 cli + 24 core + 13 ffi), including the cbindgen
  `header_matches_source` drift test — so `include/obol.h` is byte-identical when regenerated
  on Linux, and `usize` still emits as `uintptr_t`.
- **clippy** `--all-targets -D warnings` — clean.
- **cdylib:** `target/debug/libobol_ffi.so` — `ELF 64-bit LSB shared object, ARM aarch64`.
- **Go:** `go test ./...` — ok; cgo links against `libobol_ffi.so` + `obol.h`.
- **rpath proven env-free (the previously-untested claim):** the cgo binary carries
  `DT_RUNPATH = …/target/debug`; run with `env -u LD_LIBRARY_PATH` it prints `0.000995`, and
  `ldd` resolves `libobol_ffi.so => …/target/debug/libobol_ffi.so` with **no**
  `LD_LIBRARY_PATH` set. The baked `-Wl,-rpath` works on Linux as designed.
- **Python:** 5 passed.
- **Equivalence gate on Linux:** `rust == python == go == 0.000995`.

So the C ABI, both bindings, and the env-free rpath linking all hold on Linux/aarch64, not
just macOS. (x86-64 Linux and Windows remain unexercised, but nothing here is arch- or
libc-specific beyond what was just confirmed portable.)

## Conclusion

The C ABI is the single seam and the Rust core is the single source of truth. Two
independent foreign bindings, written in different languages with different FFI mechanisms
(ctypes vs cgo) and different JSON decoders, reproduce the Rust CLI's `total_usd` exactly.
No drift. Bindings re-type, never re-implement.

## Bugs found

None.
