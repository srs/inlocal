//! Reading and writing an inlang project (`project.inlang/settings.json`) and
//! the translation files it points to.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::jsonedit::{parse, JsonDoc, Node, Seg};

const MESSAGE_FORMAT_SCHEMA: &str = "https://inlang.com/schema/inlang-message-format";
const PLURAL_CATEGORIES: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];

/// How messages are laid out in the translation files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::enum_variant_names)]
pub enum Format {
    /// `@inlang/plugin-message-format` (Paraglide). Flat keys; a value is either
    /// a string or a list of variants with a `match` object.
    MessageFormat,
    /// `@inlang/plugin-i18next`. Nested keys, `{{var}}` placeholders and
    /// `_one` / `_other` plural suffixes.
    I18next,
    /// Any other plugin with a `pathPattern` (for example next-intl). Treated as
    /// nested JSON with `{var}` placeholders.
    NestedJson,
}

impl Format {
    /// Short name for display.
    pub fn label(&self) -> &'static str {
        match self {
            Format::MessageFormat => "messageFormat",
            Format::I18next => "i18next",
            Format::NestedJson => "nestedJson",
        }
    }

    /// How a variable is written in this format: `{name}` or `{{name}}`.
    pub fn placeholder(&self, name: &str) -> String {
        match self {
            Format::I18next => format!("{{{{{name}}}}}"),
            _ => format!("{{{name}}}"),
        }
    }

    pub fn variable_regex(&self) -> &'static str {
        match self {
            Format::I18next => r"\{\{\s*-?\s*([A-Za-z_][\w.]*)[^}]*\}\}",
            _ => r"\{\s*([A-Za-z_]\w*)\s*[,}]",
        }
    }
}

/// One translation file pattern, optionally tied to an i18next namespace.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSet {
    pub namespace: Option<String>,
    pub path_pattern: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// The `project.inlang` directory.
    pub dir: PathBuf,
    /// The directory `pathPattern`s are relative to (the parent of `dir`).
    pub root: PathBuf,
    pub base_locale: String,
    pub locales: Vec<String>,
    pub format: Format,
    pub files: Vec<FileSet>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSettings {
    base_locale: String,
    locales: Vec<String>,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}

impl Project {
    pub fn load(dir: &Path) -> Result<Self> {
        let dir = dir
            .canonicalize()
            .with_context(|| format!("cannot open {}", dir.display()))?;
        let settings_path = dir.join("settings.json");
        let raw = fs::read_to_string(&settings_path)
            .with_context(|| format!("cannot read {}", settings_path.display()))?;
        let settings: RawSettings = serde_json::from_str(&raw)
            .with_context(|| format!("invalid {}", settings_path.display()))?;
        let root = dir.parent().unwrap_or(&dir).to_path_buf();

        let (format, files) = detect_format(&settings.rest)?;
        let mut locales = settings.locales;
        if !locales.contains(&settings.base_locale) {
            locales.insert(0, settings.base_locale.clone());
        }
        Ok(Project {
            dir,
            root,
            base_locale: settings.base_locale,
            locales,
            format,
            files,
        })
    }

    pub fn file_path(&self, set: &FileSet, locale: &str) -> PathBuf {
        let rel = set
            .path_pattern
            .replace("{locale}", locale)
            .replace("{languageTag}", locale);
        let rel = rel.strip_prefix("./").unwrap_or(&rel);
        self.root.join(rel)
    }

    /// Every file that may hold translations, for the file watcher.
    pub fn all_files(&self) -> Vec<PathBuf> {
        let mut out = vec![self.dir.join("settings.json")];
        for set in &self.files {
            for locale in &self.locales {
                out.push(self.file_path(set, locale));
            }
        }
        out
    }

