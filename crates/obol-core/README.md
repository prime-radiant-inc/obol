# obol-core

Read an AI-agent transcript and estimate what it cost. `obol-core` parses Claude Code, Codex, and
Pi transcripts and computes per-message USD cost, handling the accounting naive summers get wrong:
two-layer dedup, cache buckets, and price tiers.

```rust
use obol_core::{estimate_cost, Source};

let est = estimate_cost(Source::Path("session.jsonl".into()), None)?;
println!("{} USD", est.total_usd);
```

Pricing comes from LiteLLM (and OpenRouter for Pi); `refresh_pricing_tables` updates the on-disk
snapshot. Output is a typed `CostEstimate` carrying `unpriced_models` and `approximations` — not a
JSON blob.

Part of [obol](https://github.com/prime-radiant-inc/obol). Apache-2.0.
