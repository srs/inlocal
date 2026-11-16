//! `inlocal check`

use std::process::ExitCode;

use crate::core::checks::{self, Issue};
use crate::core::{display_variant, Project};
use anyhow::{bail, Result};

use super::{percent, OutputFormat};

pub fn run(project: &Project, locales: &[String], format: OutputFormat) -> Result<ExitCode> {
    for locale in locales {
        if !project.locales.contains(locale) {
            bail!(
                "unknown locale '{locale}' (project has: {})",
                project.locales.join(", ")
            );
        }
    }

    let catalog = project.read_messages()?;
    let mut report = checks::run(project, &catalog);

    if !locales.is_empty() {
        report.issues.retain(|i| locales.contains(&i.locale));
        report.stats.retain(|locale, _| locales.contains(locale));
    }

    match format {
        OutputFormat::Text => print_text(&report, catalog.messages.len()),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        OutputFormat::Github => print_github(project, &report.issues),
    }

    let code = if report.issues.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    };

    Ok(code)
}

fn print_text(report: &checks::Report, message_count: usize) {
    for issue in &report.issues {
        println!(
            "{:<6} {}{}  {}",
            issue.locale,
            issue.id,
            variant_suffix(issue),
            issue.message
        );
    }

    if !report.issues.is_empty() {
        println!();
    }

    for (locale, stats) in &report.stats {
        let pct = percent(stats.translated, stats.total);
        println!(
            "{locale:<6} {pct:>5.1}%  ({}/{})",
            stats.translated, stats.total
        );
    }

    println!(
        "\n{message_count} message(s), {} issue(s)",
        report.issues.len()
    );
}

/// One `::warning` line per issue. GitHub shows these on the pull request,
/// next to the file and line they point at.
fn print_github(project: &Project, issues: &[Issue]) {
    for issue in issues {
        let title = format!("{} [{}]", issue.id, issue.locale);
        let mut properties = format!("title={}", escape_property(&title));

        if let Some(location) = project.locate(&issue.id, &issue.variant, &issue.locale) {
            let file = location
                .file
                .strip_prefix(&project.root)
                .unwrap_or(&location.file)
                .display()
                .to_string();

            properties = format!("file={},{properties}", escape_property(&file));

            if let Some(line) = location.line {
                properties = format!("{properties},line={line}");
            }
        }

        let message = format!("{}{}", issue.message, variant_suffix(issue));
        println!("::warning {properties}::{}", escape_data(&message));
    }
}

fn variant_suffix(issue: &Issue) -> String {
    if issue.variant.is_empty() {
        String::new()
    } else {
        format!(" [{}]", display_variant(&issue.variant))
    }
}

// GitHub workflow commands need %, CR and LF escaped (and : , in properties).

fn escape_data(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn escape_property(text: &str) -> String {
    escape_data(text).replace(':', "%3A").replace(',', "%2C")
}
