//! OpenAI Responses-API `usage` object -> `ProviderTokens`. `input_tokens` is a
//! *total* that still includes cached tokens, so the uncached bucket is the
//! difference — the cached-subtraction a producer must never do itself.

use super::ProviderTokens;
use serde_json::Value;

/// Normalize an OpenAI Responses `usage` object. `input_uncached =
/// input_tokens - input_tokens_details.cached_tokens` (clamped ≥ 0);
/// `cache_read = cached_tokens`; reasoning tokens fold into `output`.
pub fn normalize(usage: &Value) -> ProviderTokens {
    let g = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
    let nested = |obj: &str, k: &str| {
        usage
            .get(obj)
            .and_then(|d| d.get(k))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let cached = nested("input_tokens_details", "cached_tokens");
    let reasoning = nested("output_tokens_details", "reasoning_tokens");
    ProviderTokens {
        input_uncached: g("input_tokens").saturating_sub(cached),
        cache_read: cached,
        cache_write_5m: 0,
        cache_write_1h: 0,
        output: g("output_tokens") + reasoning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn subtracts_cached_from_input_and_folds_reasoning_into_output() {
        let usage = json!({
            "input_tokens": 100,
            "input_tokens_details": {"cached_tokens": 40},
            "output_tokens": 20,
            "output_tokens_details": {"reasoning_tokens": 5}
        });
        let t = normalize(&usage);
        assert_eq!(t.input_uncached, 60);
        assert_eq!(t.cache_read, 40);
        assert_eq!(t.cache_write_5m, 0);
        assert_eq!(t.cache_write_1h, 0);
        assert_eq!(t.output, 25);
    }

    #[test]
    fn handles_missing_details() {
        let usage = json!({"input_tokens": 70, "output_tokens": 6});
        let t = normalize(&usage);
        assert_eq!(t.input_uncached, 70);
        assert_eq!(t.cache_read, 0);
        assert_eq!(t.output, 6);
    }
}
