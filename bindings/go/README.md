# obol — Go binding (cgo)

A thin cgo binding over obol's C ABI (`obol-ffi`). It links the prebuilt shared library at
build time and re-types the JSON the Rust core returns into idiomatic Go structs. The Rust
core stays the single source of truth for all accounting; this package only marshals C
strings and unmarshals JSON.

## Build prerequisites

cgo requires a C compiler (clang on macOS, gcc on Linux) and `CGO_ENABLED=1` (the default
when a C toolchain is present). You also need the `obol-ffi` shared library
(`libobol_ffi.dylib` on macOS, `libobol_ffi.so` on Linux):

```bash
mise exec rust@1.96.0 -- cargo build -p obol-ffi
```

## How the `#cgo` directives find the library

The package preamble points the compiler and linker at the in-tree build:

```
#cgo CFLAGS:  -I${SRCDIR}/../../../crates/obol-ffi/include
#cgo LDFLAGS: -L${SRCDIR}/../../../target/debug -lobol_ffi -Wl,-rpath,${SRCDIR}/../../../target/debug
```

`${SRCDIR}` is `bindings/go/obol`, so the three `../` reach the repo root. `CFLAGS` finds the
committed `obol.h`; `LDFLAGS` links `-lobol_ffi` from `target/debug` and bakes that directory
into the binary's runtime search path with `-Wl,-rpath`. After `cargo build -p obol-ffi` the
tests and `cmd/total` run **env-free** — no `DYLD_LIBRARY_PATH` / `LD_LIBRARY_PATH` needed.

To link against a release build instead, build with `--release` and edit the two
`target/debug` paths in `obol.go` to `target/release` (or rebuild the debug dylib).

## Usage

```go
import "github.com/primeradiant/obol/bindings/go/obol"

obol.Version() // "0.1.0"

est, err := obol.EstimatePath("transcript.jsonl", "claude") // "" dialect auto-detects
// est.TotalUSD, est.PricingAsOf, est.PerModel[i].{Model,Provider,SubtotalUSD}

est, err = obol.EstimateBytes(data, "") // from in-memory bytes, auto-detect

report, err := obol.Refresh("2026-06-05") // refresh the on-disk pricing snapshot (network)
```

On a nonzero status the call returns an `*obol.ObolError` carrying `.Code`, `.Kind`, and
`.Message` from the FFI error envelope.

## Pricing tables must exist

`EstimatePath` / `EstimateBytes` read a pricing snapshot from disk. Either run `obol refresh`
(the CLI), or point `OBOL_PRICING_DIR` at a directory containing `current.json`. With no
snapshot the call returns an `*ObolError` with `Kind == "PricingTablesMissing"` (code 1).

## Ownership & safety contract

obol owns every string it returns through an out-parameter. This binding honors the contract
in `drain`: it copies the obol-owned C string into a Go `[]byte` with `C.GoString` (which
copies up to the NUL), **then** `defer`s `C.obol_string_free`, so the Rust-owned pointer never
outlives the copy and is never freed twice. A `nil` out-pointer is handled. `obol_version`
returns a static C string and is never freed. The public API returns plain Go structs — you
manage none of this yourself.

## Tests

```bash
mise exec rust@1.96.0 -- cargo build -p obol-ffi   # build the dylib first
cd bindings/go && CGO_ENABLED=1 go test ./...
```
