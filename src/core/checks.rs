//! Lint rules: missing translations, missing plural forms and variable mismatches.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use super::project::{is_catch_all, Catalog, Format, Message, Project};

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

static SINGLE_BRACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(Format::MessageFormat.variable_regex()).expect("valid regex"));

static DOUBLE_BRACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(Format::I18next.variable_regex()).expect("valid regex"));

/// Variable names used in `text`, in order of first appearance.
pub fn variable_names(format: Format, text: &str) -> Vec<String> {
    // Most texts have no variables at all; skip the regex for those.
    if !text.contains('{') {
        return Vec::new();
    }

    let re = match format {
        Format::I18next => &*DOUBLE_BRACE,
        Format::MessageFormat | Format::NestedJson => &*SINGLE_BRACE,
    };

    let mut names: Vec<String> = Vec::new();

    for capture in re.captures_iter(text) {
        let name = capture[1].to_string();

        if !names.contains(&name) {
            names.push(name);
        }
    }

    names
}

/// Check every message.
pub fn run(project: &Project, catalog: &Catalog) -> Report {
    let mut issues = Vec::new();
    let mut stats = empty_stats(project);

    for message in &catalog.messages {
        let checked = check_message(project, message);

        issues.extend(checked.issues);
        add_stats(&mut stats, &checked.stats, 1);
    }

    Report { issues, stats }
}

/// The result of checking one message.
#[derive(Debug, Clone, Default)]
pub struct MessageCheck {
    pub issues: Vec<Issue>,
    /// Per locale: (translated, total). Counted per message: 1 of 1 when the
    /// locale has text for every form, otherwise 0 of 1.
    pub stats: BTreeMap<String, (usize, usize)>,
}

/// Check one message in every locale.
///
/// Each locale is checked against its own plural forms: languages differ
/// in which forms they need (Ukrainian adds `few` and `many`, and an
/// ordinal can be a plain string in Norwegian), so a locale doesn't have to
/// mirror the base locale's forms. Variables are compared across all forms
/// of a message together, so `one` may say "a file" instead of "{count} file".
pub fn check_message(project: &Project, message: &Message) -> MessageCheck {
    let mut result = MessageCheck::default();
    let base = &project.base_locale;

    let Some(base_values) = message.values.get(base) else {
        return result;
    };

    // Nothing to translate if the source text is empty.
    if base_values.values().all(|t| t.is_empty()) {
        return result;
    }

    let expected = variables_in(project, base_values.values());

    for locale in &project.locales {
        let stat = result.stats.entry(locale.clone()).or_default();
        stat.1 += 1;

        if locale == base {
            stat.0 += 1;
            continue;
        }

        let variants = message.variants_for(locale);
        let values = message.values.get(locale);
        let text = |variant: &str| {
            values
                .and_then(|v| v.get(variant))
                .filter(|t| !t.is_empty())
        };

        let filled: Vec<&String> = variants.iter().filter_map(|v| text(v)).collect();
        let before = result.issues.len();

        if filled.is_empty() {
            // Nothing translated yet: every form is missing.
            for variant in &variants {
                result
                    .issues
                    .push(missing_issue(message, locale, variant, false));
            }
        } else {
            // Partly translated: name the empty forms.
            for variant in variants.iter().filter(|v| text(v).is_none()) {
                result
                    .issues
                    .push(missing_issue(message, locale, variant, true));
            }

            // A plural message needs the catch-all form.
            let plural = variants.iter().any(|v| !v.is_empty());
            let has_catch_all = variants.iter().any(|v| is_catch_all(v));
            let base_catch_all = message.variants.iter().find(|v| is_catch_all(v));

            if let (true, false, Some(catch_all)) = (plural, has_catch_all, base_catch_all) {
                result
                    .issues
                    .push(missing_issue(message, locale, catch_all, true));
            }

            // Variables, across all forms.
            let actual = variables_in(project, filled.iter().copied());
            let anchor = variants
                .iter()
                .find(|v| is_catch_all(v))
                .or_else(|| variants.iter().find(|v| text(v).is_some()))
                .cloned()
                .unwrap_or_default();

            for (kind, names) in [
                (IssueKind::MissingVariable, expected.difference(&actual)),
                (IssueKind::UnknownVariable, actual.difference(&expected)),
            ] {
                for name in names {
                    result.issues.push(variable_issue(
                        project, message, locale, &anchor, kind, name,
                    ));
                }
            }
        }

        let missing = result.issues[before..]
            .iter()
            .any(|i| matches!(i.kind, IssueKind::Missing | IssueKind::MissingPlural));

        if !missing {
            stat.0 += 1;
        }
    }

    result
}

