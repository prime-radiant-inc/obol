//! Anthropic `usage` object -> `ProviderTokens`. Shared by `claude.rs` (Claude
//! Code embeds this object near-verbatim) and the `obol` house dialect.

use super::ProviderTokens;
use serde_json::Value;

/// Normalize an Anthropic `usage` object. `input_tokens` is already uncached.
/// `cache_creation_input_tokens` splits into 5m/1h when the `cache_creation`
/// sub-object is present; otherwise the whole amount is treated as 5m (matching
/// the historical `claude.rs` behavior).
pub fn normalize(usage: &Value) -> ProviderTokens {
    let g = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
    let cache_creation = g("cache_creation_input_tokens");
    let (cw5, cw1) = match usage.get("cache_creation") {
        Some(cc) if cc.is_object() => (
            cc.get("ephemeral_5m_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cc.get("ephemeral_1h_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ),
        _ => (cache_creation, 0), // no split -> treat all creation as 5m
    };
    ProviderTokens {
        input_uncached: g("input_tokens"),
        cache_read: g("cache_read_input_tokens"),
        cache_write_5m: cw5,
        cache_write_1h: cw1,
        output: g("output_tokens"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn splits_cache_creation_into_5m_and_1h() {
        let usage = json!({
            "input_tokens": 12,
            "cache_read_input_tokens": 120,
            "cache_creation_input_tokens": 60,
            "cache_creation": {"ephemeral_5m_input_tokens": 50, "ephemeral_1h_input_tokens": 10},
            "output_tokens": 9
        });
        let t = normalize(&usage);
        assert_eq!(t.input_uncached, 12);
        assert_eq!(t.cache_read, 120);
        assert_eq!(t.cache_write_5m, 50);
        assert_eq!(t.cache_write_1h, 10);
        assert_eq!(t.output, 9);
    }

    #[test]
    fn falls_back_to_all_5m_without_split() {
        let usage = json!({
            "input_tokens": 0,
            "cache_read_input_tokens": 200,
            "cache_creation_input_tokens": 60,
            "output_tokens": 7
        });
        let t = normalize(&usage);
        assert_eq!(t.cache_write_5m, 60);
        assert_eq!(t.cache_write_1h, 0);
    }
}
