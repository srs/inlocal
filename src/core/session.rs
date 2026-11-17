//! An editing session for interactive editors like the TUI.
//!
//! It holds what is on disk, the unsaved edits on top of it, and the issues
//! for the combination. Front ends only render it and call its methods.

use std::cell::OnceCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::Result;

use super::checks::{self, Issue, IssueKind, LocaleStats};
use super::project::{is_catch_all, Catalog, Edit, Message, Project};
use super::usages::{self, UsageReport};

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

    /// How many edits each message has, for its status.
    edited_ids: HashMap<String, usize>,

    /// `on_disk` with `edits` applied. This is what the UI shows.
    catalog: Catalog,

    // Derived from `catalog`. Kept per message, so an edit only re-checks
    // the message it touches.
    issues: HashMap<String, Vec<Issue>>,
    message_stats: HashMap<String, BTreeMap<String, (usize, usize)>>,
    stats: BTreeMap<String, LocaleStats>,

    /// Lowercased id and texts per message, for search.
    search_text: HashMap<String, String>,

    /// Messages per filter, counted on first use after each change.
    counts: OnceCell<[usize; 4]>,
}

impl Session {
    pub fn load(project: Arc<Project>) -> Result<Self> {
        let on_disk = project.read_messages()?;

        let mut model = Self {
            project,
            on_disk,
            edits: BTreeMap::new(),
            edited_ids: HashMap::new(),
            catalog: Catalog::default(),
            issues: HashMap::new(),
            message_stats: HashMap::new(),
            stats: BTreeMap::new(),
            search_text: HashMap::new(),
            counts: OnceCell::new(),
        };

        model.recompute();

        Ok(model)
    }

    /// A code scan that can run on another thread, so a big repository
    /// doesn't block the UI.
    pub fn usage_scan_job(&self) -> impl FnOnce() -> UsageReport + Send + 'static {
        let project = self.project.clone();
        let catalog = self.on_disk.clone();

        move || usages::scan(&project, &catalog)
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
        self.catalog.get(id)
    }

    /// The current text of a field, including unsaved edits.
    pub fn value(&self, key: &FieldKey) -> String {
        if let Some(edit) = self.edits.get(key) {
            return edit.clone();
        }

        text_in(&self.on_disk, key)
    }

    /// The text to translate a field from: the same form in `locale`, or
    /// that locale's catch-all form when it doesn't have this one (English
    /// has no `few`, but Ukrainian does).
    pub fn source_text(&self, key: &FieldKey, locale: &str) -> String {
        let same = self.value(&FieldKey::new(&key.id, &key.variant, locale));

        if !same.is_empty() {
            return same;
        }

        let Some(message) = self.message(&key.id) else {
            return String::new();
        };

        let variants = message.variants_for(locale);
        let fallback = variants
            .iter()
            .find(|v| is_catch_all(v))
            .or_else(|| variants.last());

        fallback
            .map(|v| self.value(&FieldKey::new(&key.id, v, locale)))
            .unwrap_or_default()
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
        if self.edited_ids.contains_key(id) {
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
            .filter(|m| query.is_empty() || self.matches_query(&m.id, &query))
            .map(|m| m.id.clone())
            .collect()
    }

    pub fn count(&self, filter: Filter) -> usize {
        let counts = self.counts.get_or_init(|| {
            Filter::ALL.map(|f| {
                self.messages()
                    .iter()
                    .filter(|m| self.matches_filter(m, f))
                    .count()
            })
        });

        let ix = Filter::ALL.iter().position(|f| *f == filter).unwrap_or(0);
        counts[ix]
    }

    // -- Editing -------------------------------------------------------------

    /// Set a field's text. Setting it back to what is on disk drops the edit.
    pub fn set_value(&mut self, key: FieldKey, value: String) {
        let on_disk = self.on_disk.get(&key.id).and_then(|m| field_text(m, &key));

        let was_edited = self.edits.contains_key(&key);
        let is_edited = value != on_disk.unwrap_or_default();

        // What the field should hold now: the edit, or what is on disk.
        let shown = if is_edited {
            Some(value.as_str())
        } else {
            on_disk
        };

        set_field(&mut self.catalog, &key, shown);

        match (was_edited, is_edited) {
            (false, true) => *self.edited_ids.entry(key.id.clone()).or_default() += 1,
            (true, false) => self.forget_edit_of(&key.id),
            _ => {}
        }

        let id = key.id.clone();

        if is_edited {
            self.edits.insert(key, value);
        } else {
            self.edits.remove(&key);
        }

        self.recheck(&id);
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

    /// Rebuild the shown catalog and everything derived from it.
    fn recompute(&mut self) {
        let mut catalog = self.on_disk.clone();

        for (key, value) in &self.edits {
            set_field(&mut catalog, key, Some(value));
        }

        self.catalog = catalog;
        self.issues.clear();
        self.message_stats.clear();
        self.search_text.clear();
        self.stats = checks::empty_stats(&self.project);

        let ids: Vec<String> = self.catalog.messages.iter().map(|m| m.id.clone()).collect();

        for id in ids {
            self.recheck(&id);
        }

        self.edited_ids.clear();

        for key in self.edits.keys() {
            *self.edited_ids.entry(key.id.clone()).or_default() += 1;
        }
    }

    /// Re-check one message and update the totals.
    fn recheck(&mut self, id: &str) {
        self.counts = OnceCell::new();

        let Some(message) = self.catalog.get(id) else {
            return;
        };

        let checked = checks::check_message(&self.project, message);
        let search_text = search_text(message);

        if let Some(old) = self.message_stats.remove(id) {
            checks::add_stats(&mut self.stats, &old, -1);
        }

        checks::add_stats(&mut self.stats, &checked.stats, 1);

        self.message_stats.insert(id.to_string(), checked.stats);
        self.search_text.insert(id.to_string(), search_text);

        if checked.issues.is_empty() {
            self.issues.remove(id);
        } else {
            self.issues.insert(id.to_string(), checked.issues);
        }
    }

    fn forget_edit_of(&mut self, id: &str) {
        if let Some(count) = self.edited_ids.get_mut(id) {
            *count -= 1;

            if *count == 0 {
                self.edited_ids.remove(id);
            }
        }
    }

    fn matches_query(&self, id: &str, query: &str) -> bool {
        self.search_text
            .get(id)
            .is_some_and(|text| text.contains(query))
    }
}

/// The lowercased id and every text of a message, joined, for search.
fn search_text(message: &Message) -> String {
    let mut text = message.id.to_lowercase();

    for values in message.values.values() {
        for value in values.values() {
            text.push('\n');
            text.push_str(&value.to_lowercase());
        }
    }

    text
}

fn field_text<'a>(message: &'a Message, key: &FieldKey) -> Option<&'a str> {
    message
        .values
        .get(&key.locale)
        .and_then(|values| values.get(&key.variant))
        .map(String::as_str)
}

