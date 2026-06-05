//! Pi (`pi --mode json`) transcript -> Vec<MessageUsage>.
//! Reconciled with AgentsView internal/parser/pi.go (MIT, © 2026 Kenn Software LLC).

use crate::error::ObolError;
use crate::model::{MessageUsage, Provider};
use serde_json::Value;

pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| ObolError::MalformedTranscript { line: 0, msg: e.to_string() })?;
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
        // Usage lives on turn_end; the streaming message_update deltas are ignored.
        if v.get("type").and_then(Value::as_str) != Some("turn_end") {
            continue;
        }
        let msg = match v.get("message") {
            Some(m) => m,
            None => continue,
        };
        let usage = match msg.get("usage") {
            Some(u) if u.as_object().map_or(false, |o| !o.is_empty()) => u,
            _ => continue, // empty/foreign usage -> no billable record
        };

        let g = |k: &str| usage.get(k).and_then(Value::as_u64);
        let nested = |outer: &str, inner: &str| {
            usage.get(outer).and_then(|c| c.get(inner)).and_then(Value::as_u64)
        };
        let input = g("input").unwrap_or(0);
        let output = g("output").unwrap_or(0);
        let cache_read = g("cacheRead").or_else(|| nested("cache", "read")).unwrap_or(0);
        let cache_write = g("cacheWrite").or_else(|| nested("cache", "write")).unwrap_or(0);

        let provider_str = msg.get("provider").and_then(Value::as_str).unwrap_or("");
        let (namespace, provider) = route(provider_str);

        out.push(MessageUsage {
            model: msg.get("model").and_then(Value::as_str).unwrap_or("").to_string(),
            provider,
            namespace,
            input_uncached: input,
            cache_read,
            cache_write_5m: cache_write,
            cache_write_1h: 0,
            output,
            request_input_tokens: input + cache_read + cache_write,
            service_tier: None,
        });
    }
    Ok(out)
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
        // message_update ignored; empty-usage turn skipped -> 2 records
        assert_eq!(u.len(), 2, "{u:?}");

        assert_eq!(u[0].namespace, "litellm");
        assert_eq!(u[0].provider, Provider::OpenAI);
        assert_eq!(u[0].model, "gpt-5.4");
        assert_eq!(u[0].input_uncached, 15024);
        assert_eq!(u[0].cache_read, 12288);
        assert_eq!(u[0].output, 251);

        assert_eq!(u[1].namespace, "openrouter");
        assert_eq!(u[1].provider, Provider::OpenRouter);
        assert_eq!(u[1].model, "tencent/hy3-preview");
        assert_eq!(u[1].input_uncached, 6412);
        assert_eq!(u[1].cache_read, 5760);
    }
}
