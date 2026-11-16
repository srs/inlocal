//! Command-line interface: argument parsing and one module per command.

mod check;
mod info;
mod stats;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use inlocal_core::{find_project, Project};

/// A local translation editor for inlang projects.
#[derive(Parser)]
#[command(version, about)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to a project.inlang directory or a folder containing one.
    #[arg(global = true, short, long)]
    project: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Edit translations in the terminal (default).
    Tui,

    /// Open the editor in your browser.
    Serve {
        #[arg(long, default_value_t = 4321)]
        port: u16,

        /// Don't open a browser window.
        #[arg(long)]
        no_open: bool,
    },

    /// Report missing translations and broken variables. Exits 1 on issues.
    Check {
        /// Only check these locales (repeatable).
        #[arg(short, long)]
        locale: Vec<String>,

        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },

    /// Translation coverage per locale, and per namespace if there are any.
    Stats {
        #[arg(long)]
        json: bool,
    },

    /// List messages that are missing in a locale, one per line.
    Missing {
        /// The locale to look at.
        locale: String,
    },

    /// Show the detected project: format, locales and files.
    Info,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    /// Readable output for the terminal.
    Text,
    /// Machine-readable JSON.
    Json,
    /// GitHub Actions annotations, shown inline on pull requests.
    Github,
}

pub fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Tui);

    let project = load_project(cli.project)?;

    match command {
        Command::Tui => {
            crate::tui::run(project)?;
            Ok(ExitCode::SUCCESS)
        }

        Command::Serve { port, no_open } => serve(project, port, !no_open),

        Command::Check { locale, format } => check::run(&project, &locale, format),
        Command::Stats { json } => stats::run(&project, json),
        Command::Missing { locale } => stats::missing(&project, &locale),
        Command::Info => info::run(&project),
    }
}

#[cfg(feature = "web")]
fn serve(project: Project, port: u16, open_browser: bool) -> Result<ExitCode> {
    crate::server::run(project, port, open_browser)?;
    Ok(ExitCode::SUCCESS)
}

#[cfg(not(feature = "web"))]
fn serve(_: Project, _: u16, _: bool) -> Result<ExitCode> {
    anyhow::bail!("this build of inlocal has no web UI.")
}

fn load_project(path: Option<PathBuf>) -> Result<Project> {
    let start = match path {
        Some(p) => p,
        None => std::env::current_dir()?,
    };

    let dir = find_project(&start)
        .with_context(|| format!("no project.inlang found in or near {}", start.display()))?;

    Project::load(&dir)
}

/// Percent with one decimal, treating an empty locale as complete.
pub fn percent(translated: usize, total: usize) -> f64 {
    if total == 0 {
        100.0
    } else {
        translated as f64 * 100.0 / total as f64
    }
}
