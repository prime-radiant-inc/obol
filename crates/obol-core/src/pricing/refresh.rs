//! Fetch and normalize the LiteLLM price sheet into a PriceStore.

use super::{ModelPrice, PriceStore};
use crate::error::ObolError;
use serde_json::Value;
use std::collections::HashMap;

const LITELLM_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

const M: f64 = 1_000_000.0;

fn rate(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64).map(|x| x * M)
}

/// Convert one raw LiteLLM entry into a ModelPrice, or None if it isn't a
/// priced chat model.
fn normalize_entry(v: &Value) -> Option<ModelPrice> {
    let mode = v.get("mode").and_then(Value::as_str);
    if matches!(mode, Some(m) if m != "chat") {
        return None;
    }
    let input = rate(v, "input_cost_per_token")?;
    let output = rate(v, "output_cost_per_token")?;

    // Tier boundary: 200k for Anthropic, 272k for OpenAI — detect by which suffix exists.
    let (tier_boundary, suffix) = if v.get("input_cost_per_token_above_200k_tokens").is_some() {
        (Some(200_000u64), "_above_200k_tokens")
    } else if v.get("input_cost_per_token_above_272k_tokens").is_some() {
        (Some(272_000u64), "_above_272k_tokens")
    } else {
        (None, "")
    };
    let above = |stem: &str| -> Option<f64> {
        if suffix.is_empty() {
            None
        } else {
            rate(v, &format!("{stem}{suffix}"))
        }
    };

    Some(ModelPrice {
        input,
        output,
        cache_read: rate(v, "cache_read_input_token_cost").unwrap_or(0.0),
        cache_write: rate(v, "cache_creation_input_token_cost").unwrap_or(0.0),
        cache_write_1h: rate(v, "cache_creation_input_token_cost_above_1hr"),
        tier_boundary,
        input_above: above("input_cost_per_token"),
        output_above: above("output_cost_per_token"),
        cache_read_above: above("cache_read_input_token_cost"),
        cache_write_above: above("cache_creation_input_token_cost"),
    })
}

/// Build the `litellm` namespace from the raw sheet bytes.
pub fn normalize_litellm(bytes: &[u8], as_of: &str) -> Result<PriceStore, ObolError> {
    let raw: HashMap<String, Value> = serde_json::from_slice(bytes)?;
    let mut litellm = HashMap::new();
    for (key, v) in raw {
        if let Some(price) = normalize_entry(&v) {
            litellm.insert(key, price);
        }
    }
    let mut namespaces = HashMap::new();
    namespaces.insert("litellm".to_string(), litellm);
    Ok(PriceStore {
        as_of: as_of.to_string(),
        namespaces,
    })
}

/// Fetch the live sheet over HTTP. `as_of` is supplied by the caller (the lib
/// has no clock); the CLI passes today's date.
pub fn fetch_litellm(as_of: &str) -> Result<PriceStore, ObolError> {
    let body = ureq::get(LITELLM_URL)
        .call()
        .map_err(|e| ObolError::Network(e.to_string()))?
        .into_string()
        .map_err(|e| ObolError::Network(e.to_string()))?;
    normalize_litellm(body.as_bytes(), as_of)
}

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/models";

/// Build the `openrouter` namespace from the raw /api/v1/models bytes. Keys are
/// OpenRouter ids (`<vendor>/<model>`); rates are per-token strings → per-million.
/// No tier fields (OpenRouter doesn't expose them).
pub fn normalize_openrouter(bytes: &[u8]) -> Result<HashMap<String, ModelPrice>, ObolError> {
    let v: Value = serde_json::from_slice(bytes)?;
    let mut out = HashMap::new();
    let empty = vec![];
    for m in v.get("data").and_then(Value::as_array).unwrap_or(&empty) {
        let id = match m.get("id").and_then(Value::as_str) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let p = match m.get("pricing") {
            Some(p) => p,
            None => continue,
        };
        let f = |k: &str| {
            p.get(k)
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<f64>().ok())
        };
        let (prompt, completion) = match (f("prompt"), f("completion")) {
            (Some(a), Some(b)) => (a, b),
            _ => continue,
        };
        out.insert(
            id,
            ModelPrice {
                input: prompt * M,
                output: completion * M,
                cache_read: f("input_cache_read").unwrap_or(0.0) * M,
                cache_write: f("input_cache_write").unwrap_or(0.0) * M,
                cache_write_1h: None,
                tier_boundary: None,
                input_above: None,
                output_above: None,
                cache_read_above: None,
                cache_write_above: None,
            },
        );
    }
    Ok(out)
}

