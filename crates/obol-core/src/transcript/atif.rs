//! ATIF (Agent Trajectory Interchange Format) `trajectory.json` -> Vec<MessageUsage>.
//!
//! ATIF is superpowers-evals' canonical, agent-agnostic transcript: every agent's
//! raw session log is normalized to a single `Trajectory` JSON document, so obol
//! prices ONE stable input instead of re-parsing each agent's native log via the
//! per-agent dialects. The shape (ATIF v1.7):
//!
//! ```json
//! { "schema_version": "ATIF-v1.7",
//!   "agent": { "name": "claude", "version": "…", "model_name": "claude-opus-4-8" },
//!   "steps": [ { "step_id": "…", "source": "agent", "model_name": "…",
//!                "metrics": { "prompt_tokens": 12, "completion_tokens": 9,
//!                             "cached_tokens": 120, "cost_usd": 0.01 },
//!                "extra": { "provider": "anthropic", "cache_write": 60 } } ],
//!   "final_metrics": { "total_prompt_tokens": …, "total_completion_tokens": …,
//!                      "total_cost_usd": …, "extra": { "total_cached_tokens": … } } }
//! ```
//!
//! Token buckets in a normalized trajectory are DISJOINT — the evals normalizers
//! have already split cache/uncached, so this dialect maps each bucket VERBATIM
//! and must NOT re-run the provider normalizers' cache-subtraction/splitting:
//!   `prompt_tokens`     -> input_uncached  (UNCACHED input; never add cached in)
//!   `cached_tokens`     -> cache_read
//!   `extra.cache_write` -> cache_write_5m
//!   `completion_tokens` -> output
//!
//! Embedded cost is ground truth: a step's `metrics.cost_usd` (or, for a
//! trajectory carrying only `final_metrics`, `final_metrics.total_cost_usd`) is
//! used verbatim via `MessageUsage::native_cost_usd` — the cost engine then skips
//! list-price math for that record. A trajectory with no usage at all yields no
//! records, so the estimate is empty/zero with no fabricated cost.

use crate::error::ObolError;
use crate::model::{MessageUsage, Provider};
use serde_json::Value;

pub fn parse(bytes: &[u8]) -> Result<Vec<MessageUsage>, ObolError> {
    let err = |msg: String| ObolError::MalformedTranscript { line: 0, msg };

    let doc: Value = serde_json::from_slice(bytes)
        .map_err(|e| err(format!("trajectory.json is not valid JSON: {e}")))?;
    if !doc.is_object() {
        return Err(err("ATIF trajectory must be a JSON object".into()));
    }

    // `agent.model_name` is the fallback model for steps (and the only model for a
    // `final_metrics`-only trajectory). `agent.name` is informational only.
    let agent_model = doc
        .pointer("/agent/model_name")
        .and_then(Value::as_str)
        .unwrap_or("");

    let mut out = Vec::new();

    if let Some(steps) = doc.get("steps").and_then(Value::as_array) {
        for step in steps {
            if let Some(rec) = step_usage(step, agent_model) {
                out.push(rec);
            }
        }
    }

    // A trajectory that carried per-step metrics is fully described by them; the
    // `final_metrics` block is a redundant rollup in that case (summing both would
    // double-count). Only fall back to `final_metrics` when no step produced a
    // billable record — e.g. a normalizer that only emitted totals.
    if out.is_empty() {
        if let Some(rec) = final_metrics_usage(&doc, agent_model) {
            out.push(rec);
        }
    }

    Ok(out)
}

