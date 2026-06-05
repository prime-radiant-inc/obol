pub mod claude;
pub mod codex;
pub mod pi;

use crate::error::ObolError;
use crate::model::MessageUsage;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Claude,
    Codex,
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
        if v.get("type").and_then(Value::as_str) == Some("session") {
            return Ok(Dialect::Pi);
        }
        let ty = v.get("type").and_then(Value::as_str);
        if matches!(ty, Some("user") | Some("assistant")) && v.get("message").is_some() {
            return Ok(Dialect::Claude);
        }
    }
    Err(ObolError::UnknownDialect)
}

pub fn parse(bytes: &[u8], dialect: Dialect) -> Result<Vec<MessageUsage>, ObolError> {
    match dialect {
        Dialect::Claude => Ok(claude::parse(bytes)?.usages),
        Dialect::Codex => codex::parse(bytes),
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
    fn unknown_dialect_errors() {
        assert!(matches!(detect(b"{}\n{}"), Err(ObolError::UnknownDialect)));
    }
}