/// Fetch the live OpenRouter model list.
pub fn fetch_openrouter() -> Result<HashMap<String, ModelPrice>, ObolError> {
    let body = ureq::get(OPENROUTER_URL)
        .call()
        .map_err(|e| ObolError::Network(e.to_string()))?
        .into_string()
        .map_err(|e| ObolError::Network(e.to_string()))?;
    normalize_openrouter(body.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PriceStore {
        let bytes = include_bytes!("../../tests/fixtures/litellm-sample.json");
        normalize_litellm(bytes, "2026-06-04").unwrap()
    }

    #[test]
    fn normalizes_per_million_and_skips_non_chat() {
        let s = sample();
        let opus = s.lookup("litellm", "claude-opus-4-8").unwrap();
        assert!((opus.input - 5.0).abs() < 1e-9);
        assert!((opus.output - 25.0).abs() < 1e-9);
        assert!((opus.cache_read - 0.5).abs() < 1e-9);
        assert!((opus.cache_write - 6.25).abs() < 1e-9);
        assert!((opus.cache_write_1h.unwrap() - 10.0).abs() < 1e-9);
        // image model dropped
        assert!(s.lookup("litellm", "dall-e-3").is_none());
    }

    #[test]
    fn captures_200k_tier_for_sonnet() {
        let s = sample();
        let son = s.lookup("litellm", "claude-sonnet-4-5").unwrap();
        assert_eq!(son.tier_boundary, Some(200_000));
        assert!((son.input_above.unwrap() - 6.0).abs() < 1e-9);
        assert!((son.output_above.unwrap() - 22.5).abs() < 1e-9);
    }

    #[test]
    fn captures_272k_tier_for_gpt() {
        let s = sample();
        let g = s.lookup("litellm", "gpt-5.5").unwrap();
        assert_eq!(g.tier_boundary, Some(272_000));
        assert!((g.input_above.unwrap() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn captures_gpt_56_sol_rates_and_tier() {
        let s = sample();
        let g = s.lookup("litellm", "gpt-5.6-sol").unwrap();
        assert!((g.input - 5.0).abs() < 1e-9);
        assert!((g.output - 30.0).abs() < 1e-9);
        assert!((g.cache_read - 0.5).abs() < 1e-9);
        assert!((g.cache_write - 6.25).abs() < 1e-9);
        assert_eq!(g.tier_boundary, Some(272_000));
        assert!((g.input_above.unwrap() - 10.0).abs() < 1e-9);
        assert!((g.output_above.unwrap() - 45.0).abs() < 1e-9);
    }

    #[test]
    fn normalizes_openrouter_per_million_no_tiers() {
        let t = normalize_openrouter(include_bytes!(
            "../../tests/fixtures/openrouter-sample.json"
        ))
        .unwrap();
        let opus = t.get("anthropic/claude-opus-4.8").unwrap();
        assert!((opus.input - 5.0).abs() < 1e-9);
        assert!((opus.output - 25.0).abs() < 1e-9);
        assert!((opus.cache_read - 0.5).abs() < 1e-9);
        assert_eq!(opus.tier_boundary, None);
        let hy3 = t.get("tencent/hy3-preview").unwrap();
        assert!((hy3.input - 0.066).abs() < 1e-6);
        assert!(!t.contains_key("weird/no-pricing"));
    }
}
