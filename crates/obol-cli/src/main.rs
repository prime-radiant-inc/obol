use clap::{Parser, Subcommand};
use obol_core::{estimate_cost, refresh_pricing_tables, Dialect, PricingSource};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "obol", about = "Estimate agent-transcript token cost")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Estimate the cost of a transcript file.
    Estimate {
        path: PathBuf,
        #[arg(long, value_parser = ["claude", "codex", "pi", "gemini", "opencode", "copilot", "kimi", "obol"])]
        dialect: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Fetch the latest LiteLLM + OpenRouter price sheets.
    Refresh {
        /// Stamp for the snapshot: YYYY-MM-DD or YYYY-MM-DDTHH:MM:SSZ (UTC).
        /// Defaults to the current UTC datetime.
        #[arg(long)]
        as_of: Option<String>,
    },
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Estimate {
            path,
            dialect,
            json,
        } => {
            let dialect = match dialect.as_deref() {
                Some(d) => match d {
                    "claude" => Dialect::Claude,
                    "codex" => Dialect::Codex,
                    "pi" => Dialect::Pi,
                    "gemini" => Dialect::Gemini,
                    "opencode" => Dialect::Opencode,
                    "copilot" => Dialect::Copilot,
                    "kimi" => Dialect::Kimi,
                    "obol" => Dialect::Obol,
                    other => unreachable!("clap value_parser restricts dialect; got {other:?}"),
                },
                None => {
                    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                    obol_core::transcript::detect(&bytes)
                        .map_err(|e| format!("{e}; pass --dialect to choose one explicitly"))?
                }
            };
            let est = estimate_cost(&path, dialect).map_err(|e| e.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&est).map_err(|e| e.to_string())?
                );
            } else {
                let src = match est.pricing_source {
                    PricingSource::Bundled => "bundled",
                    PricingSource::Local => "local",
                };
                println!(
                    "total: ${:.4}  (pricing as of {}, {})",
                    est.total_usd, est.pricing_as_of, src
                );
                for m in &est.per_model {
                    println!("  {:30} ${:.4}", m.model, m.subtotal_usd);
                }
                if !est.unpriced_models.is_empty() {
                    println!(
                        "  unpriced (run `obol refresh`?): {}",
                        est.unpriced_models.join(", ")
                    );
                }
            }
            Ok(())
        }
        Cmd::Refresh { as_of } => {
            let as_of = as_of.unwrap_or_else(now_utc_stamp);
            let r = refresh_pricing_tables(&as_of).map_err(|e| e.to_string())?;
            println!(
                "refreshed {} models -> {}",
                r.models,
                r.written_to.display()
            );
            Ok(())
        }
    }
}

/// Format a unix timestamp as the canonical UTC stamp core accepts.
/// The library has no clock by design; the binary is where "now" lives.
fn utc_stamp_from_epoch(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (hh, mm, ss) = (rem / 3_600, rem % 3_600 / 60, rem % 60);
    // civil_from_days (Howard Hinnant), valid for the unix era onward.
    let z = days as i64 + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn now_utc_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_secs();
    utc_stamp_from_epoch(secs)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_stamp_from_epoch_formats_canonically() {
        assert_eq!(utc_stamp_from_epoch(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_stamp_from_epoch(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc_stamp_from_epoch(1_765_288_006), "2025-12-09T13:46:46Z");
        assert_eq!(utc_stamp_from_epoch(4_107_542_399), "2100-02-28T23:59:59Z");
    }

    #[test]
    fn now_stamp_passes_core_validation() {
        obol_core::pricing::as_of::validate(&now_utc_stamp()).unwrap();
    }
}
