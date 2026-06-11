//! Gemini CLI chat transcript -> Vec<MessageUsage>.
//! Reconciled with AgentsView internal/parser/gemini.go (MIT, © 2026 Kenn Software LLC).
//! On-disk form is a `$set`-mutation JSONL: each `$set.messages` is a full snapshot of
//! the conversation; the latest one wins. Usage lives on `type:"gemini"` messages;
//! `tokens.thoughts` (thinking) is billed as output.

use crate::error::ObolError;
use crate::model::{MessageUsage, Provider};
use serde_json::Value;

pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ObolError::MalformedTranscript {
        line: 0,
        msg: e.to_string(),
    })?;
    // Latest non-empty messages snapshot (`$set.messages`, or a bare top-level
    // `messages` for single-doc safety). Last write wins.
    let mut latest: Option<Value> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let msgs = v.pointer("/$set/messages").or_else(|| v.get("messages"));
        if let Some(m) = msgs {
            if m.as_array().is_some_and(|a| !a.is_empty()) {
                latest = Some(m.clone());
            }
        }
    }
    let mut out = Vec::new();
    let msgs = match latest {
        Some(Value::Array(a)) => a,
        _ => return Ok(out),
    };
    for msg in &msgs {
        if msg.get("type").and_then(Value::as_str) != Some("gemini") {
            continue;
        }
        let tok = match msg.get("tokens") {
            Some(t) if t.is_object() => t,
            _ => continue,
        };
        let g = |k: &str| tok.get(k).and_then(Value::as_u64).unwrap_or(0);
        let input = g("input");
        let cached = g("cached");
        let output = g("output") + g("thoughts");
        out.push(MessageUsage {
            model: msg
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            provider: Provider::Other("google".into()),
            namespace: "litellm".into(),
            input_uncached: input,
            cache_read: cached,
            cache_write_5m: 0,
            cache_write_1h: 0,
            output,
            request_input_tokens: input + cached,
            service_tier: None,
            native_cost_usd: None,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_latest_snapshot_and_folds_thoughts_into_output() {
        let u = parse(include_bytes!("../../tests/fixtures/gemini-mini.jsonl")).unwrap();
        assert_eq!(u.len(), 1, "{u:?}");
        assert_eq!(u[0].model, "gemini-3-flash-preview");
        assert_eq!(u[0].input_uncached, 9431);
        assert_eq!(u[0].cache_read, 0);
        assert_eq!(u[0].output, 106); // 12 output + 94 thoughts
        assert_eq!(u[0].request_input_tokens, 9431);
    }
}
