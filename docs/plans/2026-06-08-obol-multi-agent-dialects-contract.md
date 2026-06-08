# obol Contract Cleanup — Implementation Plan (Part 2 of 2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make `dialect` required and remove the bytes interface entirely — across the Rust core, the C FFI, and all four language bindings (Python, Go, TS-Bun, TS-Node) — then extend the cross-consumer parity check to the four new dialects.

**Architecture:** This is the breaking half of PRI-2114. The Rust core API becomes `estimate_cost(path: &Path, dialect: Dialect)` (no `Option`, no `Source::Bytes`); the FFI drops `obol_estimate_bytes` and rejects a NULL dialect; each binding drops its bytes wrapper and makes `dialect` required. The Rust bindings are separate Python/Go/TS packages (NOT in `cargo test --workspace`), so the Rust changes (Tasks 1–2) keep the workspace green; each binding (Tasks 3–5) is fixed and verified with its own test runner; Task 6 re-proves byte-identical `total_usd` across all five consumers for every dialect.

**Tech Stack:** Rust (obol-core, obol-cli, obol-ffi + cbindgen header), Python (ctypes), Go (purego), TypeScript (Bun `bun:ffi` + Node `koffi`), bash parity harness.

**Spec:** `docs/specs/2026-06-08-obol-multi-agent-dialects-design.md` §"Contract changes" · **Linear:** PRI-2114 · **Builds on** Part 1 (`2026-06-08-obol-multi-agent-dialects.md`, already landed on this branch).

