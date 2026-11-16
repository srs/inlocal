//! An editing session for interactive editors like the TUI.
//!
//! It holds what is on disk, the unsaved edits on top of it, and the issues
//! for the combination. Front ends only render it and call its methods.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::Result;

use crate::checks::{self, Issue, IssueKind, LocaleStats};
use crate::project::{Catalog, Edit, Message, Project};

/// Identifies one text field: a message, one of its variants, in one locale.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FieldKey {
    pub id: String,
    pub variant: String,
    pub locale: String,
}

impl FieldKey {
    pub fn new(id: &str, variant: &str, locale: &str) -> Self {
        Self {
            id: id.to_string(),
            variant: variant.to_string(),
            locale: locale.to_string(),
        }
    }
}

/// Sidebar filters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    NeedsAttention,
    Missing,
    Changed,
}

impl Filter {
    pub const ALL: [Filter; 4] = [
        Filter::All,
        Filter::NeedsAttention,
        Filter::Missing,
        Filter::Changed,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All messages",
            Filter::NeedsAttention => "Needs attention",
            Filter::Missing => "Missing",
            Filter::Changed => "Changed",
        }
    }
}

/// The colored dot next to a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warning,
    Missing,
    Changed,
}

pub struct Session {
    pub project: Arc<Project>,

    /// Messages as they are on disk.
    on_disk: Catalog,

    /// Unsaved text, keyed by field. Only values that differ from disk.
    edits: BTreeMap<FieldKey, String>,

    /// `on_disk` with `edits` applied. This is what the UI shows.
    catalog: Catalog,
    issues: HashMap<String, Vec<Issue>>,
    stats: BTreeMap<String, LocaleStats>,
}

impl Session {
    pub fn load(project: Arc<Project>) -> Result<Self> {
        let on_disk = project.read_messages()?;

        let mut model = Self {
            project,
            on_disk,
            edits: BTreeMap::new(),
            catalog: Catalog::default(),
            issues: HashMap::new(),
            stats: BTreeMap::new(),
        };
        model.recompute();

        Ok(model)
    }

    /// A model with no messages, used when the files can't be read.
    pub fn empty(project: Arc<Project>) -> Self {
        Self {
            project,
            on_disk: Catalog::default(),
            edits: BTreeMap::new(),
            catalog: Catalog::default(),
            issues: HashMap::new(),
            stats: BTreeMap::new(),
        }
    }

    /// Whether the files hold something other than what was last read.
    ///
    /// File watchers also fire for our own saves, so compare the content
    /// instead of trusting the event.
    pub fn changed_on_disk(&self) -> bool {
        match self.project.read_messages() {
            Ok(current) => current != self.on_disk,
            Err(_) => true,
        }
    }

    /// Re-read the files. Unsaved edits are kept.
    pub fn reload(&mut self) -> Result<()> {
        self.on_disk = self.project.read_messages()?;
        self.recompute();
        Ok(())
    }

    // -- Reading -------------------------------------------------------------

    pub fn messages(&self) -> &[Message] {
        &self.catalog.messages
    }

    pub fn message(&self, id: &str) -> Option<&Message> {
        self.catalog.messages.iter().find(|m| m.id == id)
    }

    /// The current text of a field, including unsaved edits.
    pub fn value(&self, key: &FieldKey) -> String {
        if let Some(edit) = self.edits.get(key) {
            return edit.clone();
        }
        text_in(&self.on_disk, key)
    }

    pub fn is_changed(&self, key: &FieldKey) -> bool {
        self.edits.contains_key(key)
    }

    pub fn has_edits(&self) -> bool {
        !self.edits.is_empty()
    }

    pub fn edit_count(&self) -> usize {
        self.edits.len()
    }

    pub fn issues(&self, id: &str) -> &[Issue] {
        self.issues.get(id).map(Vec::as_slice).unwrap_or_default()
    }

    pub fn field_issues(&self, key: &FieldKey) -> Vec<&Issue> {
        self.issues(&key.id)
            .iter()
            .filter(|i| i.locale == key.locale && i.variant == key.variant)
            .collect()
    }

    pub fn stats(&self, locale: &str) -> Option<&LocaleStats> {
        self.stats.get(locale)
    }

    pub fn status(&self, id: &str) -> Status {
        if self.edits.keys().any(|k| k.id == id) {
            return Status::Changed;
        }

        let issues = self.issues(id);
        if issues.iter().any(is_missing) {
            Status::Missing
        } else if !issues.is_empty() {
            Status::Warning
        } else {
            Status::Ok
        }
    }

    pub fn matches_filter(&self, message: &Message, filter: Filter) -> bool {
        let issues = self.issues(&message.id);

        match filter {
            Filter::All => true,
            Filter::NeedsAttention => !issues.is_empty(),
            Filter::Missing => issues.iter().any(is_missing),
            Filter::Changed => self.status(&message.id) == Status::Changed,
        }
    }

    /// Ids of messages matching a filter, an optional namespace and a search
    /// query (matched against keys and every translation, case-insensitive).
    pub fn visible_ids(&self, filter: Filter, namespace: Option<&str>, query: &str) -> Vec<String> {
        let query = query.trim().to_lowercase();

        self.messages()
            .iter()
            .filter(|m| self.matches_filter(m, filter))
            .filter(|m| match namespace {
                Some(ns) => m.id.starts_with(&format!("{ns}:")),
                None => true,
            })
            .filter(|m| query.is_empty() || matches_query(m, &query))
            .map(|m| m.id.clone())
            .collect()
    }

    pub fn count(&self, filter: Filter) -> usize {
        self.messages()
            .iter()
            .filter(|m| self.matches_filter(m, filter))
            .count()
    }

    // -- Editing -------------------------------------------------------------