    /// Read every translation file into a list of messages.
    pub fn read_messages(&self) -> Result<Catalog> {
        let mut catalog = Catalog::default();
        for set in &self.files {
            for locale in &self.locales {
                let path = self.file_path(set, locale);
                if !path.exists() {
                    continue;
                }
                let text = fs::read_to_string(&path)
                    .with_context(|| format!("cannot read {}", path.display()))?;
                let root =
                    parse(&text).with_context(|| format!("invalid JSON in {}", path.display()))?;
                let mut leaves = Vec::new();
                match self.format {
                    Format::MessageFormat => collect_message_format(&root, &mut leaves),
                    Format::I18next => collect_nested(&root, "", true, &mut leaves),
                    Format::NestedJson => collect_nested(&root, "", false, &mut leaves),
                }
                for (id, variant, value) in leaves {
                    let id = match &set.namespace {
                        Some(ns) => format!("{ns}:{id}"),
                        None => id,
                    };
                    catalog.insert(&id, &variant, locale, value, locale == &self.base_locale);
                }
            }
        }
        catalog.finish();
        Ok(catalog)
    }

    /// Apply edits to the files on disk. Untouched bytes are preserved.
    pub fn apply_edits(&self, edits: &[Edit]) -> Result<Vec<PathBuf>> {
        let mut docs: BTreeMap<PathBuf, JsonDoc> = BTreeMap::new();
        for edit in edits {
            if !self.locales.contains(&edit.locale) {
                bail!("unknown locale '{}'", edit.locale);
            }
            let (set, key) = self.split_namespace(&edit.id)?;
            let path = self.file_path(set, &edit.locale);
            if !docs.contains_key(&path) {
                let text = if path.exists() {
                    fs::read_to_string(&path)?
                } else if self.format == Format::MessageFormat {
                    format!("{{\n  \"$schema\": \"{MESSAGE_FORMAT_SCHEMA}\"\n}}\n")
                } else {
                    "{}\n".to_string()
                };
                docs.insert(path.clone(), JsonDoc::new(text));
            }
            let doc = docs.get_mut(&path).expect("inserted above");
            let json_path = self.json_path(key, &edit.variant);

            match edit.value.as_deref() {
                None | Some("") => {
                    if self.format == Format::MessageFormat && !edit.variant.is_empty() {
                        // Keep the variant structure; clear the text.
                        if doc.root()?.at(&json_path).is_some() {
                            doc.set_str(&json_path, "")?;
                        }
                    } else {
                        doc.remove(&json_path)?;
                    }
                }
                Some(value) => {
                    if self.format == Format::MessageFormat
                        && !edit.variant.is_empty()
                        && doc.root()?.at(&[Seg::key(key)]).is_none()
                    {
                        // The message is missing in this locale. Copy the base
                        // locale's variant structure with empty texts first.
                        let (raw, indent) = self.message_format_skeleton(set, key)?;
                        doc.set_raw(&[Seg::key(key)], &raw, &indent)?;
                    }
                    doc.set_str(&json_path, value)?;
                }
            }
        }

        let mut written = Vec::new();
        for (path, doc) in docs {
            parse(&doc.text)
                .with_context(|| format!("edit produced invalid JSON for {}", path.display()))?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let tmp = path.with_extension("json.inlocal-tmp");
            fs::write(&tmp, &doc.text)?;
            fs::rename(&tmp, &path)?;
            written.push(path);
        }
        Ok(written)
    }

    /// Where a translation lives: its file, and the 1-based line of its key
    /// if it exists in that file. Used for editor links and CI annotations.
    pub fn locate(&self, id: &str, variant: &str, locale: &str) -> Option<Location> {
        let (set, key) = self.split_namespace(id).ok()?;
        let file = self.file_path(set, locale);

        let line = fs::read_to_string(&file).ok().and_then(|text| {
            let root = parse(&text).ok()?;

            // Point at the variant if it exists, otherwise at the message.
            let full = self.json_path(key, variant);
            let node = root.at(&full).or_else(|| root.at(&full[..1]))?;

            let start = node.span().start;
            Some(text[..start].matches('\n').count() + 1)
        });

        Some(Location { file, line })
    }

