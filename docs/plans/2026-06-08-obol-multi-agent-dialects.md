# obol Multi-Agent Dialects + Pricing Ergonomics — Implementation Plan (Part 1 of 2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `gemini`, `opencode`, `copilot`, and `kimi` transcript dialects to obol, and make pricing work out of the box (bundled snapshot, provenance in output, newer/local precedence).

**Architecture:** Each dialect is a new `transcript/<name>.rs` module exposing `parse(&[u8]) -> Result<Vec<MessageUsage>, ObolError>`, wired into the `Dialect` enum, the `parse()` dispatch, and a best-effort `detect()`. Pricing gains an embedded snapshot used as a fallback, plus a `pricing_source` field and a max-by-`as_of` resolution rule. This plan is **purely additive** — it keeps the existing `estimate_cost(Source, Option<Dialect>)` API and the `obol_estimate_bytes` FFI intact so the workspace stays green throughout. The breaking contract cleanup (required dialect, drop the bytes interface) + binding propagation is **Part 2**, a separate plan, done atomically because it breaks all bindings the moment it lands.

**Tech Stack:** Rust (obol-core, obol-cli, obol-ffi), `serde_json`. No new dependencies — opencode reads the `opencode export` JSON, not the SQLite db (avoids a libsqlite3 C dep that would complicate the platform-wheel packaging).

**Spec:** `docs/specs/2026-06-08-obol-multi-agent-dialects-design.md` · **Linear:** PRI-2114

**Conventions:** Build/test with `cargo` (prefix `mise exec rust@1.96.0 --` if cargo isn't on PATH). The suite sets a process-global env var, so **always run tests single-threaded**: `cargo test --workspace -- --test-threads=1`. Commit after each task; reference `PRI-2114` in the subject (repo style: `feat(scope): … (PRI-2114)`).

**Ordering note:** Tasks 5–8 add each dialect to obol-core only (module + `Dialect` variant + `parse()` arm + `detect()` + fixture + tests). They do **not** touch the CLI/FFI dialect strings — adding a `Dialect` variant compiles fine while the string→`Dialect` maps keep their existing fallbacks. All CLI/FFI string wiring lands once, exhaustively, in Task 9 (which also fixes the CLI's `_ => Dialect::Pi` catch-all).

---

## File Structure

**Create:**
- `crates/obol-core/prices/bundled.json` — committed price snapshot embedded into the library.
- `crates/obol-core/src/transcript/{gemini,opencode,copilot,kimi}.rs` — the four parsers.
- `crates/obol-core/tests/fixtures/gemini-mini.jsonl` — real-derived fixture.
- `crates/obol-core/tests/fixtures/opencode-mini.json` — real-derived fixture.
- `crates/obol-core/tests/fixtures/copilot-mini.jsonl` — synthetic fixture.
- `crates/obol-core/tests/fixtures/kimi-mini.jsonl` — synthetic fixture.

**Modify:**
- `crates/obol-core/src/model.rs` — `PricingSource` enum + `CostEstimate.pricing_source`.
- `crates/obol-core/src/pricing/store.rs` — `embedded()` loader.
- `crates/obol-core/src/pricing/mod.rs` — re-export `embedded`.
- `crates/obol-core/src/cost.rs` — thread `pricing_source` into `estimate`.
- `crates/obol-core/src/lib.rs` — snapshot resolution (env override / max-by-`as_of` / embedded fallback) + the kimi integration test.
- `crates/obol-core/src/transcript/mod.rs` — `Dialect` variants, `parse()` dispatch, `detect()` heuristics + single-doc fallback.
- `crates/obol-cli/src/main.rs` — `--dialect` value set + exhaustive match + source in output.
- `crates/obol-cli/tests/cli.rs` — pricing-source + new-dialect CLI tests.
- `crates/obol-ffi/src/lib.rs` — `parse_dialect` gains the four strings.

---

## Phase 1 — Pricing ergonomics (works out of the box)

### Task 1: Bundled price snapshot + `pricing::embedded()`

**Files:**
- Create: `crates/obol-core/prices/bundled.json`
- Modify: `crates/obol-core/src/pricing/store.rs`, `crates/obol-core/src/pricing/mod.rs`

- [ ] **Step 1: Commit the snapshot asset**

The live snapshot is gitignored (`.pricing/current.json`) and outside the crate, so copy it into the crate where `include_bytes!` and `cargo publish` can see it:

```bash
mkdir -p crates/obol-core/prices
cp .pricing/current.json crates/obol-core/prices/bundled.json
```

- [ ] **Step 2: Write the failing test**

Add to the `tests` module in `crates/obol-core/src/pricing/store.rs`:

```rust
    #[test]
    fn embedded_snapshot_loads_and_has_models() {
        let s = embedded().expect("embedded snapshot parses");
        assert!(!s.as_of.is_empty(), "embedded snapshot must carry an as_of");
        assert!(
            s.lookup("litellm", "claude-opus-4-8").is_some(),
            "embedded snapshot should price a known model"
        );
    }
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p obol-core embedded_snapshot_loads_and_has_models -- --test-threads=1`
Expected: FAIL — `cannot find function 'embedded'`.

- [ ] **Step 4: Add the loader**

In `crates/obol-core/src/pricing/store.rs`, after the `current_path` function:

```rust
/// The price snapshot compiled into the library — the out-of-the-box fallback used
/// when no on-disk snapshot is newer (see `lib::estimate_cost`).
pub fn embedded() -> Result<PriceStore, ObolError> {
    const BYTES: &[u8] = include_bytes!("../../prices/bundled.json");
    PriceStore::from_json(BYTES)
}
```

In `crates/obol-core/src/pricing/mod.rs`, add `embedded` to the existing re-export (line 5):

```rust
pub use store::{current_path, embedded, pricing_dir, PriceStore};
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p obol-core embedded_snapshot_loads_and_has_models -- --test-threads=1`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/obol-core/prices/bundled.json crates/obol-core/src/pricing/store.rs crates/obol-core/src/pricing/mod.rs
git commit -m "feat(pricing): embed a price snapshot for out-of-the-box use (PRI-2114)"
```

---

### Task 2: `PricingSource` + `CostEstimate.pricing_source`

**Files:**
- Modify: `crates/obol-core/src/model.rs`, `crates/obol-core/src/cost.rs`, `crates/obol-core/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/obol-core/src/model.rs`:

```rust
    #[test]
    fn pricing_source_serializes_lowercase() {
        assert_eq!(
            serde_json::to_value(PricingSource::Bundled).unwrap(),
            serde_json::json!("bundled")
        );
        assert_eq!(
            serde_json::to_value(PricingSource::Local).unwrap(),
            serde_json::json!("local")
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p obol-core pricing_source_serializes_lowercase -- --test-threads=1`
Expected: FAIL — `cannot find type 'PricingSource'`.

- [ ] **Step 3: Add the enum and field**

In `crates/obol-core/src/model.rs`, add near `Approximation`:

```rust
/// Which snapshot priced this estimate: the one compiled into the library, or one
/// read from disk (`refresh`ed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PricingSource {
    Bundled,
    Local,
}

impl serde::Serialize for PricingSource {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            PricingSource::Bundled => "bundled",
            PricingSource::Local => "local",
        })
    }
}
```

Add the field to `CostEstimate`, right after `pricing_as_of`:

```rust
    pub pricing_as_of: String,
    pub pricing_source: PricingSource,
}
```

Then update the existing `model.rs` test `cost_estimate_serializes_with_expected_fields`: add `pricing_source: PricingSource::Bundled,` to the `CostEstimate { … }` literal and add `assert_eq!(v["pricing_source"], "bundled");`.

- [ ] **Step 4: Set the field in `cost::estimate` (temporary default)**

In `crates/obol-core/src/cost.rs`, add `PricingSource` to the model import and set the new field. **Keep the 2-arg signature for now** (a hardcoded `Bundled`) so the whole crate still compiles and every existing caller/test stays green; Task 3 turns it into a real parameter:

```rust
use crate::model::{Approximation, CostEstimate, MessageUsage, ModelCost, PricingSource, Provider, TokenBuckets};
```

```rust
        pricing_as_of: store.as_of.clone(),
        pricing_source: PricingSource::Bundled, // real value threaded in Task 3
    }