/// Put `text` into a field of the catalog, or remove the field for `None`.
fn set_field(catalog: &mut Catalog, key: &FieldKey, text: Option<&str>) {
    let Some(message) = catalog.get_mut(&key.id) else {
        return;
    };

    match text {
        Some(text) => {
            message
                .values
                .entry(key.locale.clone())
                .or_default()
                .insert(key.variant.clone(), text.to_string());
        }

        None => {
            if let Some(values) = message.values.get_mut(&key.locale) {
                values.remove(&key.variant);
            }
        }
    }
}

pub fn is_missing(issue: &Issue) -> bool {
    matches!(issue.kind, IssueKind::Missing | IssueKind::MissingPlural)
}

fn text_in(catalog: &Catalog, key: &FieldKey) -> String {
    catalog
        .get(&key.id)
        .and_then(|m| field_text(m, key))
        .unwrap_or_default()
        .to_string()
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
    fn incremental_updates_match_a_full_recheck() {
        let tmp = tempfile::tempdir().unwrap();
        let mut model = Session::load(example_project(tmp.path())).unwrap();

        let edits = [
            ("bye", "nb", "Ha det"),
            ("hello", "nb", "Hei"),
            ("hello", "nb", "Hei {name}"),
            ("bye", "nb", ""),
            ("hello", "en", "Hello {name} {x}"),
        ];

        for (id, locale, text) in edits {
            model.set_value(FieldKey::new(id, "", locale), text.into());
        }

        let issues = |m: &Session| {
            let mut all: Vec<String> = m
                .issues
                .values()
                .flatten()
                .map(|i| format!("{i:?}"))
                .collect();

            all.sort();
            all
        };

        let stats = |m: &Session| format!("{:?}", m.stats);

        let (incremental_issues, incremental_stats) = (issues(&model), stats(&model));
        let incremental_status = (model.status("hello"), model.status("bye"));

        model.recompute();

        assert_eq!(issues(&model), incremental_issues);
        assert_eq!(stats(&model), incremental_stats);
        assert_eq!(
            (model.status("hello"), model.status("bye")),
            incremental_status
        );
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
