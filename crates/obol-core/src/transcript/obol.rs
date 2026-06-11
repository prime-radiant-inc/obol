//! The `obol` house dialect: a provider-tagged raw-usage sidecar (`usage.jsonl`)
//! that in-house harnesses emit. One row per billable LLM call:
//!
//! ```json
//! {"type":"obol.usage","v":"2026-06-08","provider":"anthropic","model":"…",
//!  "service_tier":"standard","usage":{ …the SDK's usage object, verbatim… }}
//! ```
//!
//! The producer tags (provider / model / tier) and copies the raw `usage`
//! through — no arithmetic. obol dispatches on `provider` to a shared
//! normalizer (`provider::{anthropic,openai}`) and derives the rest. The
//! interpretation — the part naive summers get wrong — lives here, once.

use super::provider::{self, ProviderTokens};
use crate::error::ObolError;
use crate::model::{MessageUsage, Provider};
use serde_json::Value;

/// Schema versions this build understands. `v` is an ISO date, matched as an
/// opaque string (no date arithmetic). An unrecognized `v` is a loud error, not
/// a silent mis-parse: a newer schema may mean fields obol can't interpret.
const SCHEMA_VERSIONS: &[&str] = &["2026-06-08"];

pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ObolError::MalformedTranscript {
        line: 0,
        msg: e.to_string(),
    })?;

    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Non-JSON lines (e.g. a truncated trailing write) are skipped, like the
        // other dialects. Only well-formed JSON objects of the right type below.
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(Value::as_str) != Some("obol.usage") {
            continue;
        }
        out.push(parse_row(&v, i + 1)?);
    }
    Ok(out)
}

