//! Claude Code session JSONL -> Vec<MessageUsage>.
//! Reconciled with AgentsView internal/parser/claude.go (MIT, © 2026 Kenn Software LLC).

use crate::error::ObolError;
use crate::model::{MessageUsage, Provider};
use serde_json::Value;

#[derive(Debug, Default)]
pub struct ClaudeParse {
    pub usages: Vec<MessageUsage>,
    pub malformed_lines: usize,
}

/// Parse a Claude session file. Within-file dedup: assistant entries sharing a
/// `message.id` collapse to one, keeping the LAST entry's usage (streaming
/// snapshots overwrite; never sum).
pub fn parse(bytes: &[u8]) -> Result<ClaudeParse, ObolError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ObolError::MalformedTranscript {
        line: 0,
        msg: e.to_string(),
    })?;

    // Preserve first-seen order of message ids; overwrite usage on each new line.
    let mut order: Vec<String> = Vec::new();
    let mut by_id: std::collections::HashMap<String, MessageUsage> =
        std::collections::HashMap::new();
    let mut out = ClaudeParse::default();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                out.malformed_lines += 1;
                continue;
            }
        };
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if v.get("isCompactSummary").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let msg = match v.get("message") {
            Some(m) => m,
            None => continue,
        };
        let usage = match msg.get("usage") {
            Some(u) if u.is_object() => u,
            _ => continue,
        };

        let id = msg
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        // Token math lives in the shared Anthropic normalizer (Claude Code
        // embeds the Anthropic usage object near-verbatim). The envelope
        // navigation + streaming dedup below is this dialect's own job.
        let t = super::provider::anthropic::normalize(usage);
        let mu = MessageUsage {
            model: msg
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            provider: Provider::Anthropic,
            namespace: "litellm".into(),
            input_uncached: t.input_uncached,
            cache_read: t.cache_read,
            cache_write_5m: t.cache_write_5m,
            cache_write_1h: t.cache_write_1h,
            output: t.output,
            request_input_tokens: t.input_uncached
                + t.cache_read
                + t.cache_write_5m
                + t.cache_write_1h,
            service_tier: usage
                .get("service_tier")
                .and_then(Value::as_str)
                .map(String::from),
        };

        let key = if id.is_empty() {
            format!("__anon_{}", order.len())
        } else {
            id
        };
        if !by_id.contains_key(&key) {
            order.push(key.clone());
        }
        by_id.insert(key, mu); // last-write-wins
    }

    out.usages = order.into_iter().filter_map(|k| by_id.remove(&k)).collect();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedups_streaming_and_keeps_last_usage() {
        let bytes = include_bytes!("../../tests/fixtures/claude-mini.jsonl");
        let p = parse(bytes).unwrap();
        // msg_a (collapsed) + msg_b = 2 usages
        assert_eq!(p.usages.len(), 2, "usages: {:?}", p.usages);
        assert_eq!(p.malformed_lines, 1);

        let a = &p.usages[0];
        assert_eq!(a.output, 9, "should keep LAST msg_a output, not sum");
        assert_eq!(a.input_uncached, 12);
        assert_eq!(a.cache_read, 120);
        assert_eq!(a.cache_write_5m, 60);
        assert_eq!(a.request_input_tokens, 12 + 120 + 60);

        let b = &p.usages[1];
        assert_eq!(b.output, 7);
        assert_eq!(b.input_uncached, 0); // present-and-zero, still a real record
    }
}