**Conventions:** Rust tests run single-threaded: `cargo test --workspace -- --test-threads=1` (prefix `mise exec rust@1.96.0 --` if cargo isn't on PATH). Commit after each task; subject style `refactor(scope): … (PRI-2114)` (breaking-change commits). Each binding has its own test command (given per task).

---

## File Structure

**Modify (Rust):**
- `crates/obol-core/src/lib.rs` — remove `Source`; `estimate_cost(path, dialect)`; update `api_tests`.
- `crates/obol-cli/src/main.rs` — read file, detect-or-arg → `Dialect`, call `estimate_cost(&path, dialect)`.
- `crates/obol-ffi/src/lib.rs` — delete `obol_estimate_bytes`; `parse_dialect` NULL → error; update tests.
- `crates/obol-ffi/include/obol.h` — regenerated (drop the bytes declaration).

**Modify (bindings):**
- `bindings/python/obol/{__init__.py,_lib.py}`, `bindings/python/tests/test_obol.py`, `bindings/python/README.md`.
- `bindings/go/obol/{obol.go,loader.go,loader_unsupported.go,obol_test.go}`, `bindings/go/cmd/total/main.go`, `bindings/go/README.md`.
- `bindings/typescript/src/{index.ts,ffi.ts,ffi-bun.ts,ffi-node.ts,types.ts}`, `bindings/typescript/test/obol.test.ts`, `bindings/typescript/total.ts`, `bindings/typescript/README.md`.

**Modify (parity):**
- `scripts/validate_bindings.sh`, `bindings/testdata/` (add 4 dialect fixtures + extend `prices.json`).

---

## Task 1: Rust core — required dialect, drop `Source`

**Files:** Modify `crates/obol-core/src/lib.rs`, `crates/obol-cli/src/main.rs`.

- [ ] **Step 1: Update the core API.** In `crates/obol-core/src/lib.rs`, delete the `Source` enum and its doc comment, and replace `estimate_cost`:

```rust
/// Estimate the cost of a transcript file under the given dialect. Loads the active
/// price snapshot (bundled fallback) and prices the parsed usage.
pub fn estimate_cost(path: &Path, dialect: Dialect) -> Result<CostEstimate, ObolError> {
    let (store, source_kind) = resolve_store()?;
    let bytes = std::fs::read(path)?;
    let usages = transcript::parse(&bytes, dialect)?;
    Ok(cost::estimate(&usages, &store, source_kind))
}
```

Remove the now-unused `Source` re-export/usage. Keep `transcript::detect` public (the CLI uses it). `use std::path::Path;` is already imported (it was used by `Source`); keep it.

- [ ] **Step 2: Fix the core tests.** In `crates/obol-core/src/lib.rs` `api_tests`, every test currently calls `estimate_cost(Source::Bytes(include_bytes!(...)), Some(Dialect::X))` or `Source::Path(&p)`. Rewrite each to write the fixture to a temp file and pass the path + a bare `Dialect`. Concretely:
  - `estimate_cost_on_bytes_with_missing_tables_errors`, `estimate_cost_end_to_end_with_seeded_store`, `falls_back_to_embedded_when_no_local_snapshot`, `explicit_override_uses_local_source`, `kimi_model_surfaces_unpriced_loudly`: replace `Source::Bytes(include_bytes!("../tests/fixtures/<f>"))` with a temp file. Pattern to apply in each:

```rust
        let tmp = std::env::temp_dir().join(format!("obol-t-{}-{}", std::process::id(), line!()));
        std::fs::write(&tmp, include_bytes!("../tests/fixtures/claude-mini.jsonl").as_slice()).unwrap();
        let est = estimate_cost(&tmp, Dialect::Claude).unwrap();
        // ... assertions ...
        std::fs::remove_file(&tmp).ok();
```

  - `estimate_cost_from_path_with_autodetect` used `Source::Path(&transcript), None` (autodetect). Autodetect is no longer in `estimate_cost`. Rewrite it to detect first, then estimate: `let bytes = std::fs::read(&transcript).unwrap(); let d = transcript::detect(&bytes).unwrap(); let est = estimate_cost(&transcript, d).unwrap();` and assert `est.total_usd > 0.0`. Rename it to `estimate_cost_from_path_then_detect`.

- [ ] **Step 3: Update the CLI.** In `crates/obol-cli/src/main.rs`, the `Estimate` arm currently builds `hint: Option<Dialect>` and calls `estimate_cost(Source::Path(&path), hint)`. Replace the estimate flow so the CLI resolves a concrete `Dialect` (explicit `--dialect`, else `transcript::detect`):

```rust
            let dialect = match dialect.as_deref() {
                Some(d) => match d {
                    "claude" => Dialect::Claude,
                    "codex" => Dialect::Codex,
                    "pi" => Dialect::Pi,
                    "gemini" => Dialect::Gemini,
                    "opencode" => Dialect::Opencode,
                    "copilot" => Dialect::Copilot,
                    "kimi" => Dialect::Kimi,
                    other => unreachable!("clap value_parser restricts dialect; got {other:?}"),
                },
                None => {
                    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                    obol_core::transcript::detect(&bytes).map_err(|e| {
                        format!("{e}; pass --dialect to choose one explicitly")
                    })?
                }
            };
            let est = estimate_cost(&path, dialect).map_err(|e| e.to_string())?;
```

Update the `use obol_core::{…}` import: drop `Source`, ensure `Dialect`, `estimate_cost`, `PricingSource` are present. Confirm `obol_core::transcript` is reachable (it is — `transcript` is a public module; `detect` is public).

- [ ] **Step 4: Build + test.**

Run: `cargo test --workspace -- --test-threads=1`
Expected: PASS. Then `cargo clippy --workspace --all-targets -- -D warnings` clean.

- [ ] **Step 5: Smoke autodetect + explicit still work.**

Run: `env -u OBOL_PRICING_DIR ./target/debug/obol estimate crates/obol-core/tests/fixtures/gemini-mini.jsonl` (autodetect) and `... --dialect gemini`. Both print a total. (Build first: `cargo build -q -p obol-cli`.)

- [ ] **Step 6: Commit.**

```bash
git add crates/obol-core/src/lib.rs crates/obol-cli/src/main.rs
git commit -m "refactor(core): estimate_cost takes a required dialect; drop Source/bytes (PRI-2114)"
```

---

## Task 2: FFI — drop `obol_estimate_bytes`, reject NULL dialect, regen header

**Files:** Modify `crates/obol-ffi/src/lib.rs`, `crates/obol-ffi/include/obol.h`.

- [ ] **Step 1: Make `parse_dialect` require a dialect.** In `crates/obol-ffi/src/lib.rs`, change `parse_dialect` to return `Result<Dialect, ()>` and treat NULL as an error:

```rust
/// NULL/unknown/invalid-UTF-8 dialect -> Err(()). Known string -> Ok(Dialect).
fn parse_dialect(dialect: *const c_char) -> Result<Dialect, ()> {
    if dialect.is_null() {
        return Err(());
    }
    let s = unsafe { CStr::from_ptr(dialect) }.to_str().map_err(|_| ())?;
    match s {
        "claude" => Ok(Dialect::Claude),
        "codex" => Ok(Dialect::Codex),
        "pi" => Ok(Dialect::Pi),
        "gemini" => Ok(Dialect::Gemini),
        "opencode" => Ok(Dialect::Opencode),
        "copilot" => Ok(Dialect::Copilot),
        "kimi" => Ok(Dialect::Kimi),
        _ => Err(()),
    }
}
```

- [ ] **Step 2: Delete `obol_estimate_bytes`.** Remove the entire `#[no_mangle] pub extern "C" fn obol_estimate_bytes(...)` function from `crates/obol-ffi/src/lib.rs`.

- [ ] **Step 3: Update `obol_estimate_path`.** Its body maps the dialect and calls the core. Replace the dialect-handling + call so a missing/invalid dialect yields `ERR_INVALID_ARG`, and it calls the new path-only core API:

```rust
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
```

Update the top-of-file `use obol_core::{…}` to drop `Source` and keep `estimate_cost, Dialect, ObolError`.

- [ ] **Step 4: Fix the FFI tests.** In `crates/obol-ffi/src/lib.rs` tests:
  - Delete the bytes tests: `estimate_bytes_success_with_seeded_store`, `estimate_bytes_missing_tables_is_code_1`, `estimate_bytes_unknown_dialect_string_is_code_7`, `estimate_bytes_null_out_is_code_7`, `estimate_bytes_null_data_is_code_7`.
  - Replace them with path-based equivalents. The seeded-store success test writes the claude fixture to a temp file and calls `obol_estimate_path`:

```rust
    #[test]
    fn estimate_path_success_with_seeded_store() {
        let dir = seed_pricing();
        let f = dir.join("session.jsonl");
        std::fs::write(&f, include_bytes!("../tests/fixtures/claude-mini.jsonl").as_slice()).unwrap();
        let cpath = CString::new(f.to_str().unwrap()).unwrap();
        let claude = CString::new("claude").unwrap();
        let mut out = out_ptr();
        let code = obol_estimate_path(cpath.as_ptr(), claude.as_ptr(), &mut out);
        assert_eq!(code, OK, "code={code}");
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        obol_string_free(out);
        assert!(serde_json::from_str::<serde_json::Value>(&json).unwrap()["total_usd"].as_f64().unwrap() > 0.0);
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
```

  - Keep `estimate_path_bad_path_is_io_error` but add the now-required dialect arg: change its `obol_estimate_path(p.as_ptr(), std::ptr::null(), &mut out)` to pass a `CString::new("claude")` dialect pointer (so it reaches the IO error, not the dialect check).
  - The `maps_obol_errors_to_codes` test references `ObolError` variants — unchanged.

- [ ] **Step 5: Regenerate the header.** The `header_matches_source` test regenerates `obol.h` via cbindgen and asserts it equals the committed file. Run the project's generator (per that test's failure message):

