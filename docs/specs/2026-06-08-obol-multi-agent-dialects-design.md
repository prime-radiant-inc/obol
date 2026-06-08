# obol — kimi / gemini / copilot / opencode dialects + pricing ergonomics (design spec)

> 2026-06-08 · Hermes@cd9c94f1 · draft for review · Linear PRI-2114
> Builds on v1 (`2026-06-04-obol-design.md`) and Pi (`2026-06-05-obol-pi-design.md`).
> Adds four post-hoc file dialects so `superpowers-evals` (quorum) can price its full agent
> roster through obol, plus three pricing-ergonomics changes that let obol work out of the box.

## Goal

Teach obol to read four more on-disk agent transcripts — **gemini, opencode, copilot, kimi** —
and estimate their cost, reusing v1's parser→`MessageUsage`→cost pipeline. Tighten the public
contract (no bytes interface, no library-side auto-detect) and make the tool usable with no
`refresh` step (bundled snapshot + clear provenance in the output).

Schemas are reconciled with agentsview's MIT-licensed Go parsers (`internal/parser/*.go`,
github.com/kenn-io/agentsview) and quorum's `token_usage.py` — the same provenance v1 cites.
gemini and opencode are additionally validated against **real captured transcripts**; copilot
and kimi ship against synthetic fixtures with a real-capture follow-up (see §8).

## What this is (and isn't)

obol's job is **post-hoc cost from a persisted on-disk session file**. That framing decides the
hard boundaries below:

- **Streaming stdout is out of scope.** When an agent is driven in streaming mode (prudence runs
  `claude --output-format stream-json`, `codex exec --json`), the streamed envelope differs from
  the on-disk file (Claude's per-message `usage` is a *partial snapshot*; the authoritative
  cumulative total + `total_cost_usd` live only in a terminal `result` event; Codex's stream is
  `turn.completed.usage` with no `payload`/`event_msg`/`token_count` wrapper). obol must never be
  pointed at a stream. The decisive fact: **both CLIs still write their normal on-disk session
  file even in streaming mode**, so the caller points obol at that file. This matches agentsview,
  which is a filesystem post-hoc parser with no streaming path.
- **antigravity is excluded.** Its on-disk format (SQLite + opaque/encrypted protobuf) carries no
  token or model data; agentsview decodes only role/content by heuristic string-scraping and
  extracts zero usage. There is nothing to price. Documented non-goal.
- **Multi-file aggregation is the caller's job.** obol prices one transcript per call. A run that
  spans several files (e.g. Claude subagents) is summed/averaged/delta'd by the caller, who owns
  that semantics.

## Contract changes (apply first)

These reshape the public surface and are prerequisites for the dialect work.

### Dialect is required; no library-side auto-detect

```rust
// before: pub fn estimate_cost(source: Source, dialect: Option<Dialect>) -> Result<…>
pub fn estimate_cost(path: &Path, dialect: Dialect) -> Result<CostEstimate, ObolError>;
```

- `dialect` becomes a required argument. The consumer (quorum) launched the agent and knows the
  dialect; sniffing throws away certain knowledge to recover it probabilistically.
