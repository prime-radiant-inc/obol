# Differential validation: obol vs superpowers-evals (2026-06-04)

Task 12 acceptance criterion. Confirms obol's per-session cost and token totals
agree with the numbers `superpowers-evals/quorum/token_usage.py` computes on the
SAME real session-log files.

## Method

- Fixtures: 4 real Claude Code sessions + 4 real Codex rollouts, copied into
  `tests/corpus/{claude,codex}/` from `~/Code/prime/coding-agent-files`.
- obol: `obol refresh --as-of 2026-06-04` (LIVE LiteLLM sheet), then
  `obol estimate <file> --dialect <claude|codex> --json`.
- s-evals: `quorum.token_usage.capture_tokens(family, [file])` — HARDCODED
  list pricing verified as-of 2026-05-28 (`PRICING_ASOF`).
- Reproducer: `scripts/validate_against_sevals.sh`.

Token columns: `in` = uncached input, `out` = output, `cr` = cache_read,
`cw` = cache_write (5m+1h). obol `total_usd` rounded to 4dp.

## Results — every file matches exactly (tokens AND cost)

### Claude (Opus 4.7)

| File | obol $ | s-evals $ | in | out | cr | cw | gap |
|---|---|---|---|---|---|---|---|
| 085fac12 | 0.5231 | 0.5231 | 65 | 4131 | 250921 | 47041 | none |
| 73f53cbb | 0.7816 | 0.7816 | 39 | 8535 | 385976 | 60001 | none |
| c157b68a | 0.2061 | 0.2061 | 15 | 1062 | 163881 | 15600 | none |
| d021d1c9 | 0.3965 | 0.3965 | 22 | 2044 | 228345 | 36978 | none |

### Codex (GPT-5.5)

| File | obol $ | s-evals $ | in | out | cr | cw | gap |
|---|---|---|---|---|---|---|---|
| 019e6b26 | 0.1790 | 0.1790 | 9320 | 2396 | 120960 | 0 | none |
| 019e6c73 | 0.2309 | 0.2309 | 18960 | 3066 | 88320 | 0 | none |
| 019e6d09 | 0.2641 | 0.2641 | 26677 | 2378 | 118784 | 0 | none |
| 019e6d1b | 0.2132 | 0.2132 | 11593 | 2978 | 131712 | 0 | none |

All 8 files agree to the token and to the cent. No gaps to diagnose.

## Why the numbers agree (verified, not assumed)

**Pricing source/date — no drift.** The two tools use different price sources
(obol = live LiteLLM @ 2026-06-04; s-evals = hardcoded @ 2026-05-28), so a cost
match is only meaningful if the rates are actually equal. Spot-checked the
per-million rates obol pulled from the live sheet:

| Model | in | out | cache_read | cache_write (5m) | source |
|---|---|---|---|---|---|
| claude-opus-4-7 | 5.0 | 25.0 | 0.50 | 6.25 | obol live == s-evals hardcoded |
| gpt-5.5 | 5.0 | 30.0 | 0.50 | n/a (0) | obol live == s-evals hardcoded |

Opus 4.5+ pricing ($5/$25, not the old $15/$75) is stable across the two dates,
so the cost match is real rather than a fluke of rounding.

**Claude token dedup — same logic.** Both tools dedup assistant records by
`message.id`, last-write-wins (streaming snapshots overwrite, never sum). Same
buckets (uncached input / cache_create / cache_read / output). Hence identical
token totals.

**Codex cumulative-vs-delta — independently reconciled.** The two tools use
*structurally different* Codex accounting yet land on the same number:
- s-evals reads the LAST cumulative `info.total_token_usage`.
- obol sums per-call `info.last_token_usage` deltas (skipping identical-raw
  retransmits) and computes `uncached = input_tokens - cached_input_tokens`.

Verified by hand on all 4 rollouts: obol's delta-sum (`input_raw`, `cached`,
`output`) equals s-evals' final cumulative exactly. This is the strongest signal
in the run — two independent methods agreeing means both Codex parsers are
correct, not merely consistent with each other by construction. None of the
rollouts crossed the 272K long-context tier boundary, so the flat GPT-5.5 rate
applies in both tools.

## Known divergence point NOT exercised by this corpus

obol's Claude parser **skips** assistant records flagged `isMeta: true` or
`isCompactSummary: true` (see `transcript/claude.rs`); s-evals does **not** —
it counts any assistant record that carries a `usage` block. On a session that
contained such a record *with a usage block*, obol would report fewer tokens /
lower cost than s-evals.

Checked the corpus: **none** of the 4 Claude files contain an `isMeta` or
`isCompactSummary` assistant record that also carries a `usage` block, so this
path is not tested here and did not affect any number above. This is a latent
difference to be aware of, not a discrepancy observed in this run. Determining
which behavior is "more correct" (compact-summary usage is real billed tokens,
so arguably s-evals is right to count it; isMeta synthetic turns may or may not
be billed) is left to the controller.

## Bugs found

None. No obol bug surfaced. Token totals match s-evals on every file; cost
matches because the underlying rates are identical across the two pricing dates.
