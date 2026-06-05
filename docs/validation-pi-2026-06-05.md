# Differential validation: obol Pi dialect vs prudence Pi cells (2026-06-05)

PRI-2082 acceptance gate. Confirms obol's per-session Pi cost and token totals
agree with prudence's own per-cell numbers, computed by prudence on the SAME
real Pi `--mode json` transcripts.

## Method

- Source: two real prudence runs with Pi-harness contestants.
  - OpenAI:    `runs/animalia-codex-vs-pidev/2026-06-03T04-54-43Z/`
  - OpenRouter:`runs/animalia-openrouter/2026-06-05T05-22-24Z/`
- Each cell dir is `animalia/<contestant>/<seed>/`. The cellhash IS the
  contestant id; `result.json.contestant_spec.harness == "pi"` identifies a Pi
  cell, and every chosen transcript's first line is `{"type":"session",...}`.
- prudence number: `result.json` — tokens under `pi.tokens` (older runner) or
  `runner.tokens` (newer runner `48e3d5e9`), cost under `total_cost_usd`.
- obol: `obol refresh --as-of 2026-06-04` (LIVE — pulls both `litellm` and
  `openrouter` namespaces; 2342 models: 1996 litellm + 346 openrouter), then
  `obol estimate <decompressed.jsonl> --dialect pi --json`.
- Reproducer: `scripts/validate_pi.sh <cell-dir> [<cell-dir> ...]`.

Token columns: `in` = uncached input, `out` = output, `cr` = cache_read,
`cw` = cache_write.

## Results

| Cell | model | obol $ | prudence $ | in | out | cr | cw | token gap | cost gap |
|---|---|---|---|---|---|---|---|---|---|
| `08b0048a092b/1` (openai) | gpt-5.4 | 0.3937205 | 0.3937205 | 60945 | 11858 | 253952 | 0 | none | none |
| `65be25e1673e/1` (openai) | gpt-5.5 | 0.291725 | 0.291725 | 13423 | 6463 | 61440 | 0 | none | none |
| `1db5b506d9cf/1` (openrouter) | tencent/hy3-preview | 0.0030518 | 0.0036872 | 13331 | 8242 | 22912 | 0 | none | +0.00063 |
| `68434179a5f9/1` (openrouter) | deepseek/deepseek-v4-flash | 0.0553728 | 0.0625593 | 271508 | 26539 | 1191168 | 0 | none | +0.0072 |

**Tokens match exactly on all four cells.** OpenAI cost matches to the dollar.
The two OpenRouter cells have a cost-only gap (matching tokens) — diagnosed
below as a pure price-snapshot difference, not a bug.

## Diagnosis per gap

### Tokens: no gaps. Parser verified by hand.

obol's Pi parser sums `usage` off each `turn_end` record. Hand-summed the
gpt-5.5 cell's 21 `turn_end` usage records independently
(`input/output/cacheRead/cacheWrite`) and got
`13423 / 6463 / 61440 / 0` — identical to obol. So the agreement is the parser
genuinely reading the data, not a coincidence of reading the same field.

### OpenAI cost: exact, and the rate equality is real (not a fluke)

obol's live LiteLLM rates equal prudence's snapshot for both models, so the
to-the-dollar match is genuine:

| Model | input | output | cache_read | source |
|---|---|---|---|---|
| gpt-5.4 | 2.5 | 15.0 | 0.25 | obol live == prudence snapshot |
| gpt-5.5 | 5.0 | 30.0 | 0.50 | obol live == prudence snapshot |

Recomputing each cell's cost from these rates reproduces the `result.json`
figure to full precision (0.3937205 and 0.291725).

### OpenRouter cost: price-snapshot date difference (expected, not a bug)

The OpenRouter cells priced cleanly in obol (no `unpriced_models`), so the gap
is a rate difference between the two price sources, not a missing model. obol
pulls live OpenRouter pricing as-of 2026-06-04; prudence used its committed
snapshot `prices/current.json`. Per-million rates:

| Model | source | input | output | cache_read |
|---|---|---|---|---|
| tencent/hy3-preview | prudence snapshot | 0.066 | 0.26 | 0.029 |
| tencent/hy3-preview | obol live (2026-06-04) | 0.063 | 0.21 | 0.021 |
| deepseek/deepseek-v4-flash | prudence snapshot | 0.112 | 0.224 | 0.022 |
| deepseek/deepseek-v4-flash | obol live (2026-06-04) | 0.0983 | 0.1966 | 0.0197 |

These are preview/flash models whose OpenRouter prices fell between the snapshot
and 2026-06-04; obol's lower rates fully explain obol's lower totals. Each tool
applies its own rates to the IDENTICAL token buckets and reproduces its own
number to full precision:

- tencent: prudence rates -> 0.003687214 (== result.json); obol rates -> 0.003051825 (== obol estimate).
- deepseek: prudence rates -> 0.062559328 (== result.json); obol rates -> 0.055372813 (== obol estimate).

Same tokens, different (correct) rate snapshot. Not a discrepancy in obol.

## OpenRouter path

Priced correctly. Both OpenRouter Pi cells routed to the `openrouter` namespace
(via `pi.rs` `route("openrouter")`), found their model in obol's live OpenRouter
sheet, and produced a priced subtotal with `unpriced_models == []`. The
namespace-routing path is exercised and working; no model surfaced as unpriced.

## Bugs found

None. obol's Pi parser produces token buckets identical to prudence on all four
cells (two OpenAI, two OpenRouter), hand-verified against raw `turn_end` records.
OpenAI cost matches to full precision because the underlying rates are equal.
The OpenRouter cost gap is entirely a live-vs-snapshot OpenRouter pricing
difference on the same tokens, reconciled to full precision under each rate set.
