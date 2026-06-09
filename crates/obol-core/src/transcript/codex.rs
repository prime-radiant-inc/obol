//! Codex rollout JSONL -> Vec<MessageUsage>.
//! Reconciled with AgentsView internal/parser/codex.go (MIT, © 2026 Kenn Software LLC).

use crate::error::ObolError;
use crate::model::{MessageUsage, Provider};
use serde_json::Value;

pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ObolError::MalformedTranscript {
        line: 0,
        msg: e.to_string(),
    })?;

    let mut current_model = String::new();
    let mut last_raw: Option<String> = None;
    let mut out = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        let payload = v.get("payload").cloned().unwrap_or(Value::Null);

        if ty == "turn_context" {
            // updates running model; empty string CLEARS it
            current_model = payload
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            continue;
        }
        if ty != "event_msg" || payload.get("type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }
        let last = match payload.pointer("/info/last_token_usage") {
            Some(u) if u.is_object() => u,
            _ => continue,
        };
        // skip streaming retransmit (identical raw)
        let raw = last.to_string();
        if last_raw.as_deref() == Some(raw.as_str()) {
            continue;
        }
        last_raw = Some(raw);

        let g = |k: &str| last.get(k).and_then(Value::as_u64).unwrap_or(0);
        let input = g("input_tokens");
        let cached = g("cached_input_tokens");
        out.push(MessageUsage {
            model: current_model.clone(),
            provider: Provider::OpenAI,
            namespace: "litellm".into(),
            input_uncached: input.saturating_sub(cached),
            cache_read: cached,
            cache_write_5m: 0,
            cache_write_1h: 0,
            // Reasoning is billed as output and reported separately from output_tokens; fold it
            // in (consistent with the gemini/opencode/copilot/kimi dialects). PRI-2124.
            output: g("output_tokens") + g("reasoning_output_tokens"),
            request_input_tokens: input,
            service_tier: None,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_call_usage_dedups_and_subtracts_cache() {
        let bytes = include_bytes!("../../tests/fixtures/codex-mini.jsonl");
        let u = parse(bytes).unwrap();
        assert_eq!(u.len(), 2, "duplicate token_count should be skipped: {u:?}");
        assert_eq!(u[0].model, "gpt-5.5");
        assert_eq!(u[0].input_uncached, 200); // 1000 - 800
        assert_eq!(u[0].cache_read, 800);
        assert_eq!(u[0].output, 60); // 50 output_tokens + 10 reasoning_output_tokens
        assert_eq!(u[1].input_uncached, 100); // 2000 - 1900
        assert_eq!(u[1].cache_read, 1900);
        assert_eq!(u[1].output, 80); // 80 output_tokens + 0 reasoning_output_tokens
    }
}
