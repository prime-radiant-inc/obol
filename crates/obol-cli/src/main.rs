use clap::{Parser, Subcommand};
use obol_core::{estimate_cost, refresh_pricing_tables, Dialect, Source};
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
        #[arg(long, value_parser = ["claude", "codex", "pi"])]
        dialect: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Fetch the latest LiteLLM price sheet.
    Refresh {
        /// Date stamp for the snapshot (YYYY-MM-DD); caller supplies the clock.
        #[arg(long)]
        as_of: String,
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
            let hint = dialect.as_deref().map(|d| match d {
                "claude" => Dialect::Claude,
                "codex" => Dialect::Codex,
                _ => Dialect::Pi,
            });
            let est = estimate_cost(Source::Path(&path), hint).map_err(|e| e.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&est).map_err(|e| e.to_string())?
                );
            } else {
                println!(
                    "total: ${:.4}  (pricing as of {})",
                    est.total_usd, est.pricing_as_of
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

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
