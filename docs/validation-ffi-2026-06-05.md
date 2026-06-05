# Cross-language FFI validation: Rust CLI vs Python vs Go vs TypeScript (2026-06-05)

PRI-2084 + PRI-2085 acceptance gate. Confirms the `obol-ffi` C ABI seam is faithful: the Rust
CLI, the Python (ctypes) binding, the Go (cgo) binding, and the TypeScript binding under both
Bun (`bun:ffi`) and Node (`koffi`) all produce a byte-for-byte identical `total_usd` for the
SAME transcript priced against the SAME on-disk pricing snapshot. The bindings re-type the
core's JSON; they never re-implement accounting, and this gate proves there is no drift across
the seam.

## Method

- Single transcript: `bindings/testdata/claude-mini.jsonl` (model `claude-opus-4-8`,
  Claude dialect).
- Single pricing snapshot: `bindings/testdata/prices.json` copied to
  `$OBOL_PRICING_DIR/current.json` (a temp dir), so all five consumers read the exact
  same rates.
- Five consumers, one fixture:
  - Rust:    `obol-cli estimate <T> --dialect claude --json`, `total_usd` field.
  - Python:  `obol.estimate_path(<T>, dialect='claude').total_usd` (ctypes over the dylib).
  - Go:      `bindings/go/cmd/total <T> claude` (cgo over the dylib).
  - TS/Bun:  `bun  bindings/typescript/total.ts <T> claude` (`bun:ffi` over the dylib).
  - TS/Node: `node bindings/typescript/total.ts <T> claude` (`koffi` over the dylib).
- All five totals are normalized through one Python `float()` parse before comparison
  (`repr(float(x))`), so the comparison is strictly value-based (IEEE-754), never sensitive
  to source formatting (e.g. Go's `FormatFloat` exponent style vs serde's).
- The gate fails loudly on any mismatch — a Go failure is not swallowed.
- Reproducer: `scripts/validate_bindings.sh`.

## Results

```
rust    : 0.000995
py      : 0.000995
go      : 0.000995
ts(bun) : 0.000995
ts(node): 0.000995
OK: rust == python == go == ts(bun) == ts(node) total_usd (0.000995)
```

| Consumer | path | total_usd |
|---|---|---|
| Rust CLI | `obol-cli estimate --json` | 0.000995 |
| Python (ctypes) | `obol.estimate_path` | 0.000995 |
| Go (cgo) | `cmd/total` | 0.000995 |
| TS / Bun (`bun:ffi`) | `total.ts` | 0.000995 |
| TS / Node (`koffi`) | `total.ts` | 0.000995 |

All five agree to the full IEEE-754 value. The seam is faithful.

## Per-binding test suites

Beyond the equivalence gate, each binding has its own test suite exercising the success
path, the missing-pricing-tables error (code 1 → `PricingTablesMissing`), and the
unknown-dialect error (code 7), plus version:

- Python: `cd bindings/python && PYTHONPATH=. python -m pytest tests -q` — 5 passed.
- Go:     `cd bindings/go && CGO_ENABLED=1 go test ./...` — ok (Version, EstimatePath,
  MissingTables→code 1, UnknownDialect→code 7).
- TS:     `cd bindings/typescript && bun test` — 5 pass; `node --test test/obol.test.ts` — 5 pass.
  The *same* `node:test` file runs under both runtimes (version, estimatePath success,
  estimateBytes auto-detect, missing-tables→code 1, unknown-dialect→code 7).

Both Python and Go run env-free after `cargo build -p obol-ffi`: the Python loader falls back to
`target/debug`, and the Go binding bakes `-Wl,-rpath,…/target/debug` into the test binary. The TS
loader also falls back to `target/debug`. One TS caveat (handled in `test/pricing-env.ts`): **Bun
does not propagate runtime `process.env` writes to the native `getenv`** the Rust core reads, so
the test suite sets `OBOL_PRICING_DIR` via libc `setenv` under Bun; Node propagates it natively.

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

The TypeScript binding (PRI-2085) was Linux-verified the same way, in a stock `ubuntu:24.04`
container with Node 24 (NodeSource) and Bun installed (reproducer: `/tmp/obol-ts-linux-verify.sh`):
the cdylib built as `libobol_ffi.so`; `koffi`'s `linux_arm64` prebuild installed; the TS test
suite passed **5/5 under Bun and 5/5 under Node** (the same `node:test` file under both); and the
**five-language gate** ran green: `rust == python == go == ts(bun) == ts(node) == 0.000995`. So
`bun:ffi`/`koffi` `dlopen` of a `.so`, and the libc-`setenv` env shim under Bun, both work on
Linux.

So the C ABI, all four bindings (Python, Go, and TS under Bun + Node), and the env-free rpath
linking all hold on Linux/aarch64, not just macOS. (x86-64 Linux and Windows remain unexercised,
but nothing here is arch- or libc-specific beyond what was just confirmed portable.)

## Conclusion

The C ABI is the single seam and the Rust core is the single source of truth. Two
independent foreign bindings, written in different languages with different FFI mechanisms
(ctypes vs cgo) and different JSON decoders, reproduce the Rust CLI's `total_usd` exactly.
No drift. Bindings re-type, never re-implement.

## Bugs found

None.
