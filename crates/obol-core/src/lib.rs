//! obol-core: parse agent transcripts and estimate token cost.

pub mod cost;
pub mod error;
pub mod model;
pub mod pricing;
pub mod transcript;

pub use error::ObolError;
pub use model::{
    Approximation, CostEstimate, MessageUsage, ModelCost, PricingSource, Provider, TokenBuckets,
};
pub use transcript::Dialect;

use std::path::{Path, PathBuf};

/// Report from a pricing refresh.
#[derive(Debug, serde::Serialize)]
pub struct RefreshReport {
    pub models: usize,
    pub as_of: String,
    pub written_to: PathBuf,
}

/// Resolve the price snapshot. Explicit OBOL_PRICING_DIR wins absolutely; otherwise
/// pick whichever of {on-disk current.json, embedded} has the newer `as_of`, on-disk
/// winning ties; embedded is the floor.
fn resolve_store() -> Result<(pricing::PriceStore, PricingSource), ObolError> {
    if std::env::var_os("OBOL_PRICING_DIR").is_some() {
        let store = pricing::PriceStore::load(&pricing::current_path())?;
        return Ok((store, PricingSource::Local));
    }
    let embedded = pricing::embedded()?;
    let embedded_key = pricing::as_of::sort_key(&embedded.as_of)?;
    let local_path = pricing::current_path();
    if local_path.exists() {
        if let Ok(local) = pricing::PriceStore::load(&local_path) {
            // Compare parsed stamps, never raw strings; a local snapshot with an
            // unparseable as_of (pre-validation era) loses to the embedded floor.
            if pricing::as_of::sort_key(&local.as_of).is_ok_and(|k| k >= embedded_key) {
                return Ok((local, PricingSource::Local));
            }
        }
    }
    Ok((embedded, PricingSource::Bundled))
}

/// Estimate the cost of a transcript file under the given dialect. Loads the active
/// price snapshot (bundled fallback) and prices the parsed usage.
pub fn estimate_cost(path: &Path, dialect: Dialect) -> Result<CostEstimate, ObolError> {
    let (store, source_kind) = resolve_store()?;
    let bytes = std::fs::read(path)?;
    let usages = transcript::parse(&bytes, dialect)?;
    Ok(cost::estimate(&usages, &store, source_kind))
}

/// Fetch the LiteLLM sheet and write it as the active snapshot. `as_of` is the
/// caller's stamp — `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SSZ` (the library has no
/// clock) — validated before any network or disk I/O.
pub fn refresh_pricing_tables(as_of: &str) -> Result<RefreshReport, ObolError> {
    pricing::as_of::validate(as_of)?;
    let mut store = pricing::refresh::fetch_litellm(as_of)?; // {litellm: …}
    let openrouter = pricing::refresh::fetch_openrouter()?;
    store
        .namespaces
        .insert("openrouter".to_string(), openrouter);
    let models: usize = store.namespaces.values().map(|m| m.len()).sum();
    let dir = pricing::pricing_dir();
    store.save(&dir.join(pricing::as_of::archive_file_name(as_of)))?;
    let current = pricing::current_path();
    store.save(&current)?;
    Ok(RefreshReport {
        models,
        as_of: as_of.to_string(),
        written_to: current,
    })
}

