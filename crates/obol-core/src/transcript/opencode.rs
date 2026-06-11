//! OpenCode `opencode export` JSON -> Vec<MessageUsage>.
//! Reconciled with AgentsView internal/parser/opencode.go (MIT, © 2026 Kenn Software LLC).
//! Single document {info, messages:[...]}; per-assistant usage on the message or its
//! `step-finish` part. `tokens.reasoning` is a separate additive bucket billed as output.
//! Model = `modelID` (bare); provider routed by `providerID`.

use crate::error::ObolError;
use crate::model::{MessageUsage, Provider};
use serde_json::Value;

pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError> {
    let doc: Value = serde_json::from_slice(bytes).map_err(|e| ObolError::MalformedTranscript {
        line: 0,
        msg: e.to_string(),
    })?;
    let messages = match doc.get("messages").and_then(Value::as_array) {
        Some(m) => m,
        None => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    for msg in messages {
        if msg.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let tok = msg
            .get("tokens")
            .filter(|t| t.is_object())
            .or_else(|| step_finish_tokens(msg));
        let tok = match tok {
            Some(t) => t,
            None => continue,
        };
        let g = |k: &str| tok.get(k).and_then(Value::as_u64).unwrap_or(0);
        let input = g("input");
        let cache_read = tok
            .pointer("/cache/read")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cache_write = tok
            .pointer("/cache/write")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let output = g("output") + g("reasoning");
        let model = msg
            .get("modelID")
            .and_then(Value::as_str)
            .or_else(|| msg.pointer("/model/modelID").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        let provider_id = msg.get("providerID").and_then(Value::as_str).unwrap_or("");
        out.push(MessageUsage {
            model,
            provider: route_provider(provider_id),
            namespace: "litellm".into(),
            input_uncached: input,
            cache_read,
            cache_write_5m: cache_write,
            cache_write_1h: 0,
            output,
            request_input_tokens: input + cache_read + cache_write,
            service_tier: None,
            native_cost_usd: None,
        });
    }
    Ok(out)
}

fn step_finish_tokens(msg: &Value) -> Option<&Value> {
    msg.get("parts")?
        .as_array()?
        .iter()
        .find(|p| p.get("type").and_then(Value::as_str) == Some("step-finish"))
        .and_then(|p| p.get("tokens"))
        .filter(|t| t.is_object())
}

fn route_provider(provider_id: &str) -> Provider {
    match provider_id {
        "anthropic" => Provider::Anthropic,
        "openai" => Provider::OpenAI,
        "" => Provider::Other("opencode".into()),
        other => Provider::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provider;

    #[test]
    fn reads_message_and_step_finish_tokens() {
        let u = parse(include_bytes!("../../tests/fixtures/opencode-mini.json")).unwrap();
        assert_eq!(u.len(), 2, "{u:?}");
        assert_eq!(u[0].model, "gpt-5.5");
        assert_eq!(u[0].provider, Provider::OpenAI);
        assert_eq!(u[0].input_uncached, 7035);
        assert_eq!(u[0].output, 12);
        // fallback to the step-finish part; reasoning folds into output
        assert_eq!(u[1].input_uncached, 100);
        assert_eq!(u[1].cache_read, 50);
        assert_eq!(u[1].cache_write_5m, 7); // proves the nested /cache/write read
        assert_eq!(u[1].output, 7); // 5 + 2 reasoning
        assert_eq!(u[1].request_input_tokens, 157); // 100 + 50 + 7
    }
}
