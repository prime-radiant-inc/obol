//! Public result types + the internal per-message usage record.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    OpenAI,
    OpenRouter,
    Other(String),
}

impl Provider {
    pub fn label(&self) -> &str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAI => "openai",
            Provider::OpenRouter => "openrouter",
            Provider::Other(s) => s.as_str(),
        }
    }
}

impl serde::Serialize for Provider {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.label())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TokenBuckets {
    pub input: u64, // uncached input
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64, // 5m + 1h combined, for the summary
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelCost {
    pub model: String,
    pub provider: Provider,
    pub tokens: TokenBuckets,
    pub subtotal_usd: f64,
}

/// Which snapshot priced this estimate: the one compiled into the library, or one
/// read from disk (`refresh`ed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PricingSource {
    Bundled,
    Local,
}

impl serde::Serialize for PricingSource {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            PricingSource::Bundled => "bundled",
            PricingSource::Local => "local",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "detail")]
pub enum Approximation {
    UnpricedModel(String),
    AssumedStandardTier,
    UnknownModelForTurn,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CostEstimate {
    pub total_usd: f64,
    pub per_model: Vec<ModelCost>,
    pub tokens: TokenBuckets,
    pub unpriced_models: Vec<String>,
    pub approximations: Vec<Approximation>,
    pub pricing_as_of: String,
    pub pricing_source: PricingSource,
}

/// One billable API call extracted from a transcript. Produced by the dialect
/// parsers, consumed by the cost engine.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageUsage {
    pub model: String, // verbatim; empty string == unknown (e.g. Codex cleared turn_context)
    pub provider: Provider,
    pub namespace: String, // "litellm" in v1
    pub input_uncached: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub output: u64,
    pub request_input_tokens: u64, // full billed input for THIS call, for tier selection
    pub service_tier: Option<String>,
    /// Provider-reported all-in cost for this call, in USD, when the transcript
    /// carries one (e.g. Pi's `usage.cost.total`). `Some` is ground truth and is
    /// preferred over list-price math; `None` means price from the tables.
    pub native_cost_usd: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_serializes_as_lowercase_string() {
        assert_eq!(
            serde_json::to_value(Provider::OpenRouter).unwrap(),
            serde_json::json!("openrouter")
        );
        assert_eq!(
            serde_json::to_value(Provider::Other("bedrock".into())).unwrap(),
            serde_json::json!("bedrock")
        );
        assert_eq!(
            serde_json::to_value(Provider::OpenAI).unwrap(),
            serde_json::json!("openai")
        );
    }

    #[test]
    fn cost_estimate_serializes_with_expected_fields() {
        let est = CostEstimate {
            total_usd: 1.5,
            per_model: vec![],
            tokens: TokenBuckets::default(),
            unpriced_models: vec![],
            approximations: vec![Approximation::AssumedStandardTier],
            pricing_as_of: "2026-06-04".into(),
            pricing_source: PricingSource::Bundled,
        };
        let v: serde_json::Value = serde_json::to_value(&est).unwrap();
        assert_eq!(v["total_usd"], 1.5);
        assert_eq!(v["pricing_as_of"], "2026-06-04");
        assert_eq!(v["approximations"][0]["kind"], "AssumedStandardTier");
        assert_eq!(v["pricing_source"], "bundled");
    }

    #[test]
    fn pricing_source_serializes_lowercase() {
        assert_eq!(
            serde_json::to_value(PricingSource::Bundled).unwrap(),
            serde_json::json!("bundled")
        );
        assert_eq!(
            serde_json::to_value(PricingSource::Local).unwrap(),
            serde_json::json!("local")
        );
    }
}