```bash
./scripts/gen-header.sh
```

Confirm `crates/obol-ffi/include/obol.h` no longer declares `obol_estimate_bytes`.

- [ ] **Step 6: Build + test.**

Run: `cargo test --workspace -- --test-threads=1` → PASS (incl. `header_matches_source`). `cargo clippy --workspace --all-targets -- -D warnings` clean.

- [ ] **Step 7: Commit.**

```bash
git add crates/obol-ffi/src/lib.rs crates/obol-ffi/include/obol.h
git commit -m "refactor(ffi): drop obol_estimate_bytes; NULL dialect -> InvalidArgument (PRI-2114)"
```

---

## Task 3: Python binding — required dialect, remove bytes

**Files:** Modify `bindings/python/obol/__init__.py`, `bindings/python/obol/_lib.py`, `bindings/python/tests/test_obol.py`, `bindings/python/README.md`.
**Test command:** `cd bindings/python && PYTHONPATH=. python3 -m pytest -q` (after building the dylib — see Step 1).

- [ ] **Step 1: Rebuild the native lib.** The binding loads the FFI cdylib. Rebuild it so the bytes symbol is gone: `cargo build -p obol-ffi --release` (the loader resolves the built dylib; if the binding expects a specific path, follow its existing `_lib.py` discovery — do not change discovery logic).

