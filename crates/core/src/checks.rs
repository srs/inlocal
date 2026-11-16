//! Lint rules: missing translations, missing plural forms and variable mismatches.

use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;
use serde::Serialize;

use crate::project::{Catalog, Format, Project};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum IssueKind {
    Missing,
    MissingPlural,
    MissingVariable,
    UnknownVariable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub id: String,
    pub locale: String,
    pub variant: String,
    pub kind: IssueKind,
    pub message: String,
    /// The variable name for variable issues.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocaleStats {
    pub translated: usize,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub issues: Vec<Issue>,
    pub stats: BTreeMap<String, LocaleStats>,
}

/// Variable names used in `text`, in order of first appearance.
pub fn variable_names(format: Format, text: &str) -> Vec<String> {
    let re = Regex::new(format.variable_regex()).expect("valid regex");

    let mut names: Vec<String> = Vec::new();
    for capture in re.captures_iter(text) {
        let name = capture[1].to_string();
        if !names.contains(&name) {
            names.push(name);
        }
    }

    names
}

pub fn run(project: &Project, catalog: &Catalog) -> Report {
    let format = project.format;
    let vars =
        |text: &str| -> BTreeSet<String> { variable_names(format, text).into_iter().collect() };
    let base = &project.base_locale;

    let mut issues = Vec::new();
    let mut stats: BTreeMap<String, LocaleStats> = project
        .locales
        .iter()
        .map(|l| {
            (
                l.clone(),
                LocaleStats {
                    translated: 0,
                    total: 0,
                },
            )
        })
        .collect();

    for msg in &catalog.messages {
        let Some(base_values) = msg.values.get(base) else {
            continue;
        };
        for locale in &project.locales {
            let values = msg.values.get(locale);
            let has_any = values.is_some_and(|v| v.values().any(|t| !t.is_empty()));
            for variant in &msg.variants {
                let Some(source) = base_values.get(variant).filter(|s| !s.is_empty()) else {
                    continue;
                };
                let stat = stats.get_mut(locale).expect("seeded");
                stat.total += 1;
                let text = values
                    .and_then(|v| v.get(variant))
                    .filter(|t| !t.is_empty());
                let Some(text) = text else {
                    if locale != base {
                        let (kind, message) = if !variant.is_empty() && has_any {
                            (
                                IssueKind::MissingPlural,
                                format!(
                                    "Missing plural form \u{201c}{}\u{201d}",
                                    display_variant(variant)
                                ),
                            )
                        } else {
                            (IssueKind::Missing, "Missing translation".to_string())
                        };
                        issues.push(Issue {
                            id: msg.id.clone(),
                            locale: locale.clone(),
                            variant: variant.clone(),
                            kind,
                            message,
                            variable: None,
                        });
                    }
                    continue;
                };
                stat.translated += 1;
                if locale == base {
                    continue;
                }
                let expected = vars(source);
                let actual = vars(text);
                for v in expected.difference(&actual) {
                    issues.push(Issue {
                        id: msg.id.clone(),
                        locale: locale.clone(),
                        variant: variant.clone(),
                        kind: IssueKind::MissingVariable,
                        message: format!("Missing variable {}", project.format.placeholder(v)),
                        variable: Some(v.clone()),
                    });
                }
                for v in actual.difference(&expected) {
                    issues.push(Issue {
                        id: msg.id.clone(),
                        locale: locale.clone(),
                        variant: variant.clone(),
                        kind: IssueKind::UnknownVariable,
                        message: format!("Unknown variable {}", project.format.placeholder(v)),
                        variable: Some(v.clone()),
                    });
                }
            }
        }
    }
    Report { issues, stats }
}

/// `countPlural=other` -> `other`
pub fn display_variant(variant: &str) -> &str {
    variant.rsplit('=').next().unwrap_or(variant)
}
