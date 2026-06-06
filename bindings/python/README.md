# obol — Python binding (ctypes)

> **Installing from PyPI:** `pip install primeradianthq-obol`, then `import obol`. The native
> library is bundled in the wheel — no Rust toolchain, no `cargo build`, no `OBOL_LIB`. The
> sections below are for in-repo development.

A thin, pure-Python binding over obol's C ABI (`obol-ffi`). No build step: it loads the
prebuilt shared library with `ctypes` and re-types the JSON the Rust core returns. The Rust
core stays the single source of truth for all accounting; this package only marshals C
strings and parses JSON into dataclasses.

## Install / point at the library

The binding needs the `obol-ffi` shared library (`libobol_ffi.dylib` on macOS,
`libobol_ffi.so` on Linux). It is located, in order:

1. `$OBOL_LIB` — an explicit absolute path to the shared library.
2. Beside the installed `obol` package (for packaged installs).
3. `target/release/` then `target/debug/` relative to the repo root (for in-tree dev).

For in-tree development, just build the dylib and the `target/debug` fallback finds it with
no env:

```bash
mise exec rust@1.96.0 -- cargo build -p obol-ffi
```

If the library lives somewhere else, set `OBOL_LIB=/abs/path/to/libobol_ffi.dylib`.

## Usage

```python
import obol

print(obol.version())  # "0.1.1"

est = obol.estimate_path("transcript.jsonl", dialect="claude")  # dialect=None auto-detects
print(est.total_usd, est.pricing_as_of)
for m in est.per_model:
    print(m.model, m.provider, m.subtotal_usd)

# Or from in-memory bytes:
est = obol.estimate_bytes(open("transcript.jsonl", "rb").read())

# Refresh the on-disk pricing snapshot (network; caller supplies the date):
report = obol.refresh("2026-06-05")
```

On a nonzero status, the call raises `obol.ObolError` carrying `.code`, `.kind`, and
`.message` from the FFI error envelope.

## Pricing tables must exist

`estimate_*` reads a pricing snapshot from disk. Either run `obol refresh` (the CLI), or
point `OBOL_PRICING_DIR` at a directory containing `current.json`. With no snapshot the call
raises `ObolError` with `kind == "PricingTablesMissing"` (code 1).

## Ownership & safety contract

obol owns every string it returns through an out-parameter. This binding honors the contract
automatically in `_lib._decode_and_free`: it copies the obol-owned C string into Python
`bytes`, **then** calls `obol_string_free`, **then** the caller parses the copy. The
Rust-owned pointer never outlives the copy and is never freed twice. `obol_version()` returns
a static string and is never freed. You do not manage any of this yourself — the public API
returns plain dataclasses.

## Tests

```bash
mise exec rust@1.96.0 -- cargo build -p obol-ffi   # build the dylib first
cd bindings/python && PYTHONPATH=. python -m pytest tests -q
```