fn parse_row(v: &Value, line: usize) -> Result<MessageUsage, ObolError> {
    let err = |msg: String| ObolError::MalformedTranscript { line, msg };

    let ver = v.get("v").and_then(Value::as_str).unwrap_or("");
    if !SCHEMA_VERSIONS.contains(&ver) {
        return Err(err(format!("unknown obol.usage schema version {ver:?}")));
    }

    let provider_tag = v
        .get("provider")
        .and_then(Value::as_str)
        .ok_or_else(|| err("obol.usage row missing `provider`".into()))?;
    let usage = v
        .get("usage")
        .filter(|u| u.is_object())
        .ok_or_else(|| err("obol.usage row missing `usage` object".into()))?;

    let (provider, tokens): (Provider, ProviderTokens) = match provider_tag {
        "anthropic" => (Provider::Anthropic, provider::anthropic::normalize(usage)),
        "openai" => (Provider::OpenAI, provider::openai::normalize(usage)),
        other => {
            return Err(err(format!(
                "no usage normalizer for provider {other:?} (supported: anthropic, openai)"
            )))
        }
    };

    let request_input_tokens =
        tokens.input_uncached + tokens.cache_read + tokens.cache_write_5m + tokens.cache_write_1h;

    Ok(MessageUsage {
        model: v
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        provider,
        namespace: "litellm".into(),
        input_uncached: tokens.input_uncached,
        cache_read: tokens.cache_read,
        cache_write_5m: tokens.cache_write_5m,
        cache_write_1h: tokens.cache_write_1h,
        output: tokens.output,
        request_input_tokens,
        service_tier: v
            .get("service_tier")
            .and_then(Value::as_str)
            .map(String::from),
        native_cost_usd: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anthropic_line() -> &'static str {
        r#"{"type":"obol.usage","v":"2026-06-08","provider":"anthropic","model":"claude-opus-4-8","service_tier":"standard","usage":{"input_tokens":12,"cache_read_input_tokens":120,"cache_creation_input_tokens":60,"cache_creation":{"ephemeral_5m_input_tokens":50,"ephemeral_1h_input_tokens":10},"output_tokens":9}}"#
    }
    fn openai_line() -> &'static str {
        r#"{"type":"obol.usage","v":"2026-06-08","provider":"openai","model":"gpt-5.5","usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":40},"output_tokens":20,"output_tokens_details":{"reasoning_tokens":5}}}"#
    }

    #[test]
    fn parses_anthropic_and_openai_rows() {
        let bytes = format!("{}\n{}\n", anthropic_line(), openai_line());
        let usages = parse(bytes.as_bytes()).unwrap();
        assert_eq!(
            usages,
            vec![
                MessageUsage {
                    model: "claude-opus-4-8".into(),
                    provider: Provider::Anthropic,
                    namespace: "litellm".into(),
                    input_uncached: 12,
                    cache_read: 120,
                    cache_write_5m: 50,
                    cache_write_1h: 10,
                    output: 9,
                    request_input_tokens: 192,
                    service_tier: Some("standard".into()),
                    native_cost_usd: None,
                },
                MessageUsage {
                    model: "gpt-5.5".into(),
                    provider: Provider::OpenAI,
                    namespace: "litellm".into(),
                    input_uncached: 60,
                    cache_read: 40,
                    cache_write_5m: 0,
                    cache_write_1h: 0,
                    output: 25,
                    request_input_tokens: 100,
                    service_tier: None,
                    native_cost_usd: None,
                },
            ]
        );
    }

    #[test]
    fn unknown_schema_version_is_a_loud_error() {
        let line = r#"{"type":"obol.usage","v":"2099-12-31","provider":"anthropic","model":"x","usage":{"input_tokens":1,"output_tokens":1}}"#;
        let e = parse(line.as_bytes()).unwrap_err();
        assert!(
            matches!(e, ObolError::MalformedTranscript { line: 1, .. }),
            "got {e:?}"
        );
    }

    #[test]
    fn missing_usage_object_is_a_loud_error() {
        let line = r#"{"type":"obol.usage","v":"2026-06-08","provider":"anthropic","model":"x"}"#;
        assert!(parse(line.as_bytes()).is_err());
    }

    #[test]
    fn unknown_provider_is_a_loud_error() {
        let line = r#"{"type":"obol.usage","v":"2026-06-08","provider":"mystery","model":"x","usage":{"input_tokens":1}}"#;
        assert!(parse(line.as_bytes()).is_err());
    }

    #[test]
    fn skips_blank_and_non_obol_lines_but_keeps_valid_rows() {
        let bytes = format!(
            "\nnot json\n{{\"type\":\"something_else\"}}\n{}\n",
            anthropic_line()
        );
        let usages = parse(bytes.as_bytes()).unwrap();
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].model, "claude-opus-4-8");
    }

    #[test]
    fn missing_model_yields_empty_model_for_loud_unpriced() {
        let line = r#"{"type":"obol.usage","v":"2026-06-08","provider":"anthropic","usage":{"input_tokens":1,"output_tokens":1}}"#;
        let usages = parse(line.as_bytes()).unwrap();
        assert_eq!(usages[0].model, "");
    }

    // The integrity guarantee: the SAME Anthropic usage object, once embedded in
    // a Claude Code assistant line and once in an obol.usage row, produces the
    // SAME token buckets — because both route through provider::anthropic. The
    // math lives in one implementation.
    #[test]
    fn anthropic_buckets_match_the_claude_dialect() {
        let usage = r#"{"input_tokens":12,"cache_read_input_tokens":120,"cache_creation_input_tokens":60,"cache_creation":{"ephemeral_5m_input_tokens":50,"ephemeral_1h_input_tokens":10},"output_tokens":9}"#;
        let claude_line = format!(
            r#"{{"type":"assistant","message":{{"id":"m1","model":"m","usage":{usage}}}}}"#
        );
        let obol_line = format!(
            r#"{{"type":"obol.usage","v":"2026-06-08","provider":"anthropic","model":"m","usage":{usage}}}"#
        );
        let c = crate::transcript::claude::parse(claude_line.as_bytes())
            .unwrap()
            .usages;
        let o = parse(obol_line.as_bytes()).unwrap();
        let f = |u: &MessageUsage| {
            (
                u.input_uncached,
                u.cache_read,
                u.cache_write_5m,
                u.cache_write_1h,
                u.output,
                u.request_input_tokens,
            )
        };
        assert_eq!(f(&c[0]), f(&o[0]));
    }
}