    fn split_namespace<'a>(&'a self, id: &'a str) -> Result<(&'a FileSet, &'a str)> {
        if self.files.iter().any(|f| f.namespace.is_some()) {
            if let Some((ns, key)) = id.split_once(':') {
                if let Some(set) = self
                    .files
                    .iter()
                    .find(|f| f.namespace.as_deref() == Some(ns))
                {
                    return Ok((set, key));
                }
            }
            bail!("message '{id}' does not belong to a known namespace");
        }
        Ok((&self.files[0], id))
    }

    fn json_path(&self, key: &str, variant: &str) -> Vec<Seg> {
        match self.format {
            Format::MessageFormat => {
                if variant.is_empty() {
                    vec![Seg::key(key)]
                } else {
                    vec![
                        Seg::key(key),
                        Seg::Index(0),
                        Seg::key("match"),
                        Seg::key(variant),
                    ]
                }
            }
            Format::I18next | Format::NestedJson => {
                let mut segs: Vec<Seg> = key.split('.').map(Seg::key).collect();
                if !variant.is_empty() {
                    if let Some(Seg::Key(last)) = segs.last_mut() {
                        last.push('_');
                        last.push_str(variant);
                    }
                }
                segs
            }
        }
    }

    /// The base locale's raw JSON text for a variant message, with every
    /// `match` text emptied, plus the indent of the line it starts on.
    fn message_format_skeleton(&self, set: &FileSet, key: &str) -> Result<(String, String)> {
        let base_path = self.file_path(set, &self.base_locale);
        let text = fs::read_to_string(&base_path)?;
        let root = parse(&text)?;
        let node = root
            .at(&[Seg::key(key)])
            .context("message missing in base locale")?;
        let span = node.span();
        let mut doc = JsonDoc::new(&text[span.start..span.end]);
        let indent = crate::jsonedit::line_indent(&text, span.start);
        let match_path = [Seg::Index(0), Seg::key("match")];
        let keys: Vec<String> = doc
            .root()?
            .at(&match_path)
            .and_then(|m| {
                m.entries()
                    .map(|e| e.iter().map(|x| x.key.clone()).collect())
            })
            .unwrap_or_default();
        for k in keys {
            doc.set_str(&[Seg::Index(0), Seg::key("match"), Seg::key(k)], "")?;
        }
        Ok((doc.text, indent))
    }
}

fn detect_format(rest: &serde_json::Map<String, Value>) -> Result<(Format, Vec<FileSet>)> {
    let pattern_of = |v: &Value| -> Option<Vec<FileSet>> {
        match v.get("pathPattern")? {
            Value::String(s) => Some(vec![FileSet {
                namespace: None,
                path_pattern: s.clone(),
            }]),
            Value::Array(a) => a.first().and_then(Value::as_str).map(|s| {
                vec![FileSet {
                    namespace: None,
                    path_pattern: s.to_string(),
                }]
            }),
            Value::Object(m) => Some(
                m.iter()
                    .filter_map(|(ns, p)| {
                        Some(FileSet {
                            namespace: Some(ns.clone()),
                            path_pattern: p.as_str()?.to_string(),
                        })
                    })
                    .collect(),
            ),
            _ => None,
        }
    };

    if let Some(files) = rest.get("plugin.inlang.messageFormat").and_then(pattern_of) {
        return Ok((Format::MessageFormat, files));
    }
    if let Some(files) = rest.get("plugin.inlang.i18next").and_then(pattern_of) {
        return Ok((Format::I18next, files));
    }
    for (key, value) in rest {
        if key.starts_with("plugin.") {
            if let Some(files) = pattern_of(value) {
                return Ok((Format::NestedJson, files));
            }
        }
    }
    bail!("settings.json has no plugin with a pathPattern (expected plugin.inlang.messageFormat or plugin.inlang.i18next)")
}

/// (message id, variant, text)
type Leaf = (String, String, String);