- [ ] **Step 2: Remove the bytes ctypes declaration.** In `bindings/python/obol/_lib.py`, delete the three lines declaring `_lib.obol_estimate_bytes.restype` / `.argtypes` (the `obol_estimate_bytes` block). Leave the `obol_estimate_path` declaration intact.

- [ ] **Step 3: Required dialect + drop `estimate_bytes`.** In `bindings/python/obol/__init__.py`:
  - Remove `"estimate_bytes"` from `__all__`.
  - Delete the entire `def estimate_bytes(...)` function.
  - Change `estimate_path` to require `dialect`:

```python
def estimate_path(path, dialect: str) -> CostEstimate:
    code, payload = _lib.call(_lib._lib.obol_estimate_path, str(path).encode(), dialect.encode())
    return _estimate_result(code, payload)
```

  (Drop `Optional` from the signature and the `d = dialect.encode() if dialect else None` conditional. If `Optional` is now unused, remove its import.)

- [ ] **Step 4: Fix tests.** In `bindings/python/tests/test_obol.py`:
  - Delete `test_estimate_bytes_autodetect`.
  - Convert `test_missing_tables_raises` and `test_unknown_dialect_raises` to use `estimate_path` against the fixture file instead of `estimate_bytes`:

```python
def test_missing_tables_raises(monkeypatch):
    import obol
    monkeypatch.setenv("OBOL_PRICING_DIR", "/nonexistent/obol-py-xyz")
    with pytest.raises(obol.ObolError) as ei:
        obol.estimate_path(TESTDATA / "claude-mini.jsonl", dialect="claude")
    assert ei.value.code == 1
    assert ei.value.kind == "PricingTablesMissing"

def test_unknown_dialect_raises(seeded):
    import obol
    with pytest.raises(obol.ObolError) as ei:
        obol.estimate_path(TESTDATA / "claude-mini.jsonl", dialect="banana")
    assert ei.value.code == 7
```

  Keep `test_estimate_path_matches_expectations` (already passes an explicit dialect).

- [ ] **Step 5: README.** In `bindings/python/README.md`, remove the `# dialect=None auto-detects` comment and the "Or from in-memory bytes" block + `estimate_bytes` example; state that `dialect` is required.

- [ ] **Step 6: Test.**

Run: `cd bindings/python && PYTHONPATH=. python3 -m pytest -q`
Expected: PASS (no `estimate_bytes` references remain; `python3 -c "import obol; print(hasattr(obol,'estimate_bytes'))"` prints `False`).

- [ ] **Step 7: Commit.**

```bash
git add bindings/python
git commit -m "refactor(python): required dialect; remove estimate_bytes (PRI-2114)"
```

---

## Task 4: Go binding — required dialect, remove bytes

**Files:** Modify `bindings/go/obol/obol.go`, `bindings/go/obol/loader.go`, `bindings/go/obol/loader_unsupported.go`, `bindings/go/obol/obol_test.go`, `bindings/go/cmd/total/main.go`, `bindings/go/README.md`.
**Test command:** `cd bindings/go && CGO_ENABLED=0 go test ./...` (after `cargo build -p obol-ffi --release`).