```

- [ ] **Step 5: Run the core suite**

Run: `cargo test -p obol-core -- --test-threads=1`
Expected: PASS (crate compiles; new `pricing_source` serialization test green; existing tests unaffected).

- [ ] **Step 6: Commit**

```bash
git add crates/obol-core/src/model.rs crates/obol-core/src/cost.rs
git commit -m "feat(cost): add pricing_source to CostEstimate (PRI-2114)"
```

---

### Task 3: Snapshot resolution (newer/local precedence + embedded fallback)

**Files:**
- Modify: `crates/obol-core/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `api_tests` module in `crates/obol-core/src/lib.rs`:

```rust
    #[test]
    fn falls_back_to_embedded_when_no_local_snapshot() {
        std::env::remove_var("OBOL_PRICING_DIR");
        let est = estimate_cost(
            Source::Bytes(include_bytes!("../tests/fixtures/claude-mini.jsonl")),
            Some(Dialect::Claude),
        )
        .unwrap();
        assert_eq!(est.pricing_source, crate::model::PricingSource::Bundled);
        assert!(est.total_usd > 0.0, "embedded snapshot should price claude");
    }

    #[test]
    fn explicit_override_uses_local_source() {
        let dir = std::env::temp_dir().join(format!("obol-resolve-{}", std::process::id()));
        std::env::set_var("OBOL_PRICING_DIR", &dir);
        let store = pricing::refresh::normalize_litellm(
            include_bytes!("../tests/fixtures/litellm-sample.json"),
            "2099-01-01",
        )
        .unwrap();
        store.save(&pricing::current_path()).unwrap();
        let est = estimate_cost(
            Source::Bytes(include_bytes!("../tests/fixtures/claude-mini.jsonl")),
            Some(Dialect::Claude),
        )
        .unwrap();
        assert_eq!(est.pricing_source, crate::model::PricingSource::Local);
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("OBOL_PRICING_DIR");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p obol-core falls_back_to_embedded explicit_override_uses_local -- --test-threads=1`
Expected: FAIL to compile/run — `estimate_cost` still loads only the on-disk store and `cost::estimate` now needs a third arg.

- [ ] **Step 3: Implement resolution**

First make `cost::estimate` take the source as a real parameter. In `crates/obol-core/src/cost.rs`, change the signature and use it:

```rust
pub fn estimate(usages: &[MessageUsage], store: &PriceStore, source: PricingSource) -> CostEstimate {
```