fn collect_message_format(root: &Node, out: &mut Vec<Leaf>) {
    let Some(entries) = root.entries() else {
        return;
    };
    for entry in entries {
        if entry.key.starts_with('$') {
            continue;
        }
        match &entry.value {
            Node::Str { value, .. } => out.push((entry.key.clone(), String::new(), value.clone())),
            Node::Array { .. } => {
                let Some(matches) = entry.value.at(&[Seg::Index(0), Seg::key("match")]) else {
                    continue;
                };
                for m in matches.entries().unwrap_or_default() {
                    if let Some(text) = m.value.as_str() {
                        out.push((entry.key.clone(), m.key.clone(), text.to_owned()));
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_nested(node: &Node, prefix: &str, plural_suffixes: bool, out: &mut Vec<Leaf>) {
    let Some(entries) = node.entries() else {
        return;
    };
    for entry in entries {
        if prefix.is_empty() && entry.key.starts_with('$') {
            continue;
        }
        let path = if prefix.is_empty() {
            entry.key.clone()
        } else {
            format!("{prefix}.{}", entry.key)
        };
        match &entry.value {
            Node::Object { .. } => collect_nested(&entry.value, &path, plural_suffixes, out),
            Node::Str { value, .. } => {
                let (id, variant) = if plural_suffixes {
                    split_plural(&path)
                } else {
                    (path.clone(), String::new())
                };
                out.push((id, variant, value.clone()));
            }
            _ => {}
        }
    }
}

fn split_plural(path: &str) -> (String, String) {
    for cat in PLURAL_CATEGORIES {
        if let Some(id) = path.strip_suffix(&format!("_{cat}")) {
            if !id.is_empty() {
                return (id.to_string(), cat.to_string());
            }
        }
    }
    (path.to_string(), String::new())
}

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub file: PathBuf,
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edit {
    pub id: String,
    #[serde(default)]
    pub variant: String,
    pub locale: String,
    /// `None` or empty removes the translation.
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    /// Variant names in display order. A single empty name means a plain message.
    pub variants: Vec<String>,
    /// locale -> variant -> text
    pub values: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Catalog {
    pub messages: Vec<Message>,
}

impl Catalog {
    fn insert(&mut self, id: &str, variant: &str, locale: &str, value: String, is_base: bool) {
        let msg = match self.messages.iter_mut().position(|m| m.id == id) {
            Some(i) => &mut self.messages[i],
            None => {
                self.messages.push(Message {
                    id: id.to_string(),
                    variants: vec![],
                    values: BTreeMap::new(),
                });
                self.messages.last_mut().expect("just pushed")
            }
        };
        if is_base && !msg.variants.iter().any(|v| v == variant) {
            msg.variants.push(variant.to_string());
        }
        msg.values
            .entry(locale.to_string())
            .or_default()
            .insert(variant.to_string(), value);
    }

    fn finish(&mut self) {
        // Messages that only exist in other locales still need variants.
        for msg in &mut self.messages {
            if msg.variants.is_empty() {
                let mut seen: Vec<String> = Vec::new();
                for values in msg.values.values() {
                    for v in values.keys() {
                        if !seen.contains(v) {
                            seen.push(v.clone());
                        }
                    }
                }
                msg.variants = seen;
            }
            sort_variants(&mut msg.variants);
        }
    }
}

fn sort_variants(variants: &mut [String]) {
    let rank = |v: &str| {
        let cat = v.rsplit('=').next().unwrap_or(v);
        PLURAL_CATEGORIES
            .iter()
            .position(|c| *c == cat)
            .unwrap_or(PLURAL_CATEGORIES.len())
    };
    variants.sort_by_key(|v| rank(v));
}

/// Look for a `project.inlang` directory at `start`, in its ancestors, or a
/// few levels below it (skipping `node_modules` and hidden directories).
pub fn find_project(start: &Path) -> Option<PathBuf> {
    if start.file_name().is_some_and(|n| n == "project.inlang")
        && start.join("settings.json").exists()
    {
        return Some(start.to_path_buf());
    }
    for dir in start.ancestors() {
        let candidate = dir.join("project.inlang");
        if candidate.join("settings.json").exists() {
            return Some(candidate);
        }
    }
    fn search(dir: &Path, depth: usize) -> Option<PathBuf> {
        if depth == 0 {
            return None;
        }
        let mut dirs: Vec<PathBuf> = fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for p in &dirs {
            let name = p.file_name()?.to_string_lossy();
            if name == "project.inlang" && p.join("settings.json").exists() {
                return Some(p.clone());
            }
        }
        for p in dirs {
            let name = p.file_name()?.to_string_lossy().to_string();
            if name.starts_with('.') || name == "node_modules" || name == "target" || name == "dist"
            {
                continue;
            }
            if let Some(found) = search(&p, depth - 1) {
                return Some(found);
            }
        }
        None
    }
    search(start, 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    #[test]
    fn message_format_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(
            d,
            "project.inlang/settings.json",
            r#"{
  "baseLocale": "en", "locales": ["en", "nb"],
  "plugin.inlang.messageFormat": { "pathPattern": "./messages/{locale}.json" }
}"#,
        );
        write(
            d,
            "messages/en.json",
            r#"{
  "$schema": "https://inlang.com/schema/inlang-message-format",
  "hello": "Hello {name}",
  "count": [{ "declarations": ["input count", "local c = count: plural"], "selectors": ["c"],
    "match": { "c=one": "{count} part", "c=other": "{count} parts" } }]
}
"#,
        );
        write(d, "messages/nb.json", "{\n  \"hello\": \"Hei {name}\"\n}\n");

        let project = Project::load(&d.join("project.inlang")).unwrap();
        assert_eq!(project.format, Format::MessageFormat);
        let cat = project.read_messages().unwrap();
        let count = cat.messages.iter().find(|m| m.id == "count").unwrap();
        assert_eq!(count.variants, vec!["c=one", "c=other"]);

        project
            .apply_edits(&[Edit {
                id: "count".into(),
                variant: "c=other".into(),
                locale: "nb".into(),
                value: Some("{count} deler".into()),
            }])
            .unwrap();
        let nb = fs::read_to_string(d.join("messages/nb.json")).unwrap();
        assert!(nb.starts_with("{\n  \"hello\": \"Hei {name}\",\n  \"count\": ["));
        let cat = project.read_messages().unwrap();
        let count = cat.messages.iter().find(|m| m.id == "count").unwrap();
        assert_eq!(count.values["nb"]["c=other"], "{count} deler");

        let found = project.locate("count", "c=other", "en").unwrap();
        assert_eq!(found.file, project.root.join("messages/en.json"));
        assert_eq!(found.line, Some(5));

        let hello = project.locate("hello", "", "nb").unwrap();
        assert_eq!(hello.line, Some(2));
        assert_eq!(count.values["nb"]["c=one"], "");
    }

    #[test]
    fn i18next_namespaces_and_plurals() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(
            d,
            "project.inlang/settings.json",
            r#"{
  "baseLocale": "en", "locales": ["en", "de"],
  "plugin.inlang.i18next": { "pathPattern": { "common": "./locales/{locale}/common.json" } }
}"#,
        );
        write(
            d,
            "locales/en/common.json",
            r#"{ "parts": { "count_one": "{{count}} part", "count_other": "{{count}} parts" } }"#,
        );

        let project = Project::load(&d.join("project.inlang")).unwrap();
        let cat = project.read_messages().unwrap();
        assert_eq!(cat.messages[0].id, "common:parts.count");
        assert_eq!(cat.messages[0].variants, vec!["one", "other"]);

        project
            .apply_edits(&[Edit {
                id: "common:parts.count".into(),
                variant: "one".into(),
                locale: "de".into(),
                value: Some("{{count}} Teil".into()),
            }])
            .unwrap();
        let de = fs::read_to_string(d.join("locales/de/common.json")).unwrap();
        assert_eq!(
            de,
            "{\n  \"parts\": {\n    \"count_one\": \"{{count}} Teil\"\n  }\n}\n"
        );
    }
}
