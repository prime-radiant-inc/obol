# obol — `obol` dialect: a house usage sidecar for in-house harnesses (design spec)

> 2026-06-08 · Hermes@81684faa · draft for review · Linear PRI-2125
> Builds on v1 (`2026-06-04-obol-design.md`) and the multi-agent dialects spec
> (`2026-06-08-obol-multi-agent-dialects-design.md`).

## Goal

Give the agent harnesses **we build** — Gauntlet today, others (sen-agent, brooks, harbor-runner…)
later — one shape to emit so obol can price them, instead of obol growing a bespoke parser per
harness. The sidecar (`usage.jsonl`) carries the **raw provider usage object, tagged with its
provider**, one row per LLM call. obol reads it through a single new dialect (`Dialect::Obol`) and
does the provider interpretation it already does for every other dialect.

The whole in-house fleet collapses to **one** new parser plus the per-provider normalizers obol
already needs — not one parser per harness. Gauntlet is the first producer.

## The one idea

Every agent transcript obol parses is an **envelope wrapping a provider usage object**. obol's
parsers do two jobs:

1. **Navigate the agent envelope** — where in *this* file is the usage, dedup streaming snapshots,
   walk the message tree. Agent-specific. Grows with every new harness.
2. **Interpret the provider usage** — the cache-bucket split, OpenAI's cached-subtraction,
   `request_input_tokens` reconstruction. Provider-specific. Unavoidable — it is what lives inside
   the envelope.

The house sidecar **deletes job 1 for in-house tools.** A provider hands its `usage` back as an
in-memory object on the response — already located, nothing to parse. The producer writes that
object down verbatim, tagged with the provider, and obol does only job 2.

**Producer tags; obol normalizes.** The producer records what it uniquely knows — `provider`,
`model`, `service_tier` — and copies the raw `usage` through. It does **no arithmetic**. The
"naive summers get it wrong" math (e.g. Gauntlet's current `src/models/openai.ts:143`,
`input_tokens - cached`) moves out of the producer and into obol, where it lives once, in Rust,
shared with the agent dialects.

## What this is (and isn't)

- **In-house harnesses only.** External CLIs (Claude Code, Codex, Gemini, Copilot, Kimi, opencode)
  keep their native dialects — we don't control their output and can't make them emit this.
- **Raw usage on the wire, not normalized buckets.** Putting normalized buckets on the wire would
  push interpretation back onto producers, scattering the math across every producer language
  (TS, Go, Python). The point of this design is the opposite: keep interpretation in obol's one
  Rust implementation. The row's payload is deliberately raw.
