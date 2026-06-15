pub mod atif;
pub mod claude;
pub mod codex;
pub mod copilot;
pub mod gemini;
pub mod kimi;
pub mod obol;
pub mod opencode;
pub mod pi;
pub mod provider;

use crate::error::ObolError;
use crate::model::MessageUsage;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Atif,
    Claude,
    Codex,
    Copilot,
    Gemini,
    Kimi,
    Obol,
    Opencode,
    Pi,
}

/// Detect dialect from content: Codex lines carry a top-level `payload`
/// (session_meta/response_item/event_msg); Claude lines carry `message` with
/// type user/assistant.
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
        if v.get("payload").is_some() {
            return Ok(Dialect::Codex);
        }
        if matches!(
            v.get("type").and_then(Value::as_str),
            Some("session.shutdown") | Some("assistant.message") | Some("session.start")
        ) {
            return Ok(Dialect::Copilot);
        }
        if v.get("type").and_then(Value::as_str) == Some("usage.record") {
            return Ok(Dialect::Kimi);
        }
        if v.get("type").and_then(Value::as_str) == Some("obol.usage") {
            return Ok(Dialect::Obol);
        }
        if v.get("type").and_then(Value::as_str) == Some("session") {
            return Ok(Dialect::Pi);
        }
        if v.get("type").and_then(Value::as_str) == Some("gemini")
            || v.pointer("/$set/messages").is_some()
            || (v.get("projectHash").is_some() && v.get("kind").is_some())
        {
            return Ok(Dialect::Gemini);
        }
        let ty = v.get("type").and_then(Value::as_str);
        if matches!(ty, Some("user") | Some("assistant")) && v.get("message").is_some() {
            return Ok(Dialect::Claude);
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
        if doc.get("info").is_some() && doc.get("messages").is_some() {
            return Ok(Dialect::Opencode);
        }
    }
    Err(ObolError::UnknownDialect)
}

pub fn parse(bytes: &[u8], dialect: Dialect) -> Result<Vec<MessageUsage>, ObolError> {
    match dialect {
        Dialect::Atif => atif::parse(bytes),
        Dialect::Claude => Ok(claude::parse(bytes)?.usages),
        Dialect::Codex => codex::parse(bytes),
        Dialect::Copilot => copilot::parse(bytes),
        Dialect::Gemini => gemini::parse(bytes),
        Dialect::Kimi => kimi::parse(bytes),
        Dialect::Obol => obol::parse(bytes),
        Dialect::Opencode => opencode::parse(bytes),
        Dialect::Pi => pi::parse(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_claude_and_codex() {
        let claude = include_bytes!("../../tests/fixtures/claude-mini.jsonl");
        let codex = include_bytes!("../../tests/fixtures/codex-mini.jsonl");
        assert_eq!(detect(claude).unwrap(), Dialect::Claude);
        assert_eq!(detect(codex).unwrap(), Dialect::Codex);
    }

    #[test]
    fn detects_pi() {
        let pi = include_bytes!("../../tests/fixtures/pi-mini.jsonl");
        assert_eq!(detect(pi).unwrap(), Dialect::Pi);
    }

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