```rust
        pricing_as_of: store.as_of.clone(),
        pricing_source: source,
    }
```

Update the two `cost.rs` tests that call `estimate(...)` (`prices_known_model_exactly`, `unpriced_model_surfaces_not_silently_zero`) to pass `PricingSource::Bundled` as the third argument.

Then in `crates/obol-core/src/lib.rs`, add the import and a private resolver, and update `estimate_cost`:

```rust
use crate::model::PricingSource;

/// Resolve the price snapshot. Explicit OBOL_PRICING_DIR wins absolutely; otherwise
/// pick whichever of {on-disk current.json, embedded} has the newer `as_of`, on-disk
/// winning ties; embedded is the floor.
fn resolve_store() -> Result<(pricing::PriceStore, PricingSource), ObolError> {
    if std::env::var_os("OBOL_PRICING_DIR").is_some() {
        let store = pricing::PriceStore::load(&pricing::current_path())?;
        return Ok((store, PricingSource::Local));
    }
    let embedded = pricing::embedded()?;
    let local_path = pricing::current_path();
    if local_path.exists() {
        if let Ok(local) = pricing::PriceStore::load(&local_path) {
            if local.as_of >= embedded.as_of {
                return Ok((local, PricingSource::Local));
            }
        }
    }
    Ok((embedded, PricingSource::Bundled))
}
```

```rust
pub fn estimate_cost(source: Source, dialect: Option<Dialect>) -> Result<CostEstimate, ObolError> {
    let (store, source_kind) = resolve_store()?;
    let bytes: Vec<u8> = match source {
        Source::Path(p) => std::fs::read(p)?,
        Source::Bytes(b) => b.to_vec(),
    };
    let dialect = match dialect {
        Some(d) => d,
        None => transcript::detect(&bytes)?,
    };
    let usages = transcript::parse(&bytes, dialect)?;
    Ok(cost::estimate(&usages, &store, source_kind))
}
```

The existing `api_tests` test `estimate_cost_on_bytes_with_missing_tables_errors` set `OBOL_PRICING_DIR` to a nonexistent dir and expects `PricingTablesMissing`; that contract is preserved (explicit override, missing file → error). Leave it unchanged.

- [ ] **Step 4: Run the full core suite**

