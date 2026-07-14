use super::ModelPrice;
use crate::error::ObolError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// All price tables, keyed by namespace ("litellm") then verbatim model string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceStore {
    pub as_of: String,
    pub namespaces: HashMap<String, HashMap<String, ModelPrice>>,
}

impl PriceStore {
    pub fn lookup(&self, namespace: &str, model: &str) -> Option<&ModelPrice> {
        self.namespaces.get(namespace)?.get(model)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, ObolError> {
        Ok(serde_json::from_slice(bytes)?)
    }

    pub fn load(path: &Path) -> Result<Self, ObolError> {
        if !path.exists() {
            return Err(ObolError::PricingTablesMissing(path.to_path_buf()));
        }
        Self::from_json(&std::fs::read(path)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), ObolError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

/// Directory holding price snapshots: $OBOL_PRICING_DIR, else $XDG_DATA_HOME/obol,
/// else $HOME/.local/share/obol.
pub fn pricing_dir() -> PathBuf {
    if let Ok(d) = std::env::var("OBOL_PRICING_DIR") {
        return PathBuf::from(d);
    }
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".local").join("share")
        });
    base.join("obol")
}

/// The active snapshot the library reads.
pub fn current_path() -> PathBuf {
    pricing_dir().join("current.json")
}

/// The price snapshot compiled into the library — the out-of-the-box fallback used
/// when no on-disk snapshot is newer (see `lib::estimate_cost`).
pub fn embedded() -> Result<PriceStore, ObolError> {
    const BYTES: &[u8] = include_bytes!("../../prices/bundled.json");
    PriceStore::from_json(BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> PriceStore {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-opus-4-8".to_string(),
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
            },
        );
        let mut namespaces = HashMap::new();
        namespaces.insert("litellm".to_string(), litellm);
        PriceStore {
            as_of: "2026-06-04".into(),
            namespaces,
        }
    }

    #[test]
    fn lookup_finds_model_in_namespace() {
        let s = store();
        assert!(s.lookup("litellm", "claude-opus-4-8").is_some());
        assert!(s.lookup("litellm", "nonsense").is_none());
        assert!(s.lookup("openrouter", "claude-opus-4-8").is_none());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = std::env::temp_dir().join(format!("obol-test-{}", std::process::id()));
        let path = dir.join("current.json");
        store().save(&path).unwrap();
        let loaded = PriceStore::load(&path).unwrap();
        assert_eq!(loaded, store());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_is_pricing_tables_missing() {
        let path = PathBuf::from("/nonexistent/obol/current.json");
        match PriceStore::load(&path) {
            Err(ObolError::PricingTablesMissing(_)) => {}
            other => panic!("expected PricingTablesMissing, got {other:?}"),
        }
    }

    #[test]
    fn pricing_dir_honors_env_override() {
        let _env = crate::test_env::env_lock();
        std::env::set_var("OBOL_PRICING_DIR", "/tmp/obol-x");
        assert_eq!(pricing_dir(), PathBuf::from("/tmp/obol-x"));
        std::env::remove_var("OBOL_PRICING_DIR");
    }

    #[test]
    fn embedded_snapshot_loads_and_has_models() {
        let s = embedded().expect("embedded snapshot parses");
        assert!(!s.as_of.is_empty(), "embedded snapshot must carry an as_of");
        assert!(
            s.lookup("litellm", "claude-opus-4-8").is_some(),
            "embedded snapshot should price a known model"
        );
    }

    #[test]
    fn embedded_snapshot_prices_the_gpt_56_family() {
        // Lookup is exact-match with no alias fallback, so every id a
        // transcript can emit must be its own key (quorum pins gpt-5.6-sol).
        let s = embedded().expect("embedded snapshot parses");
        for m in ["gpt-5.6", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"] {
            assert!(
                s.lookup("litellm", m).is_some(),
                "embedded snapshot should price {m}"
            );
        }
        let sol = s.lookup("litellm", "gpt-5.6-sol").unwrap();
        assert!((sol.input - 5.0).abs() < 1e-9);
        assert!((sol.output - 30.0).abs() < 1e-9);
        assert!((sol.cache_read - 0.5).abs() < 1e-9);
    }

    #[test]
    fn embedded_snapshot_bundles_litellm_and_openrouter() {
        let s = embedded().expect("embedded snapshot parses");
        assert!(
            s.namespaces.contains_key("litellm"),
            "embedded snapshot must carry the litellm namespace"
        );
        let or = s
            .namespaces
            .get("openrouter")
            .expect("embedded snapshot must carry the openrouter namespace");
        assert!(!or.is_empty(), "openrouter namespace should be non-empty");
        // OpenRouter keys are `<vendor>/<model>` — a run billed through OpenRouter
        // prices from this namespace out of the box.
        assert!(
            or.keys().any(|k| k.contains('/')),
            "openrouter keys are vendor/model form"
        );
    }
}
