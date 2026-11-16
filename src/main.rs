mod checks;
mod jsonedit;
mod project;
mod server;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::checks::display_variant;
use crate::project::{find_project, Project};

/// A local translation editor for inlang projects.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to a project.inlang directory or a folder containing one.
    #[arg(global = true, short, long)]
    project: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Open the editor in your browser (default).
    Serve {
        #[arg(long, default_value_t = 4321)]
        port: u16,
        /// Don't open a browser window.
        #[arg(long)]
        no_open: bool,
    },
    /// Report missing translations and broken variables. Exits 1 on issues.
    Check {
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let start = match &cli.project {
        Some(p) => p.clone(),
        None => std::env::current_dir()?,
    };
    let dir = find_project(&start)
        .with_context(|| format!("no project.inlang found in or near {}", start.display()))?;
    let project = Project::load(&dir)?;

    match cli.command.unwrap_or(Command::Serve { port: 4321, no_open: false }) {
        Command::Serve { port, no_open } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(server::serve(project, port, !no_open))?;
            // Don't wait for lingering tasks (file watcher, open streams).
            rt.shutdown_timeout(std::time::Duration::from_millis(200));
            Ok(ExitCode::SUCCESS)
        }
        Command::Check { json } => check(&project, json),
    }
}

fn check(project: &Project, json: bool) -> Result<ExitCode> {
    let catalog = project.read_messages()?;
    let report = checks::run(project, &catalog);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for issue in &report.issues {
            let variant = if issue.variant.is_empty() {
                String::new()
            } else {
                format!(" [{}]", display_variant(&issue.variant))
            };
            println!("{:<6} {}{}  {}", issue.locale, issue.id, variant, issue.message);
        }
        println!();
        for (locale, s) in &report.stats {
            let pct = if s.total == 0 { 100.0 } else { s.translated as f64 * 100.0 / s.total as f64 };
            println!("{locale:<6} {pct:>5.1}%  ({}/{})", s.translated, s.total);
        }
        println!("\n{} message(s), {} issue(s)", catalog.messages.len(), report.issues.len());
    }
    Ok(if report.issues.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}
