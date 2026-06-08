//! Kimi Code wire.jsonl -> Vec<MessageUsage>.
//! Targets quorum's `usage.record` rows (full input/cache fidelity + model), not
//! agentsview's lower-fidelity StatusUpdate path. Prefer `usageScope:"turn"` rows;
//! fall back to the latest `session` row. Never mix turn + session (double-counts).

use crate::error::ObolError;
use crate::model::{MessageUsage, Provider};
use serde_json::Value;

pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ObolError::MalformedTranscript {
        line: 0,
        msg: e.to_string(),
    })?;
    let mut turns: Vec<Value> = Vec::new();
    let mut sessions: Vec<Value> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(Value::as_str) != Some("usage.record") {
            continue;
        }
        match v.get("usageScope").and_then(Value::as_str) {
            Some("turn") => turns.push(v),
            Some("session") => sessions.push(v),
            _ => {}
        }
    }
    let selected: Vec<Value> = if !turns.is_empty() {
        turns
    } else {
        match sessions
            .into_iter()
            .max_by_key(|r| r.get("time").and_then(Value::as_i64).unwrap_or(i64::MIN))
        {
            Some(latest) => vec![latest],
            None => Vec::new(),
        }
    };
    let mut out = Vec::new();
    for row in &selected {
        let usage = match row.get("usage") {
            Some(u) if u.is_object() => u,
            _ => continue,
        };
        let g = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
        let input = g("inputOther");
        let cache_read = g("inputCacheRead");
        let cache_create = g("inputCacheCreation");
        out.push(MessageUsage {
            model: row
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            provider: Provider::Other("moonshot".into()),
            namespace: "litellm".into(),
            input_uncached: input,
            cache_read,
            cache_write_5m: cache_create,
            cache_write_1h: 0,
            output: g("output"),
            request_input_tokens: input + cache_read + cache_create,
            service_tier: None,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_turn_rows_over_session() {
        let u = parse(include_bytes!("../../tests/fixtures/kimi-mini.jsonl")).unwrap();
        assert_eq!(u.len(), 2, "session row must be ignored when turns exist: {u:?}");
        assert_eq!(u[0].model, "kimi-for-coding");
        assert_eq!(u[0].input_uncached, 10);
        assert_eq!(u[0].cache_read, 20);
        assert_eq!(u[0].cache_write_5m, 30);
        assert_eq!(u[0].output, 40);
        assert_eq!(u[0].request_input_tokens, 60);
    }
}
