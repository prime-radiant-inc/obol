//! obol-core: parse agent transcripts and estimate token cost.

pub mod cost;
pub mod error;
pub mod model;
pub mod pricing;
pub mod transcript;

pub use error::ObolError;
pub use model::{Approximation, CostEstimate, MessageUsage, ModelCost, Provider, TokenBuckets};
pub use transcript::Dialect;

use std::path::{Path, PathBuf};

/// What to estimate: a file on disk, or in-memory bytes with an optional hint.
pub enum Input<'a> {
    Path(&'a Path),
    Bytes { data: &'a [u8], dialect: Option<Dialect> },
}

/// Report from a pricing refresh.
#[derive(Debug)]
pub struct RefreshReport {
    pub models: usize,
    pub as_of: String,
    pub written_to: PathBuf,
}

/// Estimate the cost of a transcript. Loads the active price snapshot from disk
/// (errors with `PricingTablesMissing` if absent).
pub fn estimate_cost(input: Input) -> Result<CostEstimate, ObolError> {
    let store = pricing::PriceStore::load(&pricing::current_path())?;
    let (bytes, hint): (Vec<u8>, Option<Dialect>) = match input {
        Input::Path(p) => (std::fs::read(p)?, None),
        Input::Bytes { data, dialect } => (data.to_vec(), dialect),
    };
    let dialect = match hint {
        Some(d) => d,
        None => transcript::detect(&bytes)?,
    };
    let usages = transcript::parse(&bytes, dialect)?;
    Ok(cost::estimate(&usages, &store))
}

/// Fetch the LiteLLM sheet and write it as the active snapshot. `as_of` is the
/// caller's date string (the library has no clock).
pub fn refresh_pricing_tables(as_of: &str) -> Result<RefreshReport, ObolError> {
    let store = pricing::refresh::fetch_litellm(as_of)?;
    let models = store.namespaces.get("litellm").map_or(0, |m| m.len());
    let dir = pricing::pricing_dir();
    store.save(&dir.join(format!("litellm-{as_of}.json")))?;
    let current = pricing::current_path();
    store.save(&current)?;
    Ok(RefreshReport { models, as_of: as_of.to_string(), written_to: current })
}

#[cfg(test)]
mod api_tests {
    use super::*;

    #[test]
    fn estimate_cost_on_bytes_with_missing_tables_errors() {
        std::env::set_var("OBOL_PRICING_DIR", "/nonexistent/obol-xyz");
        let data = include_bytes!("../tests/fixtures/claude-mini.jsonl");
        let r = estimate_cost(Input::Bytes { data, dialect: Some(Dialect::Claude) });
        assert!(matches!(r, Err(ObolError::PricingTablesMissing(_))));
        std::env::remove_var("OBOL_PRICING_DIR");
    }

    #[test]
    fn estimate_cost_end_to_end_with_seeded_store() {
        let dir = std::env::temp_dir().join(format!("obol-api-{}", std::process::id()));
        std::env::set_var("OBOL_PRICING_DIR", &dir);
        // seed the store from the sample sheet
        let store = pricing::refresh::normalize_litellm(
            include_bytes!("../tests/fixtures/litellm-sample.json"),
            "2026-06-04",
        )
        .unwrap();
        store.save(&pricing::current_path()).unwrap();

        let data = include_bytes!("../tests/fixtures/claude-mini.jsonl");
        let est = estimate_cost(Input::Bytes { data, dialect: Some(Dialect::Claude) }).unwrap();
        assert!(est.total_usd > 0.0);
        assert_eq!(est.pricing_as_of, "2026-06-04");

        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("OBOL_PRICING_DIR");
    }
}
