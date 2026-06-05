# obol — Pi dialect + OpenRouter pricing (design spec)

> 2026-06-05 · Shevek@7998e83e · draft for review · Linear PRI-2082
> Builds on v1 (`2026-06-04-obol-design.md`). Adds the third dialect (Pi) and the
> second pricing namespace (OpenRouter). Same disciplined loop: spec → plan → TDD → validate.

## Goal

Parse Pi (`pi --mode json`) transcripts and estimate their cost, routing each turn to the
right price table by its `provider`. This requires the OpenRouter pricing namespace (deferred
from v1), because Pi can reach a model through OpenRouter as well as direct/cloud providers.

## What Pi is (and isn't)

Pi is a harness, not a billing route. Prudence runs Claude, Codex, AND Pi harnesses; the
Claude/Codex cells are already obol-native (v1). This spec covers only the **Pi-harness**
transcripts. A given Pi run reaches its model through some backend, and **the `provider`
field on each turn names that backend** — `openrouter`, `openai-codex`, `anthropic`, `openai`,
etc. We assume **token (list) pricing**; we do not model subscription/negotiated/special rates
(no signal for them), and we document that as a limitation.

## Pi transcript format (from real prudence run cells)

Confirmed against `prudence/runs/animalia-codex-vs-pidev/...` (openai) and
`prudence/runs/animalia-openrouter/...` (openrouter). The on-disk file is Pi's `--mode json`
stream (prudence stores it gzip-zstd'd as `transcript.raw.jsonl.zst`, but **obol takes plain
JSONL — decompression is the caller's problem, not an obol dependency**).

- **Detection:** first non-blank line is `{"type":"session", ...}`. (Codex's first line is
  `type:"session_meta"`; Claude's lines carry `message` + `type` user/assistant. So
  `type=="session"` with no `payload` ⇒ Pi.)
- **Usage lives on `turn_end` records** (the stream also has `message_update` streaming
  deltas — ignore those; `turn_end` is the per-turn authoritative total). Per `turn_end`:

```json
"message": {
  "model": "tencent/hy3-preview",   // bare (gpt-5.4) for direct/codex; <vendor>/<model> for openrouter
  "provider": "openrouter",          // the routing key
  "usage": {"input":6412, "output":574, "cacheRead":5760, "cacheWrite":0,
            "totalTokens":12746, "cost":{...}}
}
```

- **Field paths** (reconciled with AgentsView `internal/parser/pi.go`, MIT):
  - model ← `message.model` (fall back to the most recent `model_change.modelId` if absent)
  - provider ← `message.provider`
  - usage ← `message.usage`: `input`, `output`, `cacheRead` (∥ nested `cache.read`),
    `cacheWrite` (∥ nested `cache.write`). The buckets are **disjoint** —
    `input + output + cacheRead = totalTokens` — so `input` is already uncached; no Codex-style
    subtraction.
  - **ignore `usage.cost`** (Pi's self-report; unreliable across backends — compute from tokens)
- **Empty/foreign `usage` object → skip** (no billable usage). Explicit zero is "known zero".

### MessageUsage mapping
Per `turn_end`:
- `input_uncached` ← `usage.input`
- `cache_read` ← `usage.cacheRead` (or `cache.read`)
- `cache_write_5m` ← `usage.cacheWrite` (or `cache.write`); `cache_write_1h` ← 0 (Pi doesn't split TTL)
- `output` ← `usage.output`
- `request_input_tokens` ← `input + cacheRead + cacheWrite` (for any LiteLLM tier check)
- `model` ← `message.model`; `service_tier` ← None
- `provider` / `namespace` ← from the routing rule below

## Routing: provider → namespace + key

| `provider` value | price namespace | lookup key | `Provider` label |
|---|---|---|---|
| `openrouter` | `openrouter` | `model` as-is (`tencent/hy3-preview`) | `OpenRouter` |
| `openai`, `openai-codex` | `litellm` | bare `model` (`gpt-5.4`) | `OpenAI` |
| `anthropic` | `litellm` | bare `model` (`claude-...`) | `Anthropic` |
| `bedrock`/`vertex`/other (if seen) | `litellm` | `model` (already prefixed) | `Other(provider)` |

Rule in one line: **`provider == "openrouter"` → openrouter namespace; everything else →
litellm namespace.** Key is the model string verbatim either way (no normalization). Unknown
models still surface in `unpriced_models` (never silent $0), exactly as v1.

This requires extending `Provider` beyond `{Anthropic, OpenAI}`. Add `OpenRouter` and a
catch-all `Other(String)` (carrying the raw provider for the report). `Provider` is only a
display label — `cost_for` is provider-agnostic — so this is a non-breaking widening.

## OpenRouter pricing namespace

`refresh` now pulls **both** sheets:
- LiteLLM (as v1) → `litellm` namespace.
- OpenRouter `https://openrouter.ai/api/v1/models` → `openrouter` namespace. Per model:
  key = `id` (`<vendor>/<model>`); rates from `pricing.{prompt,completion,input_cache_read,
  input_cache_write}` × 1e6. **No tier fields** (`tier_boundary` = None) — OpenRouter doesn't
  expose the 200k/272k steps, so openrouter-routed pricing is flat. Skip entries with no
  `prompt`/`completion`.

`refresh_pricing_tables` writes both namespaces into the one dated snapshot + `current.json`
(the `PriceStore` is already namespace-keyed, so this is additive — no schema change). The
`RefreshReport` reports the total model count across namespaces (a per-namespace breakdown
is YAGNI pre-1.0 and trivially derivable; not worth widening the struct).

## Dialect detection + dispatch

Extend `transcript::detect`: add the Pi check (`type=="session"` on the first usable line).
Order: `payload` ⇒ Codex; `message` + user/assistant ⇒ Claude; `type=="session"` ⇒ Pi;
else `UnknownDialect`. Add `Dialect::Pi`; `parse` dispatches to `pi::parse`.

## Testing & validation

- **Unit tests** against a small crafted Pi fixture (session header + a couple `turn_end`s,
  one openrouter + one openai, plus a streaming `message_update` to prove it's ignored and an
  empty-usage turn to prove it's skipped). Assert per-turn `MessageUsage` extraction and the
  disjoint-bucket handling.
- **OpenRouter normalize** test against a saved `/api/v1/models` excerpt fixture (key form,
  per-million conversion, cache fields, no tiers).
- **Differential validation (acceptance):** run obol over real Pi cells from
  `prudence/runs/animalia-codex-vs-pidev/*` (openai) and `animalia-openrouter/*` (openrouter),
  decompressed to plain JSONL, and compare per-session totals against prudence's own per-cell
  cost (`result.json`/`summary.json`, priced from prudence's OpenRouter snapshot). Totals
  should agree when the price snapshots align; any gap resolves to a snapshot-date difference
  or obol being more correct — documented, not papered over. Note tencent/hy3-preview etc.
  must be present in obol's OpenRouter snapshot or it surfaces as unpriced (expected, visible).

## Out of scope (still)

Language bindings + C-ABI (next milestone, once the three dialects land). Subscription/special
pricing. Pi's native `~/.pi/agent/sessions` format is the same `turn_end`-bearing dialect, so
it's covered, but we validate against the prudence cells we actually have.

## Open threads (small)

- Confirm which prudence cells are Pi-harness vs Claude/Codex-harness when assembling the
  validation set (the `.raw` may be normalized in some runs; harness↔format to verify).
- `bedrock`/`vertex` provider values are unseen in real fixtures — handle by the catch-all
  (litellm, prefixed key) but don't add dedicated tests until a fixture exists.
