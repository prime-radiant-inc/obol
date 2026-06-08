# obol — Go binding (purego)

A thin [purego](https://github.com/ebitengine/purego) binding over obol's C ABI (`obol-ffi`). It
loads the prebuilt shared library at **runtime** (`dlopen`) and re-types the JSON the Rust core
returns into idiomatic Go structs. **No cgo** — `CGO_ENABLED=0` works, so consumers need no C
compiler. The Rust core stays the single source of truth for all accounting; this package only
marshals C strings and unmarshals JSON.

## Consuming it: `obol-go`

Published consumers don't use this directory — they import the generated, self-contained module
that bundles the native libraries:

```go
import "github.com/prime-radiant-inc/obol-go" // go get github.com/prime-radiant-inc/obol-go
```

That module is generated from *this* source by the release workflow (`scripts/assemble-obol-go.sh`):
it embeds the per-platform `libobol_ffi` and extracts+`dlopen`s it on first use, so a plain
`go get` works on macOS (arm64/x64) and Linux (x64/arm64) with no toolchain. This `bindings/go/`
tree is the embed-free **source of truth**, used for in-repo development and the equivalence gate.

## How the library is located

The loader resolves `libobol_ffi` in this order (first hit wins):

1. **`OBOL_LIB`** — an explicit path to the shared library. Overrides everything.
2. **Embedded** — in the published `obol-go` module, the platform library is embedded and extracted
   to a content-hashed dir under `os.UserCacheDir()` (falling back to the temp dir), then `dlopen`'d.
   Absent in this dev tree.
3. **Dev `target/`** — repo-relative `target/release` then `target/debug`, located from the package
   source file. So after `mise exec rust@1.96.0 -- cargo build -p obol-ffi` the tests and
   `cmd/total` run **env-free**.

On macOS under a hardened runtime with library validation, an unsigned extracted dylib may be
rejected — point `OBOL_LIB` at a signed copy in that case.

## Usage

```go
import "github.com/prime-radiant-inc/obol-go" // or the in-repo package during development

obol.Version() // "0.1.1" (the Rust core version)

est, err := obol.EstimatePath("transcript.jsonl", "claude")
// est.TotalUSD, est.PricingAsOf, est.PerModel[i].{Model,Provider,SubtotalUSD}

report, err := obol.Refresh("2026-06-05") // refresh the on-disk pricing snapshot (network)
```

On a nonzero status the call returns an `*obol.ObolError` carrying `.Code`, `.Kind`, and
`.Message` from the FFI error envelope.

## Pricing tables must exist

`EstimatePath` reads a pricing snapshot from disk. Either run `obol refresh`
(the CLI), or point `OBOL_PRICING_DIR` at a directory containing `current.json`. With no
snapshot the call returns an `*ObolError` with `Kind == "PricingTablesMissing"` (code 1).

> Note: with `CGO_ENABLED=0` on Linux, a *runtime* `os.Setenv("OBOL_PRICING_DIR", …)` does **not**
> reach the dlopen'd library's `getenv` (Go makes raw syscalls and never links libc). Set the var
> **before** the process starts, or set it via libc `setenv` (the test suite does this in
> `pricing_env_test.go`). Inherited environment is fine everywhere.

## Ownership & safety contract

obol owns every string it returns through an out-parameter. This binding honors the contract in
`drain`: it copies the obol-owned C string into a Go `[]byte` (`cstr` reads up to the NUL),
**then** `defer`s `obol_string_free`, so the Rust-owned pointer never outlives the copy and is
never freed twice. A zero out-pointer is handled. `obol_version` returns a static C string and is
never freed. String/byte arguments are kept alive across the synchronous FFI call with
`runtime.KeepAlive`. The public API returns plain Go structs — you manage none of this yourself.

## Tests

```bash
mise exec rust@1.96.0 -- cargo build -p obol-ffi   # build the dylib first
cd bindings/go && CGO_ENABLED=0 go test ./...
```