/// Every variable used in any of `texts`.
fn variables_in<'a>(
    project: &Project,
    texts: impl Iterator<Item = &'a String>,
) -> BTreeSet<String> {
    texts
        .flat_map(|text| variable_names(project.format, text))
        .collect()
}

fn missing_issue(message: &Message, locale: &str, variant: &str, has_any: bool) -> Issue {
    let (kind, text) = if !variant.is_empty() && has_any {
        let form = display_variant(variant);
        (
            IssueKind::MissingPlural,
            format!("Missing plural form \u{201c}{form}\u{201d}"),
        )
    } else {
        (IssueKind::Missing, "Missing translation".to_string())
    };

    Issue {
        id: message.id.clone(),
        locale: locale.to_string(),
        variant: variant.to_string(),
        kind,
        message: text,
        variable: None,
    }
}

fn variable_issue(
    project: &Project,
    message: &Message,
    locale: &str,
    variant: &str,
    kind: IssueKind,
    name: &str,
) -> Issue {
    let what = match kind {
        IssueKind::MissingVariable => "Missing variable",
        _ => "Unknown variable",
    };

    Issue {
        id: message.id.clone(),
        locale: locale.to_string(),
        variant: variant.to_string(),
        kind,
        message: format!("{what} {}", project.format.placeholder(name)),
        variable: Some(name.to_string()),
    }
}

/// Every locale with zero counts.
pub fn empty_stats(project: &Project) -> BTreeMap<String, LocaleStats> {
    project
        .locales
        .iter()
        .map(|locale| {
            let stats = LocaleStats {
                translated: 0,
                total: 0,
            };

            (locale.clone(), stats)
        })
        .collect()
}

/// Add (`sign` = 1) or remove (`sign` = -1) one message's counts.
pub fn add_stats(
    totals: &mut BTreeMap<String, LocaleStats>,
    message: &BTreeMap<String, (usize, usize)>,
    sign: isize,
) {
    for (locale, (translated, total)) in message {
        let Some(entry) = totals.get_mut(locale) else {
            continue;
        };

        entry.translated = entry
            .translated
            .saturating_add_signed(sign * *translated as isize);

        entry.total = entry.total.saturating_add_signed(sign * *total as isize);
    }
}

/// `countPlural=other` -> `other`
pub fn display_variant(variant: &str) -> &str {
    variant.rsplit('=').next().unwrap_or(variant)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn plural(forms: &[(&str, &str)]) -> String {
        let matches: Vec<String> = forms
            .iter()
            .map(|(form, text)| format!(r#""countPlural={form}": "{text}""#))
            .collect();

        format!(
            r#"[{{ "declarations": ["input count", "local countPlural = count: plural"],
                   "selectors": ["countPlural"], "match": {{ {} }} }}]"#,
            matches.join(", ")
        )
    }

    #[test]
    fn each_locale_has_its_own_plural_forms() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();

        let write = |rel: &str, text: &str| {
            let path = d.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };

        write(
            "project.inlang/settings.json",
            r#"{ "baseLocale": "en", "locales": ["en", "nb", "uk", "sv"],
                 "plugin.inlang.messageFormat": { "pathPattern": "./messages/{locale}.json" } }"#,
        );

        // English: "one" leaves out {count}, which is fine.
        let en = plural(&[
            ("one", "Uploading a file"),
            ("*", "Uploading {count} files"),
        ]);

        write("messages/en.json", &format!(r#"{{ "files": {en} }}"#));

        // Norwegian: a plain string is a complete translation.
        write(
            "messages/nb.json",
            r#"{ "files": "Laster opp {count} filer" }"#,
        );

        // Ukrainian: more forms than English, all filled.
        let uk = plural(&[
            ("one", "{count} файл"),
            ("few", "{count} файли"),
            ("many", "{count} файлів"),
            ("*", "{count} файлу"),
        ]);

        write("messages/uk.json", &format!(r#"{{ "files": {uk} }}"#));

        // Swedish: a form without the catch-all is incomplete.
        let sv = plural(&[("one", "En fil")]);
        write("messages/sv.json", &format!(r#"{{ "files": {sv} }}"#));

        let project = Project::load(&d.join("project.inlang")).unwrap();
        let catalog = project.read_messages().unwrap();
        let report = run(&project, &catalog);

        let issues: Vec<String> = report
            .issues
            .iter()
            .map(|i| format!("{} {} {}", i.locale, display_variant(&i.variant), i.message))
            .collect();

        assert_eq!(
            issues,
            vec![
                "sv * Missing plural form \u{201c}*\u{201d}",
                "sv one Missing variable {count}",
            ]
        );

        assert_eq!(report.stats["nb"].translated, 1);
        assert_eq!(report.stats["uk"].translated, 1);
        assert_eq!(report.stats["sv"].translated, 0);
    }
}
