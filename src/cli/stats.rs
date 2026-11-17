//! `inlocal stats` and `inlocal missing`

use std::collections::BTreeMap;
use std::process::ExitCode;

use crate::core::checks::{self, IssueKind};
use crate::core::Project;
use anyhow::{bail, Result};
use serde::Serialize;

use super::percent;

#[derive(Serialize, Default, Clone, Copy)]
struct Coverage {
    translated: usize,
    total: usize,
}

pub fn run(project: &Project, json: bool) -> Result<ExitCode> {
    let catalog = project.read_messages()?;
    let report = checks::run(project, &catalog);

    // Coverage per namespace and locale, the same way `check` counts it.
    let mut by_namespace: BTreeMap<String, BTreeMap<String, Coverage>> = BTreeMap::new();

    for message in &catalog.messages {
        let checked = checks::check_message(project, message);
        let namespace = by_namespace.entry(namespace_of(&message.id)).or_default();

        for (locale, (translated, total)) in checked.stats {
            let entry = namespace.entry(locale).or_default();

            entry.translated += translated;
            entry.total += total;
        }
    }

    if json {
        let out = serde_json::json!({
            "locales": report.stats,
            "namespaces": by_namespace,
        });

        println!("{}", serde_json::to_string_pretty(&out)?);

        return Ok(ExitCode::SUCCESS);
    }

    print_table(project, &report.stats, &by_namespace);

    Ok(ExitCode::SUCCESS)
}

fn print_table(
    project: &Project,
    totals: &BTreeMap<String, checks::LocaleStats>,
    by_namespace: &BTreeMap<String, BTreeMap<String, Coverage>>,
) {
    let locales = &project.locales;
    let has_namespaces = by_namespace.len() > 1 || !by_namespace.contains_key("");

    // Header.
    print!("{:<20}", "");

    for locale in locales {
        print!("{locale:>9}");
    }

    println!();

    // One row per namespace.
    if has_namespaces {
        for (namespace, coverage) in by_namespace {
            print!("{namespace:<20}");

            for locale in locales {
                let c = coverage.get(locale).copied().unwrap_or_default();
                print!("{:>8.0}%", percent(c.translated, c.total));
            }

            println!();
        }

        println!();
    }

    // Totals.
    print!("{:<20}", "total");

    for locale in locales {
        let pct = totals
            .get(locale)
            .map(|s| percent(s.translated, s.total))
            .unwrap_or(100.0);

        print!("{pct:>8.0}%");
    }

    println!();
}

pub fn missing(project: &Project, locale: &str) -> Result<ExitCode> {
    if !project.locales.iter().any(|l| l == locale) {
        bail!(
            "unknown locale '{locale}' (project has: {})",
            project.locales.join(", ")
        );
    }

    let catalog = project.read_messages()?;
    let report = checks::run(project, &catalog);

    let mut ids: Vec<&str> = report
        .issues
        .iter()
        .filter(|i| i.locale == locale)
        .filter(|i| matches!(i.kind, IssueKind::Missing | IssueKind::MissingPlural))
        .map(|i| i.id.as_str())
        .collect();

    ids.dedup();

    for id in &ids {
        println!("{id}");
    }

    let code = if ids.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    };

    Ok(code)
}

/// `common:nav.parts` -> `common`; no namespace -> "".
fn namespace_of(id: &str) -> String {
    id.split_once(':')
        .map(|(ns, _)| ns.to_string())
        .unwrap_or_default()
}