Run: `cargo test -p obol-core -- --test-threads=1`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/obol-core/src/lib.rs
git commit -m "feat(pricing): bundled-fallback + newer/local snapshot precedence (PRI-2114)"
```

---

### Task 4: CLI surfaces the pricing source

**Files:**
- Modify: `crates/obol-cli/src/main.rs`
- Test: `crates/obol-cli/tests/cli.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/obol-cli/tests/cli.rs` (the file already imports `assert_cmd::Command`, `std::fs`, and uses `predicates::str::contains`):

```rust
#[test]
fn estimate_reports_bundled_pricing_source() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = include_str!("../../obol-core/tests/fixtures/claude-mini.jsonl");
    let transcript = tmp.path().join("session.jsonl");
    fs::write(&transcript, claude).unwrap();

    // No OBOL_PRICING_DIR -> the embedded snapshot prices it; source must be "bundled".
    Command::cargo_bin("obol")
        .unwrap()
        .env_remove("OBOL_PRICING_DIR")
        .args([
            "estimate",
            transcript.to_str().unwrap(),
            "--dialect",
            "claude",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("pricing_source"))
        .stdout(predicates::str::contains("bundled"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p obol-cli estimate_reports_bundled_pricing_source -- --test-threads=1`
Expected: FAIL — output has no `pricing_source` field yet (the human path also lacks the source label).

- [ ] **Step 3: Add the source to the human-readable print**

In `crates/obol-cli/src/main.rs`, add `PricingSource` to the `use obol_core::{…}` import, and change the summary line:

```rust
            } else {
                let src = match est.pricing_source {
                    PricingSource::Bundled => "bundled",
                    PricingSource::Local => "local",
                };
                println!(
                    "total: ${:.4}  (pricing as of {}, {})",
                    est.total_usd, est.pricing_as_of, src
                );
```

(The `--json` branch already serializes the whole `CostEstimate`, so `pricing_source` appears there automatically.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p obol-cli estimate_reports_bundled_pricing_source -- --test-threads=1`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/obol-cli/src/main.rs crates/obol-cli/tests/cli.rs
git commit -m "feat(cli): show pricing source (bundled/local) in estimate output (PRI-2114)"
```

---

## Phase 2 — The four parsers (obol-core only)

Each task adds a parser module, a `Dialect` variant, a `parse()` dispatch arm, a `detect()` heuristic, a fixture, and tests. No CLI/FFI changes here (Task 9).

### Task 5: Gemini dialect

**Files:**
- Create: `crates/obol-core/src/transcript/gemini.rs`, `crates/obol-core/tests/fixtures/gemini-mini.jsonl`
- Modify: `crates/obol-core/src/transcript/mod.rs`

- [ ] **Step 1: Create the fixture (real-derived)**

`crates/obol-core/tests/fixtures/gemini-mini.jsonl` (the 2nd `$set` is the authoritative snapshot; the 3rd line is a no-op tail proving "latest with messages" wins):

```
{"sessionId":"329cdf31","projectHash":"abc","startTime":"2026-06-08T18:46:47.000Z","kind":"main"}
{"$set":{"messages":[{"id":"m0","type":"user","content":[{"text":"Hello"}]},{"type":"gemini","model":"gemini-3-flash-preview","tokens":{"input":9431,"output":12,"cached":0,"thoughts":94,"tool":0,"total":9537}}],"lastUpdated":"2026-06-08T18:46:50.000Z"}}
{"$set":{"lastUpdated":"2026-06-08T18:46:51.000Z"}}
```

- [ ] **Step 2: Write the failing test**

Create `crates/obol-core/src/transcript/gemini.rs` with a test module only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_latest_snapshot_and_folds_thoughts_into_output() {
        let u = parse(include_bytes!("../../tests/fixtures/gemini-mini.jsonl")).unwrap();
        assert_eq!(u.len(), 1, "{u:?}");
        assert_eq!(u[0].model, "gemini-3-flash-preview");
        assert_eq!(u[0].input_uncached, 9431);
        assert_eq!(u[0].cache_read, 0);
        assert_eq!(u[0].output, 106); // 12 output + 94 thoughts
        assert_eq!(u[0].request_input_tokens, 9431);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p obol-core --lib gemini -- --test-threads=1`
Expected: FAIL — `parse` not found / module not declared.

- [ ] **Step 4: Implement the parser**

Prepend to `crates/obol-core/src/transcript/gemini.rs`:

```rust
//! Gemini CLI chat transcript -> Vec<MessageUsage>.
//! Reconciled with AgentsView internal/parser/gemini.go (MIT, © 2026 Kenn Software LLC).
//! On-disk form is a `$set`-mutation JSONL: each `$set.messages` is a full snapshot of
//! the conversation; the latest one wins. Usage lives on `type:"gemini"` messages;
//! `tokens.thoughts` (thinking) is billed as output.

use crate::error::ObolError;
use crate::model::{MessageUsage, Provider};
use serde_json::Value;

pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ObolError::MalformedTranscript {
        line: 0,
        msg: e.to_string(),
    })?;
    // Latest non-empty messages snapshot (`$set.messages`, or a bare top-level
    // `messages` for single-doc safety). Last write wins.
    let mut latest: Option<Value> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let msgs = v.pointer("/$set/messages").or_else(|| v.get("messages"));
        if let Some(m) = msgs {
            if m.as_array().is_some_and(|a| !a.is_empty()) {
                latest = Some(m.clone());
            }
        }
    }
    let mut out = Vec::new();
    let msgs = match latest {
        Some(Value::Array(a)) => a,
        _ => return Ok(out),
    };
    for msg in &msgs {
        if msg.get("type").and_then(Value::as_str) != Some("gemini") {
            continue;
        }
        let tok = match msg.get("tokens") {
            Some(t) if t.is_object() => t,
            _ => continue,
        };
        let g = |k: &str| tok.get(k).and_then(Value::as_u64).unwrap_or(0);
        let input = g("input");
        let cached = g("cached");
        let output = g("output") + g("thoughts");
        out.push(MessageUsage {
            model: msg
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            provider: Provider::Other("google".into()),
            namespace: "litellm".into(),
            input_uncached: input,
            cache_read: cached,
            cache_write_5m: 0,
            cache_write_1h: 0,
            output,
            request_input_tokens: input + cached,
            service_tier: None,
        });
    }
    Ok(out)
}
```

- [ ] **Step 5: Wire into the dialect registry**

In `crates/obol-core/src/transcript/mod.rs`: add `pub mod gemini;` at the top; add `Gemini,` to the `Dialect` enum; add `Dialect::Gemini => gemini::parse(bytes),` to the `parse()` match; and add a detection heuristic inside the `detect()` line loop (before the generic claude `matches!` check):

```rust
        if v.get("type").and_then(Value::as_str) == Some("gemini")
            || v.pointer("/$set/messages").is_some()
            || (v.get("projectHash").is_some() && v.get("kind").is_some())
        {
            return Ok(Dialect::Gemini);
        }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --workspace -- --test-threads=1`
Expected: PASS (new gemini test + existing suite green).

- [ ] **Step 7: Commit**

```bash
git add crates/obol-core/src/transcript/gemini.rs crates/obol-core/src/transcript/mod.rs crates/obol-core/tests/fixtures/gemini-mini.jsonl
git commit -m "feat(transcript): gemini dialect (\$set-log, thoughts->output) (PRI-2114)"
```

---

### Task 6: OpenCode dialect (+ single-doc `detect()` fallback)

**Files:**
- Create: `crates/obol-core/src/transcript/opencode.rs`, `crates/obol-core/tests/fixtures/opencode-mini.json`
- Modify: `crates/obol-core/src/transcript/mod.rs`

- [ ] **Step 1: Create the fixture (real-derived)**

`crates/obol-core/tests/fixtures/opencode-mini.json` — one JSON doc; first assistant has message-level tokens, second has tokens only on its `step-finish` part (exercises the fallback):

```json
{"info":{"id":"ses_15777484","directory":"/tmp/p"},"messages":[
  {"role":"user","parts":[{"type":"text","text":"hi"}]},
  {"role":"assistant","modelID":"gpt-5.5","providerID":"openai","cost":0,
   "tokens":{"input":7035,"output":12,"reasoning":0,"cache":{"read":0,"write":0}},
   "parts":[{"type":"step-finish","tokens":{"total":7047,"input":7035,"output":12,"reasoning":0,"cache":{"write":0,"read":0}},"cost":0}]},
  {"role":"assistant","modelID":"gpt-5.5","providerID":"openai",
   "parts":[{"type":"step-finish","tokens":{"input":100,"output":5,"reasoning":2,"cache":{"read":50,"write":0}}}]}
]}
```

- [ ] **Step 2: Write the failing test**

Create `crates/obol-core/src/transcript/opencode.rs` with a test module only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provider;

    #[test]
    fn reads_message_and_step_finish_tokens() {
        let u = parse(include_bytes!("../../tests/fixtures/opencode-mini.json")).unwrap();
        assert_eq!(u.len(), 2, "{u:?}");
        assert_eq!(u[0].model, "gpt-5.5");
        assert_eq!(u[0].provider, Provider::OpenAI);
        assert_eq!(u[0].input_uncached, 7035);
        assert_eq!(u[0].output, 12);
        // fallback to the step-finish part; reasoning folds into output
        assert_eq!(u[1].input_uncached, 100);
        assert_eq!(u[1].cache_read, 50);
        assert_eq!(u[1].output, 7); // 5 + 2 reasoning
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p obol-core --lib opencode -- --test-threads=1`
Expected: FAIL — `parse` not found.

- [ ] **Step 4: Implement the parser**

Prepend to `crates/obol-core/src/transcript/opencode.rs`:

```rust
//! OpenCode `opencode export` JSON -> Vec<MessageUsage>.
//! Reconciled with AgentsView internal/parser/opencode.go (MIT, © 2026 Kenn Software LLC).
//! Single document {info, messages:[...]}; per-assistant usage on the message or its
//! `step-finish` part. `tokens.reasoning` is a separate additive bucket billed as output.
//! Model = `modelID` (bare); provider routed by `providerID`.

use crate::error::ObolError;
use crate::model::{MessageUsage, Provider};
use serde_json::Value;

pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError> {
    let doc: Value = serde_json::from_slice(bytes).map_err(|e| ObolError::MalformedTranscript {
        line: 0,
        msg: e.to_string(),
    })?;
    let messages = match doc.get("messages").and_then(Value::as_array) {
        Some(m) => m,
        None => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    for msg in messages {
        if msg.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let tok = msg
            .get("tokens")
            .filter(|t| t.is_object())
            .or_else(|| step_finish_tokens(msg));
        let tok = match tok {
            Some(t) => t,
            None => continue,
        };
        let g = |k: &str| tok.get(k).and_then(Value::as_u64).unwrap_or(0);
        let input = g("input");
        let cache_read = tok.pointer("/cache/read").and_then(Value::as_u64).unwrap_or(0);
        let cache_write = tok.pointer("/cache/write").and_then(Value::as_u64).unwrap_or(0);
        let output = g("output") + g("reasoning");
        let model = msg
            .get("modelID")
            .and_then(Value::as_str)
            .or_else(|| msg.pointer("/model/modelID").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        let provider_id = msg.get("providerID").and_then(Value::as_str).unwrap_or("");
        out.push(MessageUsage {
            model,
            provider: route_provider(provider_id),
            namespace: "litellm".into(),
            input_uncached: input,
            cache_read,
            cache_write_5m: cache_write,
            cache_write_1h: 0,
            output,
            request_input_tokens: input + cache_read + cache_write,
            service_tier: None,
        });
    }
    Ok(out)
}

fn step_finish_tokens(msg: &Value) -> Option<&Value> {
    msg.get("parts")?
        .as_array()?
        .iter()
        .find(|p| p.get("type").and_then(Value::as_str) == Some("step-finish"))
        .and_then(|p| p.get("tokens"))
        .filter(|t| t.is_object())
}

fn route_provider(provider_id: &str) -> Provider {
    match provider_id {
        "anthropic" => Provider::Anthropic,
        "openai" => Provider::OpenAI,
        "" => Provider::Other("opencode".into()),
        other => Provider::Other(other.to_string()),
    }
}
```

- [ ] **Step 5: Wire into the dialect registry + add the single-doc detect fallback**

In `crates/obol-core/src/transcript/mod.rs`: add `pub mod opencode;`; add `Opencode,` to `Dialect`; add `Dialect::Opencode => opencode::parse(bytes),` to `parse()`. The on-disk file is a single JSON document, so the line loop in `detect()` can't see it — add a whole-document fallback **after** the line loop, replacing the final `Err(ObolError::UnknownDialect)`:

```rust
    // Single-document JSON formats (the line loop above can't see these).
    if let Ok(doc) = serde_json::from_slice::<Value>(bytes) {
        if doc.get("info").is_some() && doc.get("messages").is_some() {
            return Ok(Dialect::Opencode);
        }
    }
    Err(ObolError::UnknownDialect)
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --workspace -- --test-threads=1`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/obol-core/src/transcript/opencode.rs crates/obol-core/src/transcript/mod.rs crates/obol-core/tests/fixtures/opencode-mini.json
git commit -m "feat(transcript): opencode export-json dialect + single-doc detect (PRI-2114)"
```

---

### Task 7: Copilot dialect

**Files:**
- Create: `crates/obol-core/src/transcript/copilot.rs`, `crates/obol-core/tests/fixtures/copilot-mini.jsonl`
- Modify: `crates/obol-core/src/transcript/mod.rs`

- [ ] **Step 1: Create the fixture (synthetic)**

`crates/obol-core/tests/fixtures/copilot-mini.jsonl`:

```
{"type":"session.start","data":{}}
{"type":"assistant.message","data":{"outputTokens":10}}
{"type":"session.shutdown","data":{"modelMetrics":{"claude-sonnet-4-5":{"usage":{"inputTokens":52030,"cacheReadTokens":48000,"cacheWriteTokens":1200,"outputTokens":3100,"reasoningTokens":40}}}}}
```

- [ ] **Step 2: Write the failing test**

Create `crates/obol-core/src/transcript/copilot.rs` with a test module only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provider;

    #[test]
    fn reads_shutdown_aggregate_and_subtracts_cache() {
        let u = parse(include_bytes!("../../tests/fixtures/copilot-mini.jsonl")).unwrap();
        assert_eq!(u.len(), 1, "{u:?}");
        assert_eq!(u[0].model, "claude-sonnet-4-5");
        assert_eq!(u[0].provider, Provider::Anthropic);
        assert_eq!(u[0].input_uncached, 2830); // 52030 - 48000 - 1200
        assert_eq!(u[0].cache_read, 48000);
        assert_eq!(u[0].cache_write_5m, 1200);
        assert_eq!(u[0].output, 3140); // 3100 + 40 reasoning
        assert_eq!(u[0].request_input_tokens, 52030);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p obol-core --lib copilot -- --test-threads=1`
Expected: FAIL — `parse` not found.

- [ ] **Step 4: Implement the parser**

Prepend to `crates/obol-core/src/transcript/copilot.rs`:

```rust
//! Copilot CLI events.jsonl -> Vec<MessageUsage>.
//! Reconciled with AgentsView internal/parser/copilot.go (MIT, © 2026 Kenn Software LLC).
//! Authoritative per-model usage is the `session.shutdown` aggregate
//! (`data.modelMetrics.<model>.usage`). `inputTokens` is a total -> uncached is
//! input - cacheRead - cacheWrite. `reasoningTokens` billed as output. No shutdown
//! event -> no usage (documented limitation).

use crate::error::ObolError;
use crate::model::{MessageUsage, Provider};
use serde_json::Value;

pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ObolError::MalformedTranscript {
        line: 0,
        msg: e.to_string(),
    })?;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(Value::as_str) != Some("session.shutdown") {
            continue;
        }
        let metrics = match v.pointer("/data/modelMetrics").and_then(Value::as_object) {
            Some(m) => m,
            None => continue,
        };
        for (model, mv) in metrics {
            let usage = match mv.get("usage") {
                Some(u) if u.is_object() => u,
                _ => continue,
            };
            let g = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
            let total_input = g("inputTokens");
            let cache_read = g("cacheReadTokens");
            let cache_write = g("cacheWriteTokens");
            out.push(MessageUsage {
                model: model.clone(),
                provider: route_model(model),
                namespace: "litellm".into(),
                input_uncached: total_input.saturating_sub(cache_read).saturating_sub(cache_write),
                cache_read,
                cache_write_5m: cache_write,
                cache_write_1h: 0,
                output: g("outputTokens") + g("reasoningTokens"),
                request_input_tokens: total_input,
                service_tier: None,
            });
        }
    }
    Ok(out)
}

fn route_model(model: &str) -> Provider {
    let m = model.to_ascii_lowercase();
    if m.contains("claude") {
        Provider::Anthropic
    } else if m.contains("gpt") || m.contains("o1") || m.contains("o3") {
        Provider::OpenAI
    } else if m.contains("gemini") {
        Provider::Other("google".into())
    } else {
        Provider::Other("copilot".into())
    }
}
```

- [ ] **Step 5: Wire into the dialect registry**

In `crates/obol-core/src/transcript/mod.rs`: add `pub mod copilot;`; add `Copilot,` to `Dialect`; add `Dialect::Copilot => copilot::parse(bytes),` to `parse()`; add a detect heuristic in the line loop:

```rust
        if matches!(
            v.get("type").and_then(Value::as_str),
            Some("session.shutdown") | Some("assistant.message") | Some("session.start")
        ) {
            return Ok(Dialect::Copilot);
        }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --workspace -- --test-threads=1`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/obol-core/src/transcript/copilot.rs crates/obol-core/src/transcript/mod.rs crates/obol-core/tests/fixtures/copilot-mini.jsonl
git commit -m "feat(transcript): copilot dialect (shutdown aggregate) (PRI-2114)"
```

---

### Task 8: Kimi dialect (+ loud-unpriced integration test)

**Files:**
- Create: `crates/obol-core/src/transcript/kimi.rs`, `crates/obol-core/tests/fixtures/kimi-mini.jsonl`
- Modify: `crates/obol-core/src/transcript/mod.rs`, `crates/obol-core/src/lib.rs`

- [ ] **Step 1: Create the fixture (synthetic)**

`crates/obol-core/tests/fixtures/kimi-mini.jsonl` — a session row (must be ignored) plus two turn rows:

```
{"type":"usage.record","usageScope":"session","model":"kimi-for-coding","time":1800000000000,"usage":{"inputOther":999,"inputCacheRead":0,"inputCacheCreation":0,"output":999}}
{"type":"usage.record","usageScope":"turn","model":"kimi-for-coding","time":1800000000001,"usage":{"inputOther":10,"inputCacheRead":20,"inputCacheCreation":30,"output":40}}
{"type":"usage.record","usageScope":"turn","model":"kimi-for-coding","time":1800000000002,"usage":{"inputOther":1,"inputCacheRead":2,"inputCacheCreation":3,"output":4}}
```

- [ ] **Step 2: Write the failing test**

Create `crates/obol-core/src/transcript/kimi.rs` with a test module only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_turn_rows_over_session() {
        let u = parse(include_bytes!("../../tests/fixtures/kimi-mini.jsonl")).unwrap();
        assert_eq!(u.len(), 2, "session row must be ignored when turns exist: {u:?}");
        assert_eq!(u[0].model, "kimi-for-coding");
        assert_eq!(u[0].input_uncached, 10);
        assert_eq!(u[0].cache_read, 20);
        assert_eq!(u[0].cache_write_5m, 30);
        assert_eq!(u[0].output, 40);
        assert_eq!(u[0].request_input_tokens, 60);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p obol-core --lib kimi -- --test-threads=1`
Expected: FAIL — `parse` not found.

- [ ] **Step 4: Implement the parser**

Prepend to `crates/obol-core/src/transcript/kimi.rs`:

```rust
//! Kimi Code wire.jsonl -> Vec<MessageUsage>.
//! Targets quorum's `usage.record` rows (full input/cache fidelity + model), not
//! agentsview's lower-fidelity StatusUpdate path. Prefer `usageScope:"turn"` rows;
//! fall back to the latest `session` row. Never mix turn + session (double-counts).

use crate::error::ObolError;
use crate::model::{MessageUsage, Provider};
use serde_json::Value;

pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ObolError::MalformedTranscript {
        line: 0,
        msg: e.to_string(),
    })?;
    let mut turns: Vec<Value> = Vec::new();
    let mut sessions: Vec<Value> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(Value::as_str) != Some("usage.record") {
            continue;
        }
        match v.get("usageScope").and_then(Value::as_str) {
            Some("turn") => turns.push(v),
            Some("session") => sessions.push(v),
            _ => {}
        }
    }
    let selected: Vec<Value> = if !turns.is_empty() {
        turns
    } else {
        match sessions
            .into_iter()
            .max_by_key(|r| r.get("time").and_then(Value::as_i64).unwrap_or(i64::MIN))
        {
            Some(latest) => vec![latest],
            None => Vec::new(),
        }
    };
    let mut out = Vec::new();
    for row in &selected {
        let usage = match row.get("usage") {
            Some(u) if u.is_object() => u,
            _ => continue,
        };
        let g = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
        let input = g("inputOther");
        let cache_read = g("inputCacheRead");
        let cache_create = g("inputCacheCreation");
        out.push(MessageUsage {
            model: row
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            provider: Provider::Other("moonshot".into()),
            namespace: "litellm".into(),
            input_uncached: input,
            cache_read,
            cache_write_5m: cache_create,
            cache_write_1h: 0,
            output: g("output"),
            request_input_tokens: input + cache_read + cache_create,
            service_tier: None,
        });
    }
    Ok(out)
}
```

- [ ] **Step 5: Wire into the dialect registry**

In `crates/obol-core/src/transcript/mod.rs`: add `pub mod kimi;`; add `Kimi,` to `Dialect`; add `Dialect::Kimi => kimi::parse(bytes),` to `parse()`; add a detect heuristic in the line loop:

```rust
        if v.get("type").and_then(Value::as_str) == Some("usage.record") {
            return Ok(Dialect::Kimi);
        }
```

- [ ] **Step 6: Add the loud-unpriced integration test**

In `crates/obol-core/src/lib.rs` `api_tests`, prove the full stack flags the unpriced kimi model via the bundled snapshot (no `OBOL_PRICING_DIR`):

```rust
    #[test]
    fn kimi_model_surfaces_unpriced_loudly() {
        std::env::remove_var("OBOL_PRICING_DIR");
        let est = estimate_cost(
            Source::Bytes(include_bytes!("../tests/fixtures/kimi-mini.jsonl")),
            Some(Dialect::Kimi),
        )
        .unwrap();
        assert_eq!(est.total_usd, 0.0, "kimi-for-coding is unpriced -> $0");
        assert!(
            est.unpriced_models.contains(&"kimi-for-coding".to_string()),
            "must name the unpriced model: {:?}",
            est.unpriced_models
        );
    }
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test --workspace -- --test-threads=1`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add crates/obol-core/src/transcript/kimi.rs crates/obol-core/src/transcript/mod.rs crates/obol-core/tests/fixtures/kimi-mini.jsonl crates/obol-core/src/lib.rs
git commit -m "feat(transcript): kimi dialect (usage.record, loud-unpriced) (PRI-2114)"
```

---

## Phase 3 — Surface wiring + verification

### Task 9: Wire the four dialect strings into the CLI + FFI

**Files:**
- Modify: `crates/obol-cli/src/main.rs`, `crates/obol-ffi/src/lib.rs`
- Test: `crates/obol-cli/tests/cli.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/obol-cli/tests/cli.rs` (drive a new dialect end-to-end through the binary, bundled pricing):

```rust
#[test]
fn estimate_gemini_dialect_string() {
    let tmp = tempfile::tempdir().unwrap();
    let gemini = include_str!("../../obol-core/tests/fixtures/gemini-mini.jsonl");
    let transcript = tmp.path().join("session.jsonl");
    fs::write(&transcript, gemini).unwrap();

    Command::cargo_bin("obol")
        .unwrap()
        .env_remove("OBOL_PRICING_DIR")
        .args([
            "estimate",
            transcript.to_str().unwrap(),
            "--dialect",
            "gemini",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("gemini-3-flash-preview"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p obol-cli estimate_gemini_dialect_string -- --test-threads=1`
Expected: FAIL — clap rejects `--dialect gemini` (not in the value set).

- [ ] **Step 3: Update the CLI dialect set + match (fix the `_ => Pi` catch-all)**

In `crates/obol-cli/src/main.rs`, extend the `value_parser` list and replace the dialect-mapping closure with an exhaustive match (the old `_ => Dialect::Pi` would have mis-mapped every new string to Pi):

```rust
        #[arg(long, value_parser = ["claude", "codex", "pi", "gemini", "opencode", "copilot", "kimi"])]
        dialect: Option<String>,
```

```rust
            let hint = dialect.as_deref().map(|d| match d {
                "claude" => Dialect::Claude,
                "codex" => Dialect::Codex,
                "pi" => Dialect::Pi,
                "gemini" => Dialect::Gemini,
                "opencode" => Dialect::Opencode,
                "copilot" => Dialect::Copilot,
                "kimi" => Dialect::Kimi,
                other => unreachable!("clap value_parser restricts dialect; got {other:?}"),
            });
```

- [ ] **Step 4: Update the FFI `parse_dialect`**

In `crates/obol-ffi/src/lib.rs` `parse_dialect`, add the four arms before `_ => Err(())`:

```rust
        "claude" => Ok(Some(Dialect::Claude)),
        "codex" => Ok(Some(Dialect::Codex)),
        "pi" => Ok(Some(Dialect::Pi)),
        "gemini" => Ok(Some(Dialect::Gemini)),
        "opencode" => Ok(Some(Dialect::Opencode)),
        "copilot" => Ok(Some(Dialect::Copilot)),
        "kimi" => Ok(Some(Dialect::Kimi)),
        _ => Err(()),
```

(No `obol.h` change: the C signature is unchanged; only internal string→enum logic grows.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --workspace -- --test-threads=1`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/obol-cli/src/main.rs crates/obol-ffi/src/lib.rs crates/obol-cli/tests/cli.rs
git commit -m "feat(cli,ffi): accept gemini/opencode/copilot/kimi dialect strings (PRI-2114)"
```

---

### Task 10: Full-suite + clippy + smoke

**Files:** none (verification only)

- [ ] **Step 1: Full workspace test**

Run: `cargo test --workspace -- --test-threads=1`
Expected: PASS, no failures.

- [ ] **Step 2: Lint**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings. Fix any nits inline and re-run.

- [ ] **Step 3: Smoke each new dialect through the CLI (bundled pricing)**

With no `OBOL_PRICING_DIR` set:

```bash
cargo run -p obol-cli -- estimate crates/obol-core/tests/fixtures/gemini-mini.jsonl --dialect gemini --json
cargo run -p obol-cli -- estimate crates/obol-core/tests/fixtures/opencode-mini.json --dialect opencode --json
cargo run -p obol-cli -- estimate crates/obol-core/tests/fixtures/copilot-mini.jsonl --dialect copilot --json
cargo run -p obol-cli -- estimate crates/obol-core/tests/fixtures/kimi-mini.jsonl --dialect kimi --json
```

Expected: gemini/opencode/copilot print a non-zero `total_usd` with `"pricing_source": "bundled"`; kimi prints `"total_usd": 0.0` with `kimi-for-coding` in `unpriced_models`.

- [ ] **Step 4: Commit any lint fixes**

```bash
git add -A && git commit -m "chore: clippy clean for new dialects (PRI-2114)"
```

---

## Self-review notes (coverage map)

- Spec §"Pricing ergonomics" (bundle, `pricing_source`, precedence, CLI) → Tasks 1–4.
- Spec §gemini / §opencode / §copilot / §kimi parsers → Tasks 5–8 (each: parser + fixture + `Dialect` variant + `parse()` arm + `detect()`).
- Spec §"single-doc JSON fallback" in `detect()` → Task 6 Step 5.
- Spec §"Pricing & model coverage" loud-unpriced kimi → Task 8 Step 6.
- CLI/FFI dialect strings (exhaustive match, catch-all fix) → Task 9.
- Spec §"opencode reasoning is additive, billed as output" → Task 6 parser (`output + reasoning`) + test.

**Deferred to Part 2 (separate plan):** the required-dialect contract, removal of `Source::Bytes` + `obol_estimate_bytes`, `obol.h` regen, binding propagation (Python/Go/TS), and the cross-consumer parity corpus extension. These are breaking and must land atomically across the FFI + four bindings; doing them here would red the workspace mid-plan.

## Part 2 preview (next plan — `2026-06-08-obol-multi-agent-dialects-contract.md`)

1. `estimate_cost(path, dialect)` required-dialect signature; remove the `Source` enum; add a private `estimate_from_bytes(&[u8], Dialect)` for the CLI's optional `detect()` path.
2. FFI: delete `obol_estimate_bytes`; NULL dialect → `ERR_INVALID_ARG`; regen `crates/obol-ffi/include/obol.h`; update FFI tests.
3. Bindings (Python ctypes, Go purego, TS Bun+Node): drop the bytes wrapper, make `dialect` required. Read each binding first; mirror the change.
4. Extend `bindings/testdata/` + the cross-consumer parity test so byte-identical `total_usd` covers the four new dialects.
