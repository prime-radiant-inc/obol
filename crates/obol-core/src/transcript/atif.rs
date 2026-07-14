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

    // Cost precedence (no double-count guaranteed):
    //
    // 1. Sum of per-step `cost_usd` wins when ALL billable steps carry it.
    //    The cost engine uses each step's `native_cost_usd` verbatim; nothing
    //    else is added.
    //
    // 2. `final_metrics.total_cost_usd` wins when present and the per-step sum
    //    is incomplete (at least one billable step lacks `cost_usd`).  To
    //    prevent double-count the cost engine must NOT also run rate-table math
    //    on those steps: we null out every step's `native_cost_usd` (so the
    //    cost engine sees None and would normally go to the rate table) AND
    //    inject a zero-token record carrying the explicit total.  The rate-table
    //    path for each step then contributes 0 because... wait, that still runs
    //    rate-table math on tokens.  Instead: we suppress rate-table by setting
    //    `native_cost_usd = Some(0.0)` on every step and emit a separate record
    //    that carries `native_cost_usd = Some(final_total)` with zero tokens.
    //    Result: total_usd = 0*N + final_total = final_total.  Tokens are still
    //    aggregated correctly from the step records.
    //
    // 3. If no explicit cost exists anywhere, fall back to rate-table estimates
    //    (or `final_metrics` tokens for a totals-only trajectory).
    if !out.is_empty() {
        let final_total = doc
            .pointer("/final_metrics/total_cost_usd")
            .and_then(Value::as_f64)
            .filter(|c| c.is_finite() && *c >= 0.0);

        if let Some(total) = final_total {
            // Check whether every billable step already carries an explicit cost.
            let all_steps_have_cost = out.iter().all(|r| r.native_cost_usd.is_some());
            if !all_steps_have_cost {
                // Incomplete per-step cost sum: override with the explicit total.
                // Suppress rate-table math on every step so they don't add to the
                // total — set native_cost_usd = Some(0.0) on each.
                for rec in &mut out {
                    rec.native_cost_usd = Some(0.0);
                }
                // Inject a zero-token record that carries the authoritative total.
                // It uses agent.model_name so the cost is attributed to the primary
                // model; per-model cost breakdown is unavailable when only a session
                // total was logged.
                let (namespace, provider) = route(None, agent_model);
                out.push(MessageUsage {
                    model: agent_model.to_string(),
                    provider,
                    namespace,
                    input_uncached: 0,
                    cache_read: 0,
                    cache_write_5m: 0,
                    cache_write_1h: 0,
                    output: 0,
                    request_input_tokens: 0,
                    service_tier: None,
                    native_cost_usd: Some(total),
                });
            }
            // If all steps already have cost_usd, their sum is authoritative — no
            // action needed; final_metrics.total_cost_usd is a redundant rollup.
        }
        // No final_metrics.total_cost_usd: each step is priced by its own
        // native_cost_usd (if present) or by rate-table lookup.
    } else {
        // No billable step records: fall back to final_metrics for totals-only
        // trajectories (e.g. a normalizer that only emitted aggregate counts).
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

    // Defect 2: when a trajectory has per-step token records AND
    // final_metrics.total_cost_usd, the explicit total must win — not the
    // rate-table estimate derived from the token counts.
    //
    // This test verifies the parser-level output: the step records must have
    // native_cost_usd = Some(0.0) (suppressing rate-table) and an extra
    // zero-token record with native_cost_usd = Some(final_total) must be
    // appended.  The cost engine then computes: 0*N + final_total = final_total.
    #[test]
    fn final_metrics_total_cost_overrides_rate_table_when_steps_lack_cost() {
        let line = r#"{
          "schema_version":"ATIF-v1.7",
          "agent":{"name":"claude","model_name":"claude-opus-4-8"},
          "steps":[
            {"step_id":"s1","source":"agent",
             "metrics":{"prompt_tokens":1000,"completion_tokens":500}}
          ],
          "final_metrics":{"total_prompt_tokens":1000,"total_completion_tokens":500,
                           "total_cost_usd":3.14}
        }"#;
        let u = parse(line.as_bytes()).unwrap();
        // Two records: the original step (tokens, native_cost suppressed to 0)
        // and the zero-token cost record carrying the explicit total.
        assert_eq!(u.len(), 2, "expected step + cost override record: {u:?}");
        // Step record: tokens preserved, cost suppressed to 0 so rate-table is skipped.
        assert_eq!(u[0].input_uncached, 1000);
        assert_eq!(u[0].output, 500);
        assert_eq!(
            u[0].native_cost_usd,
            Some(0.0),
            "step cost must be suppressed so it doesn't add to the rate-table total"
        );
        // Cost override record: zero tokens, carries the explicit total.
        assert_eq!(u[1].input_uncached, 0);
        assert_eq!(u[1].output, 0);
        assert_eq!(
            u[1].native_cost_usd,
            Some(3.14),
            "the explicit final total must be carried on the override record"
        );
    }

    // When some steps have per-step cost_usd and final_metrics.total_cost_usd
    // also exists, the final total wins (incomplete per-step sum is discarded).
    // No double-count: suppressing mixed partial costs and using the authoritative
    // total gives exactly total_cost_usd.
    #[test]
    fn final_metrics_total_wins_over_partial_step_costs_no_double_count() {
        let line = r#"{
          "schema_version":"ATIF-v1.7",
          "agent":{"name":"claude","model_name":"claude-opus-4-8"},
          "steps":[
            {"step_id":"s1","source":"agent","model_name":"gpt-5.5",
             "metrics":{"prompt_tokens":100,"completion_tokens":20,"cost_usd":0.50},
             "extra":{"provider":"openai"}},
            {"step_id":"s2","source":"agent","model_name":"claude-opus-4-8",
             "metrics":{"prompt_tokens":200,"completion_tokens":10}}
          ],
          "final_metrics":{"total_cost_usd":1.23}
        }"#;
        let u = parse(line.as_bytes()).unwrap();
        // 2 step records + 1 override record.
        assert_eq!(u.len(), 3, "step1 + step2 + override: {u:?}");
        // Both step records have their cost suppressed to 0.
        assert_eq!(u[0].native_cost_usd, Some(0.0), "step1 cost suppressed");
        assert_eq!(u[1].native_cost_usd, Some(0.0), "step2 cost suppressed");
        // Override record carries the authoritative total.
        assert_eq!(u[2].native_cost_usd, Some(1.23));
        assert_eq!(u[2].input_uncached, 0);
        assert_eq!(u[2].output, 0);
        // Tokens from the step records are still preserved for accounting.
        let total_input: u64 = u.iter().map(|r| r.input_uncached).sum();
        let total_output: u64 = u.iter().map(|r| r.output).sum();
        assert_eq!(total_input, 300, "step tokens preserved: {total_input}");
        assert_eq!(total_output, 30, "step tokens preserved: {total_output}");
    }

    // When ALL steps already carry per-step cost_usd, the sum of those costs
    // is authoritative and final_metrics.total_cost_usd must NOT override or
    // add to it.  This guards against double-count when both sources agree.
    #[test]
    fn all_steps_have_cost_usd_final_metrics_total_is_ignored() {
        let line = r#"{
          "schema_version":"ATIF-v1.7",
          "agent":{"name":"claude","model_name":"claude-opus-4-8"},
          "steps":[
            {"step_id":"s1","source":"agent","model_name":"gpt-5.5",
             "metrics":{"prompt_tokens":100,"completion_tokens":20,"cost_usd":0.50},
             "extra":{"provider":"openai"}},
            {"step_id":"s2","source":"agent","model_name":"claude-opus-4-8",
             "metrics":{"prompt_tokens":200,"completion_tokens":10,"cost_usd":0.75}}
          ],
          "final_metrics":{"total_cost_usd":999.99}
        }"#;
        let u = parse(line.as_bytes()).unwrap();
        // Only the two step records — no override injected when all steps priced.
        assert_eq!(
            u.len(),
            2,
            "no override record when all steps have cost: {u:?}"
        );
        // Per-step costs preserved as-is.
        assert_eq!(u[0].native_cost_usd, Some(0.50));
        assert_eq!(u[1].native_cost_usd, Some(0.75));
        // The sum is 1.25, NOT 999.99.  The final_metrics total is the rollup, not
        // an authoritative override, because per-step costs are already complete.
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