- [ ] **Step 1: Remove the bytes func + symbol.**
  - In `bindings/go/obol/obol.go`, delete the entire `func EstimateBytes(...)` (and its doc comment). Update the `EstimatePath` doc comment to drop the "dialect \"\" means auto-detect" clause.
  - In `bindings/go/obol/loader.go`, delete the `cEstimateBytes func(...)` var (in the `var (...)` block) and the `purego.RegisterLibFunc(&cEstimateBytes, h, "obol_estimate_bytes")` line.
  - In `bindings/go/obol/loader_unsupported.go`, delete the `cEstimateBytes func(...)` var line.

- [ ] **Step 2: Enforce required dialect.** In `bindings/go/obol/obol.go` `EstimatePath`, reject empty dialect at the Go layer with a clear error before calling the FFI:

```go
func EstimatePath(path, dialect string) (*CostEstimate, error) {
	if err := ensureLoaded(); err != nil {
		return nil, err
	}
	if dialect == "" {
		return nil, &ObolError{Code: 7, Kind: "InvalidArgument", Message: "dialect is required"}
	}
	p := append([]byte(path), 0)
	d := append([]byte(dialect), 0)
	var out uintptr
	code := cEstimatePath(&p[0], &d[0], &out)
	runtime.KeepAlive(p)
	runtime.KeepAlive(d)
	return decodeEstimate(code, drain(out))
}
```

(Confirm the `ObolError` struct fields are `Code`, `Kind`, `Message` — adjust the literal to match. If `dialectBytes`/`bytePtr` are now unused, remove them; if still used elsewhere, leave them.)

- [ ] **Step 3: cmd/total requires the dialect.** In `bindings/go/cmd/total/main.go`, make the dialect arg mandatory:

```go
	if len(os.Args) < 3 {
		fmt.Fprintln(os.Stderr, "usage: total <transcript> <dialect>")
		os.Exit(2)
	}
	est, err := obol.EstimatePath(os.Args[1], os.Args[2])
```

- [ ] **Step 4: Fix tests.** In `bindings/go/obol/obol_test.go`, change `TestMissingTablesIsError` and `TestUnknownDialectIsError` from `EstimateBytes(data, "...")` to `EstimatePath(filepath.Join(testdata(t), "claude-mini.jsonl"), "...")` (drop the `os.ReadFile` lines). Same expected codes (1 and 7).

- [ ] **Step 5: README.** In `bindings/go/README.md`, remove the `EstimateBytes` line and the `// "" dialect auto-detects` comment; show `EstimatePath("transcript.jsonl", "claude")` with dialect required.

- [ ] **Step 6: Test.**

Run: `cd bindings/go && CGO_ENABLED=0 go test ./...`
Expected: PASS. Also `go vet ./...` clean.

- [ ] **Step 7: Commit.**

```bash
git add bindings/go
git commit -m "refactor(go): required dialect; remove EstimateBytes (PRI-2114)"
```

---

## Task 5: TypeScript binding — required dialect, remove bytes, new dialect types

**Files:** Modify `bindings/typescript/src/{index.ts,ffi.ts,ffi-bun.ts,ffi-node.ts,types.ts}`, `bindings/typescript/test/obol.test.ts`, `bindings/typescript/total.ts`, `bindings/typescript/README.md`.
**Test command:** `package.json` has no `scripts`; run the unit tests under both runtimes directly: `cd bindings/typescript && bun test test/obol.test.ts` and `node --test test/obol.test.ts`. (Node runs `.ts` directly here — see how `scripts/validate_bindings.sh` invokes `node total.ts`; match that invocation if `node --test` needs a TS loader flag.) Build the dylib first: `cargo build -p obol-ffi --release`.

- [ ] **Step 1: Dialect type — required + the four new values.** In `bindings/typescript/src/types.ts`, change:

```typescript
export type Dialect = "claude" | "codex" | "pi" | "gemini" | "opencode" | "copilot" | "kimi";
```

- [ ] **Step 2: Public API — drop bytes, require dialect.** In `bindings/typescript/src/index.ts`:

```typescript
export async function estimatePath(path: string, dialect: Dialect): Promise<CostEstimate> {
  return unwrap<CostEstimate>((await backend()).estimatePath(path, dialect));
}
```

Delete the `estimateBytes` export entirely.

- [ ] **Step 3: Backend interface.** In `bindings/typescript/src/ffi.ts`, change the interface:

```typescript
export interface FfiBackend {
  version(): string;
  estimatePath(path: string, dialect: string): RawResult;
  refresh(asOf: string): RawResult;
}
```

Delete the `estimateBytes(...)` member.

- [ ] **Step 4: Bun backend.** In `bindings/typescript/src/ffi-bun.ts`: delete the `obol_estimate_bytes` line from the `dlopen` symbols object, and delete the `estimateBytes(...) { ... }` method. Keep `estimatePath`. If the `nonEmpty` helper is now unused, remove it.

- [ ] **Step 5: Node backend.** In `bindings/typescript/src/ffi-node.ts`: delete the `const obol_estimate_bytes = lib.func(...)` declaration and the `estimateBytes(...) { ... }` method. Keep `estimatePath`. Remove `nonEmpty` if unused.

- [ ] **Step 6: Fix tests.** In `bindings/typescript/test/obol.test.ts`:
  - Delete the `"estimateBytes autodetect"` test.
  - Convert the `"missing tables -> ObolError code 1"` and `"unknown dialect -> ObolError code 7"` tests from `estimateBytes(data, ...)` to `estimatePath(TRANSCRIPT, ...)` (drop the `readFileSync` lines; `TRANSCRIPT` is the fixture path already in the file). Keep the same expected codes/kinds.

- [ ] **Step 7: total.ts requires dialect.** In `bindings/typescript/total.ts`:

```typescript
import { estimatePath, type Dialect } from "./src/index.ts";

const path = process.argv[2];
const dialect = process.argv[3] as Dialect | undefined;
if (!path || !dialect) {
  console.error("usage: total <transcript> <dialect>");
  process.exit(2);
}
const est = await estimatePath(path, dialect);
console.log(est.total_usd);
```

- [ ] **Step 8: README.** In `bindings/typescript/README.md`, remove the `estimateBytes` example and the "dialect optional → auto-detect" comment; show `estimatePath("session.jsonl", "claude")` with dialect required.

- [ ] **Step 9: Test (both runtimes).**

Run: `cd bindings/typescript && bun test` and `node --test` (or the configured scripts).
Expected: PASS under both Bun and Node.

- [ ] **Step 10: Commit.**

```bash
git add bindings/typescript
git commit -m "refactor(ts): required dialect; remove estimateBytes; add new dialect types (PRI-2114)"
```

---

## Task 6: Cross-consumer parity for the four new dialects

**Files:** Modify `scripts/validate_bindings.sh`; add fixtures under `bindings/testdata/`.
**Run command:** `bash scripts/validate_bindings.sh`.

- [ ] **Step 1: Add dialect fixtures to the parity corpus.** Copy the four core fixtures into the parity testdata so all five consumers read the same bytes:

```bash
cp crates/obol-core/tests/fixtures/gemini-mini.jsonl   bindings/testdata/gemini-mini.jsonl
cp crates/obol-core/tests/fixtures/opencode-mini.json  bindings/testdata/opencode-mini.json
cp crates/obol-core/tests/fixtures/copilot-mini.jsonl  bindings/testdata/copilot-mini.jsonl
cp crates/obol-core/tests/fixtures/kimi-mini.jsonl     bindings/testdata/kimi-mini.jsonl
```

