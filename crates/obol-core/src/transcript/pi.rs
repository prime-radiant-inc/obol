//! Pi (`pi --mode json`) transcript -> Vec<MessageUsage>.
//! Reconciled with AgentsView internal/parser/pi.go (MIT, © 2026 Kenn Software LLC).

use crate::error::ObolError;
use crate::model::{MessageUsage, Provider};
use serde_json::Value;

pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ObolError::MalformedTranscript {
        line: 0,
        msg: e.to_string(),
    })?;
    // Pi serializes the same usage on two disjoint channels: the `pi --mode json`
    // stdout stream carries it on `turn_end` rows, while persisted session files
    // carry it on `type:"message"` (role=assistant) rows and contain no `turn_end`.
    // Bucket each shape separately and return one — never the sum — so a transcript
    // that ever carried both (it shouldn't) can't be double-counted.
    let mut from_turn_end = Vec::new();
    let mut from_message = Vec::new();
    let mut current_model = String::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ty = v.get("type").and_then(Value::as_str);
        // Track the running model so a row without `message.model` can inherit it.
        if ty == Some("model_change") {
            if let Some(m) = v.get("modelId").and_then(Value::as_str) {
                current_model = m.to_string();
            }
            continue;
        }
        match ty {
            // Stdout-stream shape; message_update/start/end deltas are ignored.
            Some("turn_end") => {
                if let Some(rec) = extract(v.get("message"), &current_model) {
                    from_turn_end.push(rec);
                }
            }
            // Session-file shape: usage on the persisted assistant message.
            Some("message") => {
                let msg = v.get("message");
                let is_assistant =
                    msg.and_then(|m| m.get("role")).and_then(Value::as_str) == Some("assistant");
                if is_assistant {
                    if let Some(rec) = extract(msg, &current_model) {
                        from_message.push(rec);
                    }
                }
            }
            _ => {}
        }
    }
    // Prefer the stream aggregate when present; otherwise the persisted messages.
    Ok(if from_turn_end.is_empty() {
        from_message
    } else {
        from_turn_end
    })
}

/// Build a usage record from a Pi `message` object (shared by both channels).
/// Returns `None` when usage is absent or an empty object — no billable record.
fn extract(msg: Option<&Value>, current_model: &str) -> Option<MessageUsage> {
    let msg = msg?;
    let usage = match msg.get("usage") {
        Some(u) if u.as_object().is_some_and(|o| !o.is_empty()) => u,
        _ => return None, // empty/foreign usage -> no billable record
    };

    let g = |k: &str| usage.get(k).and_then(Value::as_u64);
    let nested = |outer: &str, inner: &str| {
        usage
            .get(outer)
            .and_then(|c| c.get(inner))
            .and_then(Value::as_u64)
    };
    let input = g("input").unwrap_or(0);
    let output = g("output").unwrap_or(0);
    let cache_read = g("cacheRead")
        .or_else(|| nested("cache", "read"))
        .unwrap_or(0);
    let cache_write = g("cacheWrite")
        .or_else(|| nested("cache", "write"))
        .unwrap_or(0);

    let provider_str = msg.get("provider").and_then(Value::as_str).unwrap_or("");
    let (namespace, provider) = route(provider_str);

    let model = msg
        .get("model")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| current_model.to_string());

    Some(MessageUsage {
        model,
        provider,
        namespace,
        input_uncached: input,
        cache_read,
        cache_write_5m: cache_write,
        cache_write_1h: 0,
        output,
        request_input_tokens: input + cache_read + cache_write,
        service_tier: None,
    })
}

/// Map Pi's `provider` (a backend/route name) to (price namespace, Provider label).
/// Only `openrouter` uses the OpenRouter table; everything else prices from LiteLLM.
fn route(provider: &str) -> (String, Provider) {
    match provider {
        "openrouter" => ("openrouter".to_string(), Provider::OpenRouter),
        "anthropic" => ("litellm".to_string(), Provider::Anthropic),
        "openai" | "openai-codex" => ("litellm".to_string(), Provider::OpenAI),
        other => ("litellm".to_string(), Provider::Other(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_turn_end_usage_and_routes_by_provider() {
        let u = parse(include_bytes!("../../tests/fixtures/pi-mini.jsonl")).unwrap();
        // message_update ignored; empty-usage turn skipped -> 3 records
        assert_eq!(u.len(), 3, "{u:?}");

        assert_eq!(u[0].namespace, "litellm");
        assert_eq!(u[0].provider, Provider::OpenAI);
        assert_eq!(u[0].model, "gpt-5.4");
        assert_eq!(u[0].input_uncached, 15024);
        assert_eq!(u[0].cache_read, 12288);
        assert_eq!(u[0].output, 251);

        assert_eq!(u[1].namespace, "openrouter");
        assert_eq!(u[1].provider, Provider::OpenRouter);
        assert_eq!(u[1].model, "tencent/hy3-preview"); // vendor-qualified key, verbatim
        assert_eq!(u[1].input_uncached, 6412);
        assert_eq!(u[1].cache_read, 5760);

        // anthropic turn with no `message.model` inherits the prior model_change
        assert_eq!(u[2].namespace, "litellm");
        assert_eq!(u[2].provider, Provider::Anthropic);
        assert_eq!(u[2].model, "claude-opus-4-5");
        assert_eq!(u[2].input_uncached, 100);
    }

    #[test]
    fn reads_session_file_message_rows() {
        // Persisted session files carry usage on `type:"message"` assistant rows
        // (role=user rows and empty-usage rows skipped) and contain no `turn_end`.
        let u = parse(include_bytes!("../../tests/fixtures/pi-session-mini.jsonl")).unwrap();
        assert_eq!(u.len(), 3, "{u:?}");

        assert_eq!(u[0].namespace, "litellm");
        assert_eq!(u[0].provider, Provider::OpenAI);
        assert_eq!(u[0].model, "gpt-5.5");
        assert_eq!(u[0].input_uncached, 6197);
        assert_eq!(u[0].cache_read, 64512);
        assert_eq!(u[0].output, 972);

        assert_eq!(u[1].namespace, "openrouter");
        assert_eq!(u[1].provider, Provider::OpenRouter);
        assert_eq!(u[1].model, "tencent/hy3-preview");
        assert_eq!(u[1].input_uncached, 6412);
        assert_eq!(u[1].cache_read, 5760);

        // anthropic message with no `message.model` inherits the prior model_change
        assert_eq!(u[2].namespace, "litellm");
        assert_eq!(u[2].provider, Provider::Anthropic);
        assert_eq!(u[2].model, "claude-opus-4-5");
        assert_eq!(u[2].input_uncached, 100);
    }

    #[test]
    fn prefers_turn_end_over_message_never_summing_both() {
        // The two channels are disjoint in practice; if a transcript ever carried
        // both, we return the turn_end bucket alone — not the sum — so the same
        // usage can't be counted twice.
        let mixed = br#"{"type":"turn_end","message":{"model":"gpt-5.5","provider":"openai","usage":{"input":1275,"output":5}}}
{"type":"message","message":{"role":"assistant","model":"gpt-5.5","provider":"openai","usage":{"input":1275,"output":5}}}"#;
        let u = parse(mixed).unwrap();
        assert_eq!(u.len(), 1, "{u:?}");
        assert_eq!(u[0].input_uncached, 1275);
    }
}