    /// Set a field's text. Setting it back to what is on disk drops the edit.
    pub fn set_value(&mut self, key: FieldKey, value: String) {
        if value == text_in(&self.on_disk, &key) {
            self.edits.remove(&key);
        } else {
            self.edits.insert(key, value);
        }
        self.recompute();
    }

    pub fn discard(&mut self) {
        self.edits.clear();
        self.recompute();
    }

    /// Write all edits to disk and re-read the files.
    pub fn save(&mut self) -> Result<()> {
        if self.edits.is_empty() {
            return Ok(());
        }

        let edits: Vec<Edit> = self
            .edits
            .iter()
            .map(|(key, value)| Edit {
                id: key.id.clone(),
                variant: key.variant.clone(),
                locale: key.locale.clone(),
                value: (!value.is_empty()).then(|| value.clone()),
            })
            .collect();

        self.project.apply_edits(&edits)?;
        self.edits.clear();
        self.reload()
    }

    // -- Internals -----------------------------------------------------------

    /// Rebuild the shown catalog and its issues from disk + edits.
    fn recompute(&mut self) {
        let mut catalog = self.on_disk.clone();

        for (key, value) in &self.edits {
            let Some(message) = catalog.messages.iter_mut().find(|m| m.id == key.id) else {
                continue;
            };
            message
                .values
                .entry(key.locale.clone())
                .or_default()
                .insert(key.variant.clone(), value.clone());
        }

        let report = checks::run(&self.project, &catalog);

        let mut issues: HashMap<String, Vec<Issue>> = HashMap::new();
        for issue in report.issues {
            issues.entry(issue.id.clone()).or_default().push(issue);
        }

        self.catalog = catalog;
        self.issues = issues;
        self.stats = report.stats;
    }
}

fn matches_query(message: &Message, query: &str) -> bool {
    if message.id.to_lowercase().contains(query) {
        return true;
    }

    message
        .values
        .values()
        .flat_map(|variants| variants.values())
        .any(|text| text.to_lowercase().contains(query))
}

pub fn is_missing(issue: &Issue) -> bool {
    matches!(issue.kind, IssueKind::Missing | IssueKind::MissingPlural)
}

fn text_in(catalog: &Catalog, key: &FieldKey) -> String {
    catalog
        .messages
        .iter()
        .find(|m| m.id == key.id)
        .and_then(|m| m.values.get(&key.locale))
        .and_then(|values| values.get(&key.variant))
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn example_project(dir: &Path) -> Arc<Project> {
        let write = |rel: &str, text: &str| {
            let path = dir.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };

        write(
            "project.inlang/settings.json",
            r#"{
                "baseLocale": "en",
                "locales": ["en", "nb"],
                "plugin.inlang.messageFormat": { "pathPattern": "./messages/{locale}.json" }
            }"#,
        );
        write(
            "messages/en.json",
            r#"{ "hello": "Hello {name}", "bye": "Bye" }"#,
        );
        write("messages/nb.json", r#"{ "hello": "Hei {name}" }"#);

        Arc::new(Project::load(&dir.join("project.inlang")).unwrap())
    }

    #[test]
    fn edits_update_status_and_issues() {
        let tmp = tempfile::tempdir().unwrap();
        let mut model = Session::load(example_project(tmp.path())).unwrap();
        let bye_nb = FieldKey::new("bye", "", "nb");

        assert_eq!(model.status("bye"), Status::Missing);
        assert_eq!(model.count(Filter::Missing), 1);

        model.set_value(bye_nb.clone(), "Ha det".into());
        assert_eq!(model.status("bye"), Status::Changed);
        assert!(model.field_issues(&bye_nb).is_empty());

        model.set_value(bye_nb.clone(), String::new());
        assert!(
            !model.has_edits(),
            "back to the on-disk value drops the edit"
        );
    }

    #[test]
    fn own_saves_are_not_outside_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut model = Session::load(example_project(tmp.path())).unwrap();

        model.set_value(FieldKey::new("bye", "", "nb"), "Ha det".into());
        model.save().unwrap();
        assert!(!model.changed_on_disk());

        fs::write(
            tmp.path().join("messages/nb.json"),
            r#"{ "hello": "Hallo {name}" }"#,
        )
        .unwrap();
        assert!(model.changed_on_disk());
    }

    #[test]
    fn search_matches_keys_and_translations() {
        let tmp = tempfile::tempdir().unwrap();
        let model = Session::load(example_project(tmp.path())).unwrap();

        assert_eq!(model.visible_ids(Filter::All, None, "HEI"), vec!["hello"]);
        assert_eq!(model.visible_ids(Filter::All, None, "by"), vec!["bye"]);
        assert_eq!(model.visible_ids(Filter::Missing, None, ""), vec!["bye"]);
    }

    #[test]
    fn save_writes_and_clears_edits() {
        let tmp = tempfile::tempdir().unwrap();
        let mut model = Session::load(example_project(tmp.path())).unwrap();

        model.set_value(FieldKey::new("bye", "", "nb"), "Ha det".into());
        model.save().unwrap();

        assert!(!model.has_edits());
        assert_eq!(model.status("bye"), Status::Ok);

        let nb = fs::read_to_string(tmp.path().join("messages/nb.json")).unwrap();
        assert_eq!(nb, "{ \"hello\": \"Hei {name}\", \"bye\": \"Ha det\" }");
    }

    #[test]
    fn variable_issues_follow_edits() {
        let tmp = tempfile::tempdir().unwrap();
        let mut model = Session::load(example_project(tmp.path())).unwrap();
        let hello_nb = FieldKey::new("hello", "", "nb");

        model.set_value(hello_nb.clone(), "Hei".into());

        let issues = model.field_issues(&hello_nb);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, IssueKind::MissingVariable);
    }
}
