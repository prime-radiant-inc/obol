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

/// Where to read the transcript bytes from. The dialect hint is a separate
/// argument to `estimate_cost`, so it applies equally to a path or to bytes.
pub enum Source<'a> {
    Path(&'a Path),
    Bytes(&'a [u8]),
}

/// Report from a pricing refresh.
#[derive(Debug)]
pub struct RefreshReport {
    pub models: usize,
    pub as_of: String,
    pub written_to: PathBuf,
}

/// Estimate the cost of a transcript. `dialect` is an optional hint; when `None`,
/// the dialect is detected from the content. Loads the active price snapshot from
/// disk (errors with `PricingTablesMissing` if absent).
pub fn estimate_cost(source: Source, dialect: Option<Dialect>) -> Result<CostEstimate, ObolError> {
    let store = pricing::PriceStore::load(&pricing::current_path())?;
    let bytes: Vec<u8> = match source {
        Source::Path(p) => std::fs::read(p)?,
        Source::Bytes(b) => b.to_vec(),
    };
    let dialect = match dialect {
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
        let r = estimate_cost(Source::Bytes(data), Some(Dialect::Claude));
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
        let est = estimate_cost(Source::Bytes(data), Some(Dialect::Claude)).unwrap();
        assert!(est.total_usd > 0.0);
        assert_eq!(est.pricing_as_of, "2026-06-04");

        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("OBOL_PRICING_DIR");
    }

    #[test]
    fn estimate_cost_from_path_with_autodetect() {
        let dir = std::env::temp_dir().join(format!("obol-path-{}", std::process::id()));
        std::env::set_var("OBOL_PRICING_DIR", &dir);
        let store = pricing::refresh::normalize_litellm(
            include_bytes!("../tests/fixtures/litellm-sample.json"),
            "2026-06-04",
        )
        .unwrap();
        store.save(&pricing::current_path()).unwrap();

        // Write the Claude fixture to a real file and price it via Source::Path
        // with NO dialect hint — exercises both the path input and auto-detect.
        let transcript = dir.join("session.jsonl");
        std::fs::write(&transcript, include_bytes!("../tests/fixtures/claude-mini.jsonl")).unwrap();
        let est = estimate_cost(Source::Path(&transcript), None).unwrap();
        assert!(est.total_usd > 0.0);

        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("OBOL_PRICING_DIR");
    }
}