- [ ] **Step 2: Extend the parity price sheet** so gemini/opencode/copilot price to nonzero (kimi stays unpriced → 0, still a valid parity check). Edit `bindings/testdata/prices.json` to add three models to the `litellm` namespace (rates copied from `crates/obol-core/prices/bundled.json` for exactness):

```json
{"as_of":"2026-06-05","namespaces":{"litellm":{
  "claude-opus-4-8":{"input":5.0,"output":25.0,"cache_read":0.5,"cache_write":6.25},
  "claude-sonnet-4-5":{"input":3.0,"output":15.0,"cache_read":0.3,"cache_write":3.75},
  "gemini-3-flash-preview":{"input":0.5,"output":2.5,"cache_read":0.05,"cache_write":0.0},
  "gpt-5.5":{"input":5.0,"output":30.0,"cache_read":0.5,"cache_write":0.0}
}}}
```

(If the real bundled rates differ, use those — the only requirement is that all five consumers read this same file, so values just need to be internally consistent. Pull the exact `claude-sonnet-4-5`, `gemini-3-flash-preview`, `gpt-5.5` entries from `bundled.json` if you want production-accurate numbers.)

- [ ] **Step 3: Parameterize the harness over dialects.** In `scripts/validate_bindings.sh`, factor the five-consumer comparison into a function `check <transcript> <dialect>` that runs all five consumers and asserts equality (reuse the existing `norm` helper and the existing per-consumer command lines, substituting `$T`→`$1` and `claude`→`$2`). Then call it once per dialect:

```bash
check bindings/testdata/claude-mini.jsonl   claude
check bindings/testdata/gemini-mini.jsonl   gemini
check bindings/testdata/opencode-mini.json  opencode
check bindings/testdata/copilot-mini.jsonl  copilot
check bindings/testdata/kimi-mini.jsonl     kimi
```

`check` must `exit 1` on any mismatch and echo `OK: <dialect> rust==python==go==ts(bun)==ts(node) (<value>)` on success. (The existing single-dialect assertion block is the body of `check`.)

- [ ] **Step 4: Run the parity check.**

Run: `bash scripts/validate_bindings.sh`
Expected: five `OK: <dialect> …` lines (claude, gemini, opencode, copilot, kimi), exit 0. kimi prints `0.0` from all five (unpriced parity).

- [ ] **Step 5: Commit.**

```bash
git add scripts/validate_bindings.sh bindings/testdata
git commit -m "test(parity): byte-identical total_usd across 5 consumers for all dialects (PRI-2114)"
```

---

## Self-review notes (coverage map)

- Spec §"Dialect is required; no library-side auto-detect" → Task 1 (core `estimate_cost(path, dialect)`, CLI detect-or-arg) + Task 2 (FFI NULL → InvalidArgument).
- Spec §"No bytes interface" → Task 1 (drop `Source`/`Source::Bytes`), Task 2 (drop `obol_estimate_bytes` + header), Tasks 3–5 (drop each binding's bytes wrapper).
- Spec §"Bindings drop their bytes wrapper + require dialect" → Tasks 3 (Python), 4 (Go), 5 (TS-Bun+Node, + new dialect types).
- Spec §"Cross-consumer parity covers the new dialects" → Task 6.
- `obol.h` regen guarded by the existing `header_matches_source` test → Task 2 Step 5.

**Verification gate before finishing:** `cargo test --workspace -- --test-threads=1`, each binding's own test command (Tasks 3–5), and `bash scripts/validate_bindings.sh` (Task 6) all green.

## Notes for the executor

- The Rust bindings are separate packages, so Tasks 1–2 keep `cargo test --workspace` green; the binding test runners are the gate for Tasks 3–5.
- Each binding loads the freshly built `obol-ffi` cdylib — run `cargo build -p obol-ffi --release` before a binding's tests so the removed `obol_estimate_bytes` symbol is actually gone from the loaded library.
- Do not regenerate `obol.h` by hand — use `./scripts/gen-header.sh` (Task 2) so it matches what `header_matches_source` expects.