- **Cost projection only.** This is a usage sidecar, not a transcript/replay/observability format.
  A full common transcript was considered and rejected as out of proportion to the need; harnesses
  keep their own native transcripts (Gauntlet's `run.jsonl`, etc.) and emit this *alongside*.
- **Post-hoc, on-disk.** Like the rest of obol — the sidecar is appended to disk as calls complete;
  obol is never pointed at a live stream.

## The wire format — `usage.jsonl`

JSONL, one row per billable LLM call, appended as the call completes (append-only, crash-robust,
matches producers' existing `appendFile` pattern).

**Envelope (obol-defined, the "house" part):**

| field | req | meaning |
|---|---|---|
| `type` | yes | always `"obol.usage"` — the detection marker |
| `v` | yes | schema version as an ISO date, e.g. `"2026-06-08"` — self-locating in time, matching obol's `as_of` grain |
| `provider` | yes | `"anthropic"` \| `"openai"` \| `"openrouter"` \| other; routes the normalizer |
| `model` | yes | verbatim model string, priced exact-match (empty ⇒ unpriced, surfaced loudly) |
| `service_tier` | no | billing tier tag; absent ⇒ standard (`AssumedStandardTier`) |
| `usage` | yes | the provider's `usage` object, **verbatim** |

Non-pricing context fields (`turn`, `ts`, `call_id`, …) are allowed and ignored by obol.

**`service_tier` is a tag, not math.** The producer knows the tier it requested, so it can record it
accurately — strictly better than Codex, whose on-disk format hides it and forces obol to assume
standard. obol reads the tier from the envelope only; it does not dig into the raw `usage` for it.
The `usage` payload is purely token numbers for the normalizer.

**Anthropic row** (`usage` = the `@anthropic-ai/sdk` response usage, untouched):

```json
{"type":"obol.usage","v":"2026-06-08","provider":"anthropic","model":"claude-opus-4-8","service_tier":"standard",
 "usage":{"input_tokens":12,"cache_read_input_tokens":120,"cache_creation_input_tokens":60,
          "cache_creation":{"ephemeral_5m_input_tokens":60,"ephemeral_1h_input_tokens":0},
          "output_tokens":9}}
```

**OpenAI row** (`usage` = the `openai` Responses usage, untouched — note `input_tokens` is a *total*
that still includes `cached_tokens`; obol does the subtraction):

```json
{"type":"obol.usage","v":"2026-06-08","provider":"openai","model":"gpt-5.5",
 "usage":{"input_tokens":7041,"input_tokens_details":{"cached_tokens":4000},
          "output_tokens":6,"output_tokens_details":{"reasoning_tokens":0}}}
```

## Read side

### `Dialect::Obol`

A new `transcript/obol.rs` exposing `pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError>`,
mirroring the other parsers. For each line:

- skip blank lines; a line that fails to parse as JSON ⇒ `malformed_lines += 1` (never a silent
  `$0`), exactly as `claude.rs` does;
- reject an unknown `v` loudly (a schema date obol doesn't recognize) — matched against a known
  set of schema dates as an opaque string, not by date arithmetic; a versioned error, not a mis-parse;
- read the envelope tags; dispatch on `provider` to a provider normalizer (below) to get the token
  buckets; attach `model` / `service_tier`;
- `request_input_tokens` is **derived** by obol (`input_uncached + cache_read + cache_write_5m +
  cache_write_1h`) — it is never on the wire.

No streaming dedup: producers emit one *final* row per call, so the `message.id` collapse `claude.rs`
needs does not apply here.

`detect()` (CLI-only per the multi-agent spec; the library requires an explicit dialect) gains a
branch: `type == "obol.usage"` ⇒ `Dialect::Obol`. `mod.rs` gains the `Obol` variant and a `parse()`
dispatch arm.

### Provider normalizers — the math, in one place

Extract the provider interpretation into `transcript/provider/{anthropic,openai}.rs`, each exposing
`fn normalize(usage: &serde_json::Value) -> MessageUsage`-shaped buckets:

- **anthropic**: `input_uncached = input_tokens` (already uncached); `cache_read =
  cache_read_input_tokens`; split `cache_creation.ephemeral_5m/1h_input_tokens` into
  `cache_write_5m` / `cache_write_1h`, falling back to all-5m when the split is absent (matches
  `claude.rs`); `output = output_tokens`.
- **openai**: `input_uncached = input_tokens − input_tokens_details.cached_tokens` (clamp ≥ 0);
  `cache_read = cached_tokens`; `output = output_tokens (+ output_tokens_details.reasoning_tokens)`,
  reasoning billed as output.

The `obol` dialect calls these. **`claude.rs` is rewired to delegate to `provider/anthropic.rs`** —
clean, because Claude Code embeds the Anthropic usage object near-verbatim, so the integrity test
below pins the Anthropic math to one implementation. The OpenAI normalizer reads the raw
Responses-API usage shape that producers emit; whether `codex.rs` can share it depends on Codex's
on-disk field names matching (its stream/file usage may be shaped differently). If they align, fold
it in; if not, `codex.rs` keeps its inline math for v1 and the consolidation is a follow-up. Either
way the house dialect has its own correct OpenAI normalizer. (The long-tail parsers — gemini,
copilot, kimi, pi, opencode — keep their inline math; folding them in is out of scope here.)

An unknown `provider` routes to `Provider::Other(s)` and is priced by exact `model` string; a miss
surfaces as `unpriced` (loud), never silently zeroed.

## Producer integration — Gauntlet (first adopter)

Gauntlet already holds the raw `response.usage` at the call site (`src/models/anthropic.ts:196`,
`src/models/openai.ts:140`) but *reduces* it into its own `TokenUsage` before the logger sees it.
To emit raw:

1. `models/{anthropic,openai}.ts` surface the raw provider `usage` object and the provider id on
   `AgentResponse` (alongside the reduced `TokenUsage`, which stays for Gauntlet's own display).
2. `agent/agent.ts` passes provider + raw usage to the logger on each response.
3. `EvidenceLogger` captures `provider` / `model` at `logRunStart` and appends one `usage.jsonl` row
   per `logLlmResponse`, carrying the provider tag + raw usage + tier.

Gauntlet's own `TokenUsage` reduction is no longer the cost source of truth — obol over the sidecar
is. Removing the reduction (and its `openai.ts:143` subtraction) is an optional later cleanup, not
part of v1. Per-row `model` means a multi-model run (quorum) needs no special case — obol sums per
model.

## Pricing

Unchanged. Exact-string lookup against the bundled-or-refreshed snapshot, with the loud-unpriced
path and pricing provenance from the multi-agent spec. The sidecar inherits all of it for free.

## Testing

- **obol unit tests** (`transcript/obol.rs`): a raw Anthropic row and a raw OpenAI row → expected
  `MessageUsage`; the OpenAI cached-subtraction; `request_input_tokens` derivation; malformed-line
  counting; loud unknown-`v`; loud unpriced model; absent-tier ⇒ `AssumedStandardTier`.
- **The integrity test:** `obol.rs` parsing `{provider:"anthropic", usage:X}` yields the **same**
  `MessageUsage` as `claude.rs` parsing a Claude Code line whose embedded usage is `X` — because
  both now route through `provider/anthropic.rs`. This is the executable proof that the Anthropic
  math is one implementation. (Add the OpenAI equivalent via `codex.rs` only if that rewire lands.)
- **Fixture:** `tests/fixtures/obol-usage-mini.jsonl` (one Anthropic + one OpenAI row + a malformed
  line).
- **Cross-consumer parity:** extend `bindings/testdata/` so the byte-identical-`total_usd` guarantee
  across all five consumers covers `Dialect::Obol`.
- **Detection test** for `type:"obol.usage"`.
- **Gauntlet:** a vetting run emits a `usage.jsonl` that obol prices with zero malformed and zero
  unpriced rows; an assertion that obol's total over the sidecar matches a hand-computed expected.

## Out of scope / follow-ups

- A full common **transcript** format (replay/observability) — deliberately not this.
- Folding the long-tail agent parsers (gemini, copilot, kimi, pi, opencode) onto shared provider
  normalizers.
- Removing Gauntlet's bespoke `TokenUsage` reduction once obol is the cost source of truth.
- Go / Python writer sugar (a typed `UsageRow` + append helper). Pure convenience — emitting the row
  is ~5 lines in any language with no obol dependency, so this is optional, not load-bearing.
- Threading non-standard `service_tier` if any in-house harness ever requests batch/priority.
- Adopting the sidecar in the next in-house harness (sen-agent, brooks, harbor-runner, …).

## Files touched (orientation, not exhaustive)

**obol:**
- `crates/obol-core/src/transcript/obol.rs` (new), `transcript/provider/{anthropic,openai}.rs` (new,
  extracted).
- `transcript/mod.rs` (`Dialect::Obol`, `parse()` arm, `detect()` branch), `transcript/claude.rs` /
  `transcript/codex.rs` (delegate token math to the shared normalizers).
- `crates/obol-cli/src/main.rs` (`--dialect obol`), `crates/obol-ffi/src/lib.rs` +
  `include/obol.h` (new dialect string, regen), `bindings/*` (dialect enum entry).
- `crates/obol-core/tests/fixtures/obol-usage-mini.jsonl` (new).

**gauntlet:**
- `src/models/{anthropic,openai}.ts` (surface raw usage + provider on `AgentResponse`).
- `src/agent/agent.ts` (pass provider + raw usage to the logger).
- `src/evidence/logger.ts` (`usage.jsonl` writer; capture provider/model at run_start).
- tests.
