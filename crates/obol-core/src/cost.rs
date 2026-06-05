//! Price a Vec<MessageUsage> against a PriceStore into a CostEstimate.

use crate::model::{Approximation, CostEstimate, MessageUsage, ModelCost, Provider, TokenBuckets};
use crate::pricing::{cost_for, PriceStore};
use std::collections::BTreeMap;

pub fn estimate(usages: &[MessageUsage], store: &PriceStore) -> CostEstimate {
    let mut per_model: BTreeMap<String, ModelCost> = BTreeMap::new();
    let mut totals = TokenBuckets::default();
    let mut total_usd = 0.0;
    let mut unpriced: Vec<String> = Vec::new();
    let mut saw_openai = false;
    let mut saw_unknown_model = false;

    for u in usages {
        totals.input += u.input_uncached;
        totals.output += u.output;
        totals.cache_read += u.cache_read;
        totals.cache_write += u.cache_write_5m + u.cache_write_1h;
        if u.provider == Provider::OpenAI {
            saw_openai = true;
        }
        if u.model.is_empty() {
            saw_unknown_model = true;
        }

        let price = store.lookup(&u.namespace, &u.model);
        let subtotal = match price {
            Some(p) => cost_for(p, u),
            None => {
                if !unpriced.contains(&u.model) {
                    unpriced.push(u.model.clone());
                }
                0.0
            }
        };
        total_usd += subtotal;

        let entry = per_model
            .entry(u.model.clone())
            .or_insert_with(|| ModelCost {
                model: u.model.clone(),
                provider: u.provider.clone(),
                tokens: TokenBuckets::default(),
                subtotal_usd: 0.0,
            });
        entry.tokens.input += u.input_uncached;
        entry.tokens.output += u.output;
        entry.tokens.cache_read += u.cache_read;
        entry.tokens.cache_write += u.cache_write_5m + u.cache_write_1h;
        entry.subtotal_usd += subtotal;
    }

    let mut approximations = Vec::new();
    for m in &unpriced {
        approximations.push(Approximation::UnpricedModel(m.clone()));
    }
    if saw_openai {
        approximations.push(Approximation::AssumedStandardTier);
    }
    if saw_unknown_model {
        approximations.push(Approximation::UnknownModelForTurn);
    }

    CostEstimate {
        total_usd,
        per_model: per_model.into_values().collect(),
        tokens: totals,
        unpriced_models: unpriced,
        approximations,
        pricing_as_of: store.as_of.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::refresh::normalize_litellm;

    fn store() -> PriceStore {
        let bytes = include_bytes!("../tests/fixtures/litellm-sample.json");
        normalize_litellm(bytes, "2026-06-04").unwrap()
    }

    fn opus_usage() -> MessageUsage {
        MessageUsage {
            model: "claude-opus-4-8".into(),
            provider: Provider::Anthropic,
            namespace: "litellm".into(),
            input_uncached: 1_000_000,
            cache_read: 0,
            cache_write_5m: 0,
            cache_write_1h: 0,
            output: 1_000_000,
            request_input_tokens: 1_000_000,
            service_tier: Some("standard".into()),
        }
    }

    #[test]
    fn prices_known_model_exactly() {
        let est = estimate(&[opus_usage()], &store());
        // 1M input @5 + 1M output @25 = 30
        assert!((est.total_usd - 30.0).abs() < 1e-9, "got {}", est.total_usd);
        assert_eq!(est.per_model.len(), 1);
        assert_eq!(est.tokens.input, 1_000_000);
        assert!(est.unpriced_models.is_empty());
        assert_eq!(est.pricing_as_of, "2026-06-04");
    }

    #[test]
    fn unpriced_model_surfaces_not_silently_zero() {
        let mut u = opus_usage();
        u.model = "claude-opus-9-9".into();
        let est = estimate(&[u], &store());
        assert_eq!(est.total_usd, 0.0);
        assert_eq!(est.unpriced_models, vec!["claude-opus-9-9".to_string()]);
        assert!(est
            .approximations
            .iter()
            .any(|a| matches!(a, Approximation::UnpricedModel(m) if m == "claude-opus-9-9")));
    }
}