- `transcript::detect()` stays public but is **CLI-only and best-effort**. The CLI uses it when
  `--dialect` is omitted; the library never calls it. `detect()` gains a whole-document JSON
  fallback (today it only sniffs line-by-line, so it can't see single-doc formats), and cheap
  unambiguous heuristics for the new dialects (see each §). Detection is explicitly a convenience,
  not a guarantee.

### No bytes interface

- Remove the `Source` enum and `Source::Bytes`. The core still works on bytes *internally* (it
  must, to parse) via a private `estimate_from_bytes(&[u8], Dialect)` helper, but bytes never
  cross the FFI/binding seam.
- **FFI:** delete `obol_estimate_bytes`. Keep only `obol_estimate_path(path, dialect, out_json)`.
  A NULL `dialect` now returns `ERR_INVALID_ARG` (was: auto-detect). `parse_dialect` gains
  `"gemini" | "opencode" | "copilot" | "kimi"`. Regenerate `crates/obol-ffi/include/obol.h`
  (the header-parity test enforces it).
- **Bindings:** Python, Go, TS-Bun, TS-Node each drop their bytes wrapper and make `dialect` a
  required argument. This is a breaking binding change — acceptable pre-1.0.

## The four parsers

Each is a new module under `crates/obol-core/src/transcript/`, exposing
`pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError>`, mirroring
`claude.rs`/`codex.rs`/`pi.rs`. Each adds a `Dialect` variant, a `parse()` dispatch arm, and a
best-effort `detect()` heuristic. Model strings are kept **verbatim** (no cross-source
normalization — see v1/Pi pricing design); `namespace = "litellm"`; `provider` is routed by
model/agent shape.

### gemini

**File:** `~/.gemini/tmp/<projectHash>/chats/session-<ts>-<id>.jsonl`. Discovered by quorum via
`GEMINI_CLI_HOME`. **Real fixture captured** (`session-2026-06-08T18-46-329cdf31.jsonl`).

**Format — a `$set`-mutation log, not plain message-JSONL.** Lines are one of: a session header
(`{"sessionId","projectHash","startTime","kind":"main"}`), a top-level event
(`{"type":"error"|"info"|"user", ...}`), or a state patch
`{"$set":{"messages":[...], "lastUpdated":...}}`. Each `$set.messages` is a full snapshot of the
conversation so far. The assistant turn carrying usage is a message of `type:"gemini"`:

```json
{"type":"gemini","model":"gemini-3-flash-preview",
 "tokens":{"input":9431,"output":12,"cached":0,"thoughts":94,"tool":0,"total":9537}}
```

**Parse:** scan all lines; track the latest line that carries a non-empty `$set.messages`; that
array is the authoritative final snapshot. Emit one `MessageUsage` per `type:"gemini"` message in
it. Map: `input_uncached = tokens.input`, `cache_read = tokens.cached`, `cache_write_* = 0`,
`output = tokens.output + tokens.thoughts` (thinking tokens are billed as output, per agentsview),
`request_input_tokens = tokens.input + tokens.cached`. `provider = Other("google")`. `tokens.tool`
and `tokens.total` are ignored. (Multi-turn snapshots are assumed full; if a future capture shows
incremental `messages` patches, revisit — the hello-world is single-turn.)

**Pricing:** `gemini-3-flash-preview` is present in the LiteLLM snapshot → priced.

**Detection (best-effort):** a session-header line with `projectHash` + `kind`, or any line with a
`$set.messages` array, or a `type:"gemini"` message.

### opencode

**Artifact:** the single-document JSON produced by `opencode export <session>` —
`{info:{…}, messages:[…]}` — **not** the raw SQLite `opencode.db`. quorum already runs
`opencode export` into a per-session file. (The raw-db path is a documented future option, §7; it
would need a libsqlite3 C dependency that complicates the platform-wheel packaging, so it is
deferred.) Per-message token schema **verified** from a real local `opencode.db` (`message.data`
and `step-finish` `part.data` blobs are exactly what the export serializes).

**Per-message / per-`step-finish` token shape:**

```json
{"role":"assistant","modelID":"gpt-5.5-pro","providerID":"openai","cost":0,
 "tokens":{"input":7041,"output":6,"reasoning":0,"cache":{"read":0,"write":0}}}
```

**Parse:** walk `messages[]`; for each assistant message, read its `tokens` object (and/or a
`step-finish` part's `tokens` — look in both, prefer message-level when present). Map:
`input_uncached = tokens.input`, `cache_read = tokens.cache.read`,
`cache_write_5m = tokens.cache.write`, `output = tokens.output + tokens.reasoning`,
`request_input_tokens = input + cache.read + cache.write`. Model = `modelID` (verbatim, bare e.g.
`gpt-5.5-pro`); route `provider`/`namespace` from `providerID` (`openai`→OpenAI, `anthropic`→
Anthropic, else Other), all priced from the LiteLLM namespace. (`tokens.reasoning` is a separate
additive bucket — `tokens.total = input + output + reasoning + cache.read + cache.write`, and the
`session` table keeps `tokens_output`/`tokens_reasoning` distinct — so it is folded into output and
billed at the output rate, consistent with copilot's `reasoningTokens` and gemini's `thoughts`.)

**Pricing:** bare `gpt-…`/`claude-…` model strings resolve in the snapshot where present;
`gpt-5.5-pro` may be unpriced → surfaces loudly (§6). No silent normalization.

**Detection (best-effort):** single-doc JSON with top-level `info` + `messages` array.

### copilot

**File:** `<COPILOT_HOME>/session-state/<id>/events.jsonl` (or bare `<id>.jsonl`). JSONL, one
event per line. **Synthetic fixture** (no real capture yet).

**Token source — the session aggregate, emitted at shutdown.** Per-message `assistant.message`
events carry only `data.outputTokens`; the authoritative per-model accounting is in the
`session.shutdown` event:

```json
{"type":"session.shutdown","data":{"modelMetrics":{
  "claude-sonnet-4-6":{"usage":{"inputTokens":52030,"cacheReadTokens":48000,
     "cacheWriteTokens":1200,"outputTokens":3100,"reasoningTokens":40}}}}}
```

**Parse:** on `session.shutdown`, iterate `data.modelMetrics` (keyed by model). Emit one
`MessageUsage` per model. `inputTokens` is a **total** → `input_uncached = inputTokens -
cacheReadTokens - cacheWriteTokens` (clamp ≥ 0); `cache_read = cacheReadTokens`,
`cache_write_5m = cacheWriteTokens`, `output = outputTokens + reasoningTokens` (reasoning billed as
output), `request_input_tokens = inputTokens`. Model = the metrics map key (verbatim). Route
provider by model shape (claude→Anthropic, gpt→OpenAI, gemini→Other("google")).

**Limitation:** if a run produced no `session.shutdown` (crash/kill), there is no aggregate and the
parser yields no usage (documented; surfaces as an empty estimate, not a wrong one).

**Detection (best-effort):** a line with `type:"session.start"`, `type:"assistant.message"`, or
`type:"session.shutdown"`.

### kimi

**File:** `<KIMI_CODE_HOME>/sessions/<hash>/<id>/wire.jsonl`. JSONL. **Synthetic fixture.**

**Token source — quorum's `usage.record` rows** (full input/cache fidelity, model present). obol
deliberately targets this schema rather than agentsview's lower-fidelity `StatusUpdate` path
(output + context only, no model):

```json
{"type":"usage.record","usageScope":"turn","model":"kimi-for-coding","time":1800000000000,
 "usage":{"inputOther":10,"inputCacheRead":20,"inputCacheCreation":30,"output":40}}
```

**Parse:** collect `usage.record` rows. Prefer `usageScope:"turn"` rows (one `MessageUsage` each);
if there are none, fall back to the single latest `usageScope:"session"` row (by `time`) — never
mix turn + session (double-counts). Map: `input_uncached = usage.inputOther`,
`cache_read = usage.inputCacheRead`, `cache_write_5m = usage.inputCacheCreation`,
`output = usage.output`, `request_input_tokens = inputOther + inputCacheRead + inputCacheCreation`.
Model = `model` (verbatim, `kimi-for-coding`); `provider = Other("moonshot")`.

**Pricing:** `kimi-for-coding` is **not** in the LiteLLM snapshot and won't be after `refresh`
(the snapshot has `moonshot/kimi-k2-*`, not this string). kimi therefore parses tokens correctly
but surfaces a **loud `UnpricedModel`** ($0, flagged — §6). A pricing alias is deferred (§8); we do
not invent a price on synthetic data.

**Detection (best-effort):** a line with `type:"usage.record"`.

## Pricing & model coverage

obol's lookup is exact-string against `namespaces[namespace][model]`. Coverage is best-effort and
every gap is surfaced loudly — never silently zeroed.

- gemini / opencode / copilot emit model strings that resolve in the LiteLLM snapshot where
  present → priced.
- **Unpriced is loud, not silent.** When `store.lookup` misses, the model still appears in
  `per_model` with its full token buckets and `subtotal_usd: 0.0`, is named in
  `CostEstimate.unpriced_models`, and is recorded as `Approximation::UnpricedModel(model)`. The CLI
  prints an `unpriced (...)` line. `total_usd` sums only what it can price; the caller reads
  `unpriced_models` to know the total is partial. (Existing behavior; covered by
  `cost.rs::unpriced_model_surfaces_not_silently_zero`.) This is the only correct outcome for
  `kimi-for-coding`, `gpt-5.5-pro`, and any dot-form model name pending §8.
- **Known follow-up gotcha:** dot vs dash model keys (`claude-sonnet-4.6` vs `claude-sonnet-4-6`).
  obol emits verbatim and lets misses surface; a deliberate in-namespace canonicalization is §8
  work, not a silent rewrite here.

## Pricing ergonomics

### Bundle a snapshot (works out of the box)

Embed a price snapshot in `obol-core` via `include_bytes!` (the committed `.pricing/current.json`,
~700 KB; carries its own `as_of`). `pricing::embedded() -> PriceStore` returns it. obol prices
with no prior `refresh`.

### Surface pricing provenance in the output

`CostEstimate.pricing_as_of` already exists and prints in the CLI. Add a sibling field
**`pricing_source: "bundled" | "local"`** so it is obvious whether the estimate used the shipped
snapshot or a refreshed on-disk one. Both fields reflect whichever snapshot was actually used.

### Defer to the newer / local snapshot

Snapshot resolution in `estimate_cost` becomes:

1. If `OBOL_PRICING_DIR` is set explicitly → use that dir's `current.json` (override wins
   absolutely; missing → `PricingTablesMissing`, preserving the power-user contract).
2. Otherwise pick **max-by-`as_of`** of { on-disk `current.json` (XDG dir, if present), embedded };
   on-disk wins ties (`pricing_source:"local"`). A fresh `refresh` beats the bundle; a stale local
   loses to a newer shipped bundle.

`PricingTablesMissing` retires as a default-path error — there is always the embedded fallback.

## Testing

- **Per-parser unit tests** under `crates/obol-core/`: extraction correctness, the gemini
  latest-`$set.messages` snapshot rule, the copilot shutdown-aggregate + `inputTokens` subtraction,
  the kimi turn-vs-session preference, and the loud-unpriced kimi path.
- **Real fixtures** for gemini (`gemini-mini.jsonl`, from the captured session) and opencode
  (`opencode-mini.json`, built from the verified per-message schema). **Synthetic fixtures** for
  copilot (`copilot-mini.jsonl`) and kimi (`kimi-mini.jsonl`).
- **Pricing-ergonomics tests:** embedded-fallback when no on-disk snapshot; max-by-`as_of`
  precedence; explicit-`OBOL_PRICING_DIR` override; `pricing_source` value.
- **Detection tests** (best-effort) for the four, including the single-doc JSON fallback.
- **Cross-consumer parity:** extend `bindings/testdata/` so the existing "byte-identical
  `total_usd` across all five consumers" guarantee covers the new dialects (on the fixtures above).
- **FFI tests** updated: drop the bytes tests, exercise path + required-dialect + NULL-dialect →
  `ERR_INVALID_ARG`.

## Out of scope / future (follow-up ticket)

- Capture real **copilot** + **kimi** runs; validate the parsers byte-exact.
- Confirm `opencode export` actually serializes the per-message `tokens` (verified in the db; the
  export wrapper is assumed to carry it).
- Add a **kimi pricing alias** once the real model string is confirmed.
- Resolve dot/dash model-key gaps and `gpt-5.5-pro` coverage (in-namespace, deliberate).
- **opencode-sqlite** support (read `opencode.db`'s `session` aggregate / message blobs directly),
  if/when a consumer needs to price the raw db without exporting — weighed against the libsqlite3 C
  dependency cost.
- **antigravity** — only if a real run ever exposes token data in some artifact.

## Files touched (orientation, not exhaustive)

- `crates/obol-core/src/transcript/{gemini,opencode,copilot,kimi}.rs` (new) + `mod.rs`
  (`Dialect` variants, `parse()` dispatch, `detect()` heuristics + single-doc fallback).
- `crates/obol-core/src/lib.rs` (`estimate_cost` signature, `Source` removal, snapshot resolution),
  `pricing/store.rs` (`embedded()`, precedence), `model.rs` (`pricing_source`).
- `crates/obol-ffi/src/lib.rs` (drop `obol_estimate_bytes`, required dialect, new dialect strings)
  + `include/obol.h` (regen).
- `crates/obol-cli/src/main.rs` (`--dialect` value set, CLI-only detect).
- `bindings/{python,go,typescript}` (drop bytes wrapper, required dialect) + `bindings/testdata/`.
- `crates/obol-core/tests/fixtures/` (four new fixtures).