/// Test-only serialization for env-var-mutating tests, shared crate-wide.
///
/// Several tests mutate process-global env vars (OBOL_PRICING_DIR, XDG_DATA_HOME)
/// and the on-disk snapshot they resolve to. Cargo runs them on multiple threads
/// in one process, so without serialization they race across module boundaries:
/// one test's `set_var` is torn down by another's `remove_var` mid-body, and a
/// `store.save(current_path())` then lands in the developer's REAL
/// ~/.local/share/obol — leaking a fixture snapshot (e.g. the 2099 stamp) that
/// out-ranks every real one. Every test touching env holds this lock for its whole
/// body. Because it spans modules (lib.rs + pricing/store.rs), it must live at the
/// crate root, not inside one test module.
#[cfg(test)]
pub(crate) mod test_env {
    use std::sync::{Mutex, MutexGuard};
    static ENV_LOCK: Mutex<()> = Mutex::new(());
    pub(crate) fn env_lock() -> MutexGuard<'static, ()> {
        // Recover rather than propagate if a prior test panicked while holding it;
        // a poisoned lock must not cascade unrelated failures.
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod api_tests {
    use super::*;
    use crate::test_env::env_lock;

    #[test]
    fn estimate_cost_on_bytes_with_missing_tables_errors() {
        let _env = env_lock();
        std::env::set_var("OBOL_PRICING_DIR", "/nonexistent/obol-xyz");
        let tmp = std::env::temp_dir().join(format!("obol-t-{}-{}", std::process::id(), line!()));
        std::fs::write(
            &tmp,
            include_bytes!("../tests/fixtures/claude-mini.jsonl").as_slice(),
        )
        .unwrap();
        assert!(matches!(
            estimate_cost(&tmp, Dialect::Claude),
            Err(ObolError::PricingTablesMissing(_))
        ));
        std::fs::remove_file(&tmp).ok();
        std::env::remove_var("OBOL_PRICING_DIR");
    }

    #[test]
    fn estimate_cost_end_to_end_with_seeded_store() {
        let _env = env_lock();
        let dir = std::env::temp_dir().join(format!("obol-api-{}", std::process::id()));
        std::env::set_var("OBOL_PRICING_DIR", &dir);
        // seed the store from the sample sheet
        let store = pricing::refresh::normalize_litellm(
            include_bytes!("../tests/fixtures/litellm-sample.json"),
            "2026-06-04",
        )
        .unwrap();
        store.save(&pricing::current_path()).unwrap();

        let tmp = std::env::temp_dir().join(format!("obol-t-{}-{}", std::process::id(), line!()));
        std::fs::write(
            &tmp,
            include_bytes!("../tests/fixtures/claude-mini.jsonl").as_slice(),
        )
        .unwrap();
        let est = estimate_cost(&tmp, Dialect::Claude).unwrap();
        assert!(est.total_usd > 0.0);
        assert_eq!(est.pricing_as_of, "2026-06-04");
        std::fs::remove_file(&tmp).ok();

        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("OBOL_PRICING_DIR");
    }

    #[test]
    fn estimate_cost_from_path_then_detect() {
        let _env = env_lock();
        let dir = std::env::temp_dir().join(format!("obol-path-{}", std::process::id()));
        std::env::set_var("OBOL_PRICING_DIR", &dir);
        let store = pricing::refresh::normalize_litellm(
            include_bytes!("../tests/fixtures/litellm-sample.json"),
            "2026-06-04",
        )
        .unwrap();
        store.save(&pricing::current_path()).unwrap();

        // Write the Claude fixture to a real file, detect dialect, then price.
        let transcript = dir.join("session.jsonl");
        std::fs::write(
            &transcript,
            include_bytes!("../tests/fixtures/claude-mini.jsonl"),
        )
        .unwrap();
        let bytes = std::fs::read(&transcript).unwrap();
        let d = transcript::detect(&bytes).unwrap();
        let est = estimate_cost(&transcript, d).unwrap();
        assert!(est.total_usd > 0.0);

        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("OBOL_PRICING_DIR");
    }

    #[test]
    fn falls_back_to_embedded_when_no_local_snapshot() {
        let _env = env_lock();
        // Force "no on-disk snapshot" hermetically: point XDG at an empty dir so
        // `current_path()` resolves to a nonexistent file and the embedded snapshot
        // is used. (Setting OBOL_PRICING_DIR instead would take the explicit-override
        // branch and error PricingTablesMissing rather than fall back to embedded.)
        let xdg = std::env::temp_dir().join(format!("obol-xdg-{}", std::process::id()));
        std::fs::create_dir_all(&xdg).unwrap();
        std::env::remove_var("OBOL_PRICING_DIR");
        std::env::set_var("XDG_DATA_HOME", &xdg);
        let tmp = std::env::temp_dir().join(format!("obol-t-{}-{}", std::process::id(), line!()));
        std::fs::write(
            &tmp,
            include_bytes!("../tests/fixtures/claude-mini.jsonl").as_slice(),
        )
        .unwrap();
        let est = estimate_cost(&tmp, Dialect::Claude).unwrap();
        assert_eq!(est.pricing_source, crate::model::PricingSource::Bundled);
        assert!(est.total_usd > 0.0, "embedded snapshot should price claude");
        std::fs::remove_file(&tmp).ok();
        std::env::remove_var("XDG_DATA_HOME");
        std::fs::remove_dir_all(&xdg).ok();
    }

    #[test]
    fn explicit_override_uses_local_source() {
        let _env = env_lock();
        let dir = std::env::temp_dir().join(format!("obol-resolve-{}", std::process::id()));
        std::env::set_var("OBOL_PRICING_DIR", &dir);
        let store = pricing::refresh::normalize_litellm(
            include_bytes!("../tests/fixtures/litellm-sample.json"),
            "2099-01-01",
        )
        .unwrap();
        store.save(&pricing::current_path()).unwrap();
        let tmp = std::env::temp_dir().join(format!("obol-t-{}-{}", std::process::id(), line!()));
        std::fs::write(
            &tmp,
            include_bytes!("../tests/fixtures/claude-mini.jsonl").as_slice(),
        )
        .unwrap();
        let est = estimate_cost(&tmp, Dialect::Claude).unwrap();
        assert_eq!(est.pricing_source, crate::model::PricingSource::Local);
        std::fs::remove_file(&tmp).ok();
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("OBOL_PRICING_DIR");
    }

    #[test]
    fn kimi_model_surfaces_unpriced_loudly() {
        let _env = env_lock();
        // Hermetic: point XDG at an empty dir so we price against the embedded
        // snapshot, not whatever the developer happens to have refreshed locally.
        let xdg = std::env::temp_dir().join(format!("obol-xdg-kimi-{}", std::process::id()));
        std::fs::create_dir_all(&xdg).unwrap();
        std::env::remove_var("OBOL_PRICING_DIR");
        std::env::set_var("XDG_DATA_HOME", &xdg);
        let tmp = std::env::temp_dir().join(format!("obol-t-{}-{}", std::process::id(), line!()));
        std::fs::write(
            &tmp,
            include_bytes!("../tests/fixtures/kimi-mini.jsonl").as_slice(),
        )
        .unwrap();
        let est = estimate_cost(&tmp, Dialect::Kimi).unwrap();
        assert_eq!(est.total_usd, 0.0, "kimi-for-coding is unpriced -> $0");
        assert!(
            est.unpriced_models.contains(&"kimi-for-coding".to_string()),
            "must name the unpriced model: {:?}",
            est.unpriced_models
        );
        std::fs::remove_file(&tmp).ok();
        std::env::remove_var("XDG_DATA_HOME");
        std::fs::remove_dir_all(&xdg).ok();
    }

    #[test]
    fn local_snapshot_with_invalid_as_of_loses_to_embedded() {
        let _env = env_lock();
        // "junk-zzzz" sorts lexicographically above any ISO date, which is exactly
        // the bug: precedence must be decided by parsed stamps, not raw strings.
        let xdg = std::env::temp_dir().join(format!("obol-xdg-junk-{}", std::process::id()));
        std::fs::create_dir_all(&xdg).unwrap();
        std::env::remove_var("OBOL_PRICING_DIR");
        std::env::set_var("XDG_DATA_HOME", &xdg);
        let store = pricing::refresh::normalize_litellm(
            include_bytes!("../tests/fixtures/litellm-sample.json"),
            "junk-zzzz",
        )
        .unwrap();
        std::fs::create_dir_all(pricing::pricing_dir()).unwrap();
        store.save(&pricing::current_path()).unwrap();
        let tmp = std::env::temp_dir().join(format!("obol-t-{}-{}", std::process::id(), line!()));
        std::fs::write(
            &tmp,
            include_bytes!("../tests/fixtures/claude-mini.jsonl").as_slice(),
        )
        .unwrap();
        let est = estimate_cost(&tmp, Dialect::Claude).unwrap();
        assert_eq!(
            est.pricing_source,
            crate::model::PricingSource::Bundled,
            "a junk-stamped local snapshot must not beat the embedded floor"
        );
        std::fs::remove_file(&tmp).ok();
        std::env::remove_var("XDG_DATA_HOME");
        std::fs::remove_dir_all(&xdg).ok();
    }

    #[test]
    fn local_datetime_stamp_beats_embedded_date() {
        let _env = env_lock();
        let xdg = std::env::temp_dir().join(format!("obol-xdg-dt-{}", std::process::id()));
        std::fs::create_dir_all(&xdg).unwrap();
        std::env::remove_var("OBOL_PRICING_DIR");
        std::env::set_var("XDG_DATA_HOME", &xdg);
        let store = pricing::refresh::normalize_litellm(
            include_bytes!("../tests/fixtures/litellm-sample.json"),
            "2099-01-01T08:30:00Z",
        )
        .unwrap();
        std::fs::create_dir_all(pricing::pricing_dir()).unwrap();
        store.save(&pricing::current_path()).unwrap();
        let tmp = std::env::temp_dir().join(format!("obol-t-{}-{}", std::process::id(), line!()));
        std::fs::write(
            &tmp,
            include_bytes!("../tests/fixtures/claude-mini.jsonl").as_slice(),
        )
        .unwrap();
        let est = estimate_cost(&tmp, Dialect::Claude).unwrap();
        assert_eq!(est.pricing_source, crate::model::PricingSource::Local);
        std::fs::remove_file(&tmp).ok();
        std::env::remove_var("XDG_DATA_HOME");
        std::fs::remove_dir_all(&xdg).ok();
    }

    #[test]
    fn refresh_rejects_invalid_as_of_before_any_network_or_disk_io() {
        let _env = env_lock();
        let xdg = std::env::temp_dir().join(format!("obol-xdg-rej-{}", std::process::id()));
        std::fs::create_dir_all(&xdg).unwrap();
        std::env::remove_var("OBOL_PRICING_DIR");
        std::env::set_var("XDG_DATA_HOME", &xdg);
        assert!(matches!(
            refresh_pricing_tables("Apr-2027"),
            Err(ObolError::InvalidAsOf(_))
        ));
        assert!(
            !pricing::current_path().exists(),
            "rejected refresh must not write a snapshot"
        );
        std::env::remove_var("XDG_DATA_HOME");
        std::fs::remove_dir_all(&xdg).ok();
    }

    #[test]
    fn refresh_report_serializes() {
        let r = RefreshReport {
            models: 7,
            as_of: "2026-06-05".into(),
            written_to: "/x/current.json".into(),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["models"], 7);
        assert_eq!(v["as_of"], "2026-06-05");
        assert_eq!(v["written_to"], "/x/current.json");
    }
}
