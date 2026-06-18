pub mod atif;
pub mod obol;
pub mod provider;

use crate::error::ObolError;
use crate::model::MessageUsage;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Atif,
    Obol,
}

/// Detect dialect from content: `obol.usage` lines carry `{"type":"obol.usage",...}`;
/// ATIF trajectories are a single-document JSON with a `schema_version` starting with "ATIF-".
pub fn detect(bytes: &[u8]) -> Result<Dialect, ObolError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ObolError::UnknownDialect)?;
    for line in text.lines().take(20) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(Value::as_str) == Some("obol.usage") {
            return Ok(Dialect::Obol);
        }
    }
    // Single-document JSON formats (the line loop above can't see these).
    if let Ok(doc) = serde_json::from_slice::<Value>(bytes) {
        // ATIF trajectory.json: a versioned single document with an agent + steps.
        if doc
            .get("schema_version")
            .and_then(Value::as_str)
            .is_some_and(|v| v.starts_with("ATIF-"))
        {
            return Ok(Dialect::Atif);
        }
    }
    Err(ObolError::UnknownDialect)
}

pub fn parse(bytes: &[u8], dialect: Dialect) -> Result<Vec<MessageUsage>, ObolError> {
    match dialect {
        Dialect::Atif => atif::parse(bytes),
        Dialect::Obol => obol::parse(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_obol() {
        let obol = include_bytes!("../../tests/fixtures/obol-usage-mini.jsonl");
        assert_eq!(detect(obol).unwrap(), Dialect::Obol);
    }

    #[test]
    fn detects_atif() {
        let atif = include_bytes!("../../tests/fixtures/atif-mini.json");
        assert_eq!(detect(atif).unwrap(), Dialect::Atif);
    }

    #[test]
    fn unknown_dialect_errors() {
        assert!(matches!(detect(b"{}\n{}"), Err(ObolError::UnknownDialect)));
    }
}
