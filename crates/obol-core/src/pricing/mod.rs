//! Price tables and the per-message cost kernel.

pub mod as_of;
pub mod refresh;
pub mod store;
pub use store::{current_path, embedded, pricing_dir, PriceStore};

use crate::model::MessageUsage;
use serde::{Deserialize, Serialize};

/// Per-million-USD rates for one model. Tier-`above` fields apply when a single
/// request's input exceeds `tier_boundary`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write: f64, // 5-minute cache write
    #[serde(default)]
    pub cache_write_1h: Option<f64>,
    #[serde(default)]
    pub tier_boundary: Option<u64>,
    #[serde(default)]
    pub input_above: Option<f64>,
    #[serde(default)]
    pub output_above: Option<f64>,
    #[serde(default)]
    pub cache_read_above: Option<f64>,
    #[serde(default)]
    pub cache_write_above: Option<f64>,
}

/// USD cost of one message's tokens under `price`.
pub fn cost_for(price: &ModelPrice, u: &MessageUsage) -> f64 {
    let above = price
        .tier_boundary
        .is_some_and(|b| u.request_input_tokens > b);
    let pick = |base: f64, over: Option<f64>| if above { over.unwrap_or(base) } else { base };

    let r_in = pick(price.input, price.input_above);
    let r_out = pick(price.output, price.output_above);
    let r_cr = pick(price.cache_read, price.cache_read_above);
    let r_cw5 = pick(price.cache_write, price.cache_write_above);
    let r_cw1 = price.cache_write_1h.unwrap_or(price.cache_write);

    (u.input_uncached as f64 * r_in
        + u.output as f64 * r_out
        + u.cache_read as f64 * r_cr
        + u.cache_write_5m as f64 * r_cw5
        + u.cache_write_1h as f64 * r_cw1)
        / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provider;

    fn usage(input: u64, cr: u64, cw5: u64, out: u64, req_in: u64) -> MessageUsage {
        MessageUsage {
            model: "claude-opus-4-8".into(),
            provider: Provider::Anthropic,
            namespace: "litellm".into(),
            input_uncached: input,
            cache_read: cr,
            cache_write_5m: cw5,
            cache_write_1h: 0,
            output: out,
            request_input_tokens: req_in,
            service_tier: Some("standard".into()),
            native_cost_usd: None,
        }
    }

    // claude-opus-4-8: input 5, output 25, cache_read 0.5, cache_write 6.25 per-million.
    fn opus() -> ModelPrice {
        ModelPrice {
            input: 5.0,
            output: 25.0,
            cache_read: 0.5,
            cache_write: 6.25,
            cache_write_1h: Some(10.0),
            tier_boundary: None,
            input_above: None,
            output_above: None,
            cache_read_above: None,
            cache_write_above: None,
        }
    }

    #[test]
    fn flat_model_costs_each_bucket() {
        // 1M uncached in, 1M cache_read, 1M cache_write_5m, 1M out
        let u = usage(1_000_000, 1_000_000, 1_000_000, 1_000_000, 3_000_000);
        let c = cost_for(&opus(), &u);
        assert!((c - (5.0 + 0.5 + 6.25 + 25.0)).abs() < 1e-9, "got {c}");
    }

    #[test]
    fn tiered_model_switches_rates_above_boundary() {
        // sonnet-4-5: base input 3, above-200k input 6, output 15/22.5
        let price = ModelPrice {
            input: 3.0,
            output: 15.0,
            cache_read: 0.3,
            cache_write: 3.75,
            cache_write_1h: None,
            tier_boundary: Some(200_000),
            input_above: Some(6.0),
            output_above: Some(22.5),
            cache_read_above: Some(0.6),
            cache_write_above: Some(7.5),
        };
        // request input 300k > 200k -> above rates
        let u = usage(1_000_000, 0, 0, 1_000_000, 300_000);
        let c = cost_for(&price, &u);
        assert!((c - (6.0 + 22.5)).abs() < 1e-9, "got {c}");
        // request input 100k < 200k -> base rates
        let u2 = usage(1_000_000, 0, 0, 1_000_000, 100_000);
        let c2 = cost_for(&price, &u2);
        assert!((c2 - (3.0 + 15.0)).abs() < 1e-9, "got {c2}");
    }
}