/// Build a usage record from one ATIF step. Returns `None` when the step carries
/// no usage at all (no token buckets and no cost) — not a billable record.
fn step_usage(step: &Value, agent_model: &str) -> Option<MessageUsage> {
    let metrics = step.get("metrics");
    let extra = step.get("extra");

    let m = |k: &str| metrics.and_then(|v| v.get(k)).and_then(Value::as_u64);
    let input_uncached = m("prompt_tokens").unwrap_or(0);
    let cache_read = m("cached_tokens").unwrap_or(0);
    let output = m("completion_tokens").unwrap_or(0);
    let cache_write = extra
        .and_then(|v| v.get("cache_write"))
        .and_then(Value::as_u64)
        .unwrap_or(0);

    let native_cost_usd = metrics
        .and_then(|v| v.get("cost_usd"))
        .and_then(Value::as_f64)
        .filter(|c| c.is_finite() && *c >= 0.0);

    // No tokens AND no cost -> nothing to price for this step.
    if input_uncached == 0
        && cache_read == 0
        && output == 0
        && cache_write == 0
        && native_cost_usd.is_none()
    {
        return None;
    }

    let model = step
        .get("model_name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(agent_model)
        .to_string();

    let provider_tag = extra
        .and_then(|v| v.get("provider"))
        .and_then(Value::as_str);
    let (namespace, provider) = route(provider_tag, &model);

    Some(MessageUsage {
        model,
        provider,
        namespace,
        input_uncached,
        cache_read,
        cache_write_5m: cache_write,
        cache_write_1h: 0,
        output,
        request_input_tokens: input_uncached + cache_read + cache_write,
        service_tier: None,
        native_cost_usd,
    })
}

/// Build a single usage record from `final_metrics` for a totals-only trajectory.
/// Returns `None` when there is no usage to price.
fn final_metrics_usage(doc: &Value, agent_model: &str) -> Option<MessageUsage> {
    let fm = doc.get("final_metrics")?;

    let g = |k: &str| fm.get(k).and_then(Value::as_u64).unwrap_or(0);
    let input_uncached = g("total_prompt_tokens");
    let output = g("total_completion_tokens");
    let cache_read = fm
        .pointer("/extra/total_cached_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    let native_cost_usd = fm
        .get("total_cost_usd")
        .and_then(Value::as_f64)
        .filter(|c| c.is_finite() && *c >= 0.0);

    if input_uncached == 0 && cache_read == 0 && output == 0 && native_cost_usd.is_none() {
        return None;
    }

    let provider_tag = fm.pointer("/extra/provider").and_then(Value::as_str);
    let (namespace, provider) = route(provider_tag, agent_model);

    Some(MessageUsage {
        model: agent_model.to_string(),
        provider,
        namespace,
        input_uncached,
        cache_read,
        cache_write_5m: 0,
        cache_write_1h: 0,
        output,
        request_input_tokens: input_uncached + cache_read,
        service_tier: None,
        native_cost_usd,
    })
}

/// Resolve (price namespace, Provider label) from an explicit ATIF provider tag,
/// falling back to inference from the model string. Only `openrouter` prices from
/// the OpenRouter table; everything else prices from LiteLLM (provider is a label).
fn route(provider_tag: Option<&str>, model: &str) -> (String, Provider) {
    match provider_tag {
        Some("openrouter") => ("openrouter".to_string(), Provider::OpenRouter),
        Some("anthropic") => ("litellm".to_string(), Provider::Anthropic),
        Some("openai") | Some("openai-codex") => ("litellm".to_string(), Provider::OpenAI),
        Some(other) if !other.is_empty() => {
            ("litellm".to_string(), Provider::Other(other.to_string()))
        }
        // No explicit provider: infer from the model string, as the agent dialects
        // tag a fixed provider per family. Pricing keys off namespace+model, so the
        // Provider label here is informational; an unknown family is `Other("")`.
        _ => ("litellm".to_string(), infer_provider(model)),
    }
}

/// Best-effort provider label from a model string (display only — pricing keys off
/// the verbatim model in the litellm namespace).
fn infer_provider(model: &str) -> Provider {
    let m = model.to_ascii_lowercase();
    if m.starts_with("claude") {
        Provider::Anthropic
    } else if m.starts_with("gpt")
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
    {
        Provider::OpenAI
    } else if m.starts_with("gemini") {
        Provider::Other("google".into())
    } else {
        // Unknown family (or empty model): label is informational; an empty/unknown
        // model is surfaced as unpriced by the cost engine, never fabricated.
        Provider::Other(String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Disjoint buckets are mapped verbatim — prompt_tokens is uncached input and
    // must NOT have cached_tokens folded in, and cache_write comes from extra.
    #[test]
    fn maps_disjoint_buckets_verbatim() {
        let line = r#"{
          "schema_version":"ATIF-v1.7",
          "agent":{"name":"claude","model_name":"claude-opus-4-8"},
          "steps":[
            {"step_id":"s1","source":"agent",
             "metrics":{"prompt_tokens":12,"cached_tokens":120,"completion_tokens":9},
             "extra":{"provider":"anthropic","cache_write":60}}
          ]
        }"#;
        let u = parse(line.as_bytes()).unwrap();
        assert_eq!(u.len(), 1, "{u:?}");
        assert_eq!(u[0].model, "claude-opus-4-8");
        assert_eq!(u[0].provider, Provider::Anthropic);
        assert_eq!(u[0].namespace, "litellm");
        assert_eq!(u[0].input_uncached, 12); // verbatim — cached NOT added in
        assert_eq!(u[0].cache_read, 120);
        assert_eq!(u[0].cache_write_5m, 60);
        assert_eq!(u[0].cache_write_1h, 0);
        assert_eq!(u[0].output, 9);
        assert_eq!(u[0].request_input_tokens, 12 + 120 + 60);
        assert_eq!(u[0].native_cost_usd, None); // no cost_usd -> price by rates
    }

    // A step's `cost_usd` is ground truth: surfaced as native_cost_usd, used
    // verbatim by the cost engine (no re-pricing by rates).
    #[test]
    fn embedded_step_cost_is_native_cost() {
        let line = r#"{
          "schema_version":"ATIF-v1.7",
          "agent":{"name":"codex"},
          "steps":[
            {"step_id":"s1","source":"agent","model_name":"gpt-5.5",
             "metrics":{"prompt_tokens":100,"completion_tokens":20,"cost_usd":1.69},
             "extra":{"provider":"openai"}}
          ]
        }"#;
        let u = parse(line.as_bytes()).unwrap();
        assert_eq!(u.len(), 1, "{u:?}");
        assert_eq!(u[0].model, "gpt-5.5");
        assert_eq!(u[0].provider, Provider::OpenAI);
        assert_eq!(u[0].native_cost_usd, Some(1.69));
    }

    // Steps with no usage (system/user turns the normalizer emits with no metrics)
    // produce no billable record.
    #[test]
    fn steps_without_usage_are_skipped() {
        let line = r#"{
          "schema_version":"ATIF-v1.7",
          "agent":{"name":"claude","model_name":"claude-opus-4-8"},
          "steps":[
            {"step_id":"s0","source":"system"},
            {"step_id":"s1","source":"user"},
            {"step_id":"s2","source":"agent",
             "metrics":{"prompt_tokens":5,"completion_tokens":3}}
          ]
        }"#;
        let u = parse(line.as_bytes()).unwrap();
        assert_eq!(u.len(), 1, "only the agent step has usage: {u:?}");
        assert_eq!(u[0].input_uncached, 5);
        assert_eq!(u[0].output, 3);
        // step has no model_name -> inherits agent.model_name
        assert_eq!(u[0].model, "claude-opus-4-8");
    }

    // A step without an explicit provider infers the label from the model family.
    #[test]
    fn infers_provider_from_model_when_absent() {
        let line = r#"{
          "schema_version":"ATIF-v1.7",
          "agent":{"name":"claude"},
          "steps":[
            {"step_id":"s1","source":"agent","model_name":"claude-opus-4-8",
             "metrics":{"prompt_tokens":1,"completion_tokens":1}}
          ]
        }"#;
        let u = parse(line.as_bytes()).unwrap();
        assert_eq!(u[0].provider, Provider::Anthropic);
        assert_eq!(u[0].namespace, "litellm");
    }

    // openrouter routes to the openrouter namespace, like the pi dialect.
    #[test]
    fn openrouter_provider_routes_to_openrouter_namespace() {
        let line = r#"{
          "schema_version":"ATIF-v1.7",
          "agent":{"name":"pi"},
          "steps":[
            {"step_id":"s1","source":"agent","model_name":"tencent/hy3-preview",
             "metrics":{"prompt_tokens":10,"completion_tokens":2},
             "extra":{"provider":"openrouter"}}
          ]
        }"#;
        let u = parse(line.as_bytes()).unwrap();
        assert_eq!(u[0].namespace, "openrouter");
        assert_eq!(u[0].provider, Provider::OpenRouter);
        assert_eq!(u[0].model, "tencent/hy3-preview");
    }

    // A trajectory carrying ONLY final_metrics (no per-step usage) is priced from
    // the totals, using agent.model_name as the model.
    #[test]
    fn final_metrics_only_trajectory() {
        let line = r#"{
          "schema_version":"ATIF-v1.7",
          "agent":{"name":"claude","model_name":"claude-opus-4-8"},
          "steps":[{"step_id":"s0","source":"system"}],
          "final_metrics":{"total_prompt_tokens":100,"total_completion_tokens":50,
                           "extra":{"total_cached_tokens":200}}
        }"#;
        let u = parse(line.as_bytes()).unwrap();
        assert_eq!(u.len(), 1, "{u:?}");
        assert_eq!(u[0].model, "claude-opus-4-8");
        assert_eq!(u[0].input_uncached, 100);
        assert_eq!(u[0].output, 50);
        assert_eq!(u[0].cache_read, 200);
        assert_eq!(u[0].request_input_tokens, 300);
        assert_eq!(u[0].native_cost_usd, None);
    }

    // final_metrics.total_cost_usd is ground truth for a totals-only trajectory.
    #[test]
    fn final_metrics_total_cost_is_native_cost() {
        let line = r#"{
          "schema_version":"ATIF-v1.7",
          "agent":{"name":"claude","model_name":"claude-opus-4-8"},
          "final_metrics":{"total_prompt_tokens":100,"total_completion_tokens":50,
                           "total_cost_usd":2.5}
        }"#;
        let u = parse(line.as_bytes()).unwrap();
        assert_eq!(u.len(), 1, "{u:?}");
        assert_eq!(u[0].native_cost_usd, Some(2.5));
    }

    // When steps carry usage, final_metrics is a redundant rollup and must NOT be
    // added — otherwise the same usage is counted twice.
    #[test]
    fn step_usage_wins_over_final_metrics_no_double_count() {
        let line = r#"{
          "schema_version":"ATIF-v1.7",
          "agent":{"name":"claude","model_name":"claude-opus-4-8"},
          "steps":[
            {"step_id":"s1","source":"agent",
             "metrics":{"prompt_tokens":10,"completion_tokens":5}}
          ],
          "final_metrics":{"total_prompt_tokens":10,"total_completion_tokens":5}
        }"#;
        let u = parse(line.as_bytes()).unwrap();
        assert_eq!(
            u.len(),
            1,
            "final_metrics must not add a second record: {u:?}"
        );
        assert_eq!(u[0].input_uncached, 10);
        assert_eq!(u[0].output, 5);
    }

    // A trajectory with no usage at all (e.g. an antigravity run obol can't price)
    // yields no records -> the cost engine fabricates nothing.
    #[test]
    fn empty_trajectory_yields_no_records() {
        let line = r#"{
          "schema_version":"ATIF-v1.7",
          "agent":{"name":"antigravity"},
          "steps":[{"step_id":"s0","source":"system"},{"step_id":"s1","source":"user"}]
        }"#;
        let u = parse(line.as_bytes()).unwrap();
        assert!(u.is_empty(), "no usage -> no records: {u:?}");
    }

    // A step with no model_name and no agent.model_name yields the empty model,
    // which the cost engine surfaces as unpriced (never a fabricated cost).
    #[test]
    fn missing_model_yields_empty_model_for_loud_unpriced() {
        let line = r#"{
          "schema_version":"ATIF-v1.7",
          "agent":{"name":"mystery"},
          "steps":[
            {"step_id":"s1","source":"agent",
             "metrics":{"prompt_tokens":1,"completion_tokens":1}}
          ]
        }"#;
        let u = parse(line.as_bytes()).unwrap();
        assert_eq!(u[0].model, "");
    }

    #[test]
    fn non_object_document_is_a_loud_error() {
        assert!(parse(b"[]").is_err());
        assert!(parse(b"not json").is_err());
    }

    // The fixture covers the priced cases end to end.
    #[test]
    fn parses_the_fixture_trajectory() {
        let u = parse(include_bytes!("../../tests/fixtures/atif-mini.json")).unwrap();
        // 3 agent steps with usage; the system/user steps are skipped.
        assert_eq!(u.len(), 3, "{u:?}");

        // step 1: anthropic, disjoint buckets, priced by rates
        assert_eq!(u[0].model, "claude-opus-4-8");
        assert_eq!(u[0].provider, Provider::Anthropic);
        assert_eq!(u[0].input_uncached, 1_000_000);
        assert_eq!(u[0].cache_read, 1_000_000);
        assert_eq!(u[0].cache_write_5m, 1_000_000);
        assert_eq!(u[0].output, 1_000_000);
        assert_eq!(u[0].native_cost_usd, None);

        // step 2: openai with an embedded cost_usd -> ground truth
        assert_eq!(u[1].model, "gpt-5.5");
        assert_eq!(u[1].provider, Provider::OpenAI);
        assert_eq!(u[1].native_cost_usd, Some(0.5));

        // step 3: an unpriced model, priced by rates -> $0 + surfaced unpriced
        assert_eq!(u[2].model, "made-up-model-zzz");
        assert_eq!(u[2].native_cost_usd, None);
    }
}
