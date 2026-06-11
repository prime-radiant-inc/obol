//! Copilot CLI events.jsonl -> Vec<MessageUsage>.
//! Reconciled with AgentsView internal/parser/copilot.go (MIT, © 2026 Kenn Software LLC).
//! Authoritative per-model usage is the `session.shutdown` aggregate
//! (`data.modelMetrics.<model>.usage`). `inputTokens` is a total -> uncached is
//! input - cacheRead - cacheWrite. `reasoningTokens` billed as output. No shutdown
//! event -> no usage (documented limitation).

use crate::error::ObolError;
use crate::model::{MessageUsage, Provider};
use serde_json::Value;

pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ObolError::MalformedTranscript {
        line: 0,
        msg: e.to_string(),
    })?;
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
        if v.get("type").and_then(Value::as_str) != Some("session.shutdown") {
            continue;
        }
        let metrics = match v.pointer("/data/modelMetrics").and_then(Value::as_object) {
            Some(m) => m,
            None => continue,
        };
        for (model, mv) in metrics {
            let usage = match mv.get("usage") {
                Some(u) if u.is_object() => u,
                _ => continue,
            };
            let g = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
            let total_input = g("inputTokens");
            let cache_read = g("cacheReadTokens");
            let cache_write = g("cacheWriteTokens");
            out.push(MessageUsage {
                model: model.clone(),
                provider: route_model(model),
                namespace: "litellm".into(),
                input_uncached: total_input
                    .saturating_sub(cache_read)
                    .saturating_sub(cache_write),
                cache_read,
                cache_write_5m: cache_write,
                cache_write_1h: 0,
                output: g("outputTokens") + g("reasoningTokens"),
                request_input_tokens: total_input,
                service_tier: None,
                native_cost_usd: None,
            });
        }
    }
    Ok(out)
}

fn route_model(model: &str) -> Provider {
    let m = model.to_ascii_lowercase();
    if m.contains("claude") {
        Provider::Anthropic
    } else if m.contains("gpt") || m.contains("o1") || m.contains("o3") {
        Provider::OpenAI
    } else if m.contains("gemini") {
        Provider::Other("google".into())
    } else {
        Provider::Other("copilot".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provider;

    #[test]
    fn reads_shutdown_aggregate_and_subtracts_cache() {
        let u = parse(include_bytes!("../../tests/fixtures/copilot-mini.jsonl")).unwrap();
        assert_eq!(u.len(), 1, "{u:?}");
        assert_eq!(u[0].model, "claude-sonnet-4-5");
        assert_eq!(u[0].provider, Provider::Anthropic);
        assert_eq!(u[0].input_uncached, 2830); // 52030 - 48000 - 1200
        assert_eq!(u[0].cache_read, 48000);
        assert_eq!(u[0].cache_write_5m, 1200);
        assert_eq!(u[0].output, 3140); // 3100 + 40 reasoning
        assert_eq!(u[0].request_input_tokens, 52030);
    }
}
