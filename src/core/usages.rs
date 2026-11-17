//! Finding where messages are used in the project's source code.
//!
//! This is pattern matching, not parsing. It understands the common ways
//! each library references a message:
//!
//! - Paraglide: `m.key()`, `m["key"]()`, and `import { key } from ".../paraglide/messages"`
//! - i18next: `t("key")`, `t("ns:key")`, `i18nKey="key"`, with the namespace
//!   taken from `useTranslation("ns")` in the same file
//! - next-intl: `t("key")` with the namespace from `useTranslations("ns")`
//!
//! Keys built at runtime, like ``t(`status.${state}`)``, can't be resolved.
//! Their fixed prefix (`status.`) is recorded, so messages starting with it
//! count as "possibly used" instead of unused.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use super::project::{Catalog, Format, Project};

/// One place in the source code that references a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// The message id, or the key as written if it doesn't resolve to one.
    pub id: String,
    /// Absolute path of the source file.
    pub file: PathBuf,
    /// 1-based line and column.
    pub line: usize,
    pub column: usize,
    /// The trimmed source line, for showing in a list.
    pub snippet: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageReport {
    /// Usages of known messages, by message id.
    pub usages: BTreeMap<String, Vec<Usage>>,
    /// Keys used in code that don't exist in the translation files.
    pub undefined: Vec<Usage>,
    /// Fixed beginnings of keys built at runtime.
    pub dynamic_prefixes: Vec<String>,
    pub files_scanned: usize,
}

impl UsageReport {
    /// Messages nothing references. The second list holds messages that
    /// match a runtime-built key's prefix, so they may still be in use.
    pub fn unused<'a>(&self, catalog: &'a Catalog) -> (Vec<&'a str>, Vec<&'a str>) {
        let mut unused = Vec::new();
        let mut maybe = Vec::new();

        for message in &catalog.messages {
            if self.usages.contains_key(&message.id) {
                continue;
            }

            if self.dynamic_prefix_for(&message.id).is_some() {
                maybe.push(message.id.as_str());
            } else {
                unused.push(message.id.as_str());
            }
        }

        (unused, maybe)
    }

    /// The runtime-built key prefix that could produce this message, like
    /// `status.` for ``t(`status.${state}`)``.
    pub fn dynamic_prefix_for(&self, id: &str) -> Option<&str> {
        let key = id.split_once(':').map_or(id, |(_, k)| k);

        self.dynamic_prefixes
            .iter()
            .find(|p| id.starts_with(p.as_str()) || key.starts_with(p.as_str()))
            .map(String::as_str)
    }
}

/// Source files worth looking at.
const EXTENSIONS: [&str; 12] = [
    "js", "jsx", "ts", "tsx", "mjs", "cjs", "mts", "cts", "svelte", "vue", "astro", "mdx",
];

/// Folders that never hold hand-written code. `paraglide` is the folder
/// Paraglide generates its message functions into; scanning it would make
/// every message look used.
const SKIPPED_DIRS: [&str; 10] = [
    "node_modules",
    "dist",
    "build",
    "target",
    "coverage",
    ".git",
    ".next",
    ".svelte-kit",
    "paraglide",
    "project.inlang",
];

const MAX_FILE_SIZE: u64 = 1024 * 1024;

/// Scan the project's folder for message usages.
pub fn scan(project: &Project, catalog: &Catalog) -> UsageReport {
    let context = Context::new(project, catalog);
    let mut report = UsageReport::default();

    for path in source_files(&project.root) {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };

        report.files_scanned += 1;
        scan_text(&context, &path, &text, &mut report);
    }

    report.dynamic_prefixes.sort();
    report.dynamic_prefixes.dedup();

    report
}

/// Every source file under `root`, honoring `.gitignore`.
fn source_files(root: &Path) -> Vec<PathBuf> {
    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .require_git(false)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !SKIPPED_DIRS.contains(&name.as_ref())
        })
        .build();

    let mut files = Vec::new();

    for entry in walker.flatten() {
        let path = entry.path();

        let wanted = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| EXTENSIONS.contains(&e));

        let small = entry
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() <= MAX_FILE_SIZE);

        if wanted && small {
            files.push(path.to_path_buf());
        }
    }

    files.sort();
    files
}

// -- Matching -----------------------------------------------------------------

/// What we know about the project while scanning.
struct Context {
    format: Format,
    known: HashSet<String>,
    namespaces: Vec<String>,
}

impl Context {
    fn new(project: &Project, catalog: &Catalog) -> Self {
        Self {
            format: project.format,
            known: catalog.messages.iter().map(|m| m.id.clone()).collect(),
            namespaces: project
                .files
                .iter()
                .filter_map(|f| f.namespace.clone())
                .collect(),
        }
    }
}

/// A key as written in the code, before it is resolved to a message id.
struct Reference {
    key: String,
    offset: usize,
    /// Only report it as undefined if we're sure it is meant as a message.
    certain: bool,
}

fn scan_text(context: &Context, path: &Path, text: &str, report: &mut UsageReport) {
    let references = match context.format {
        Format::MessageFormat => paraglide_references(text),
        Format::I18next | Format::NestedJson => t_function_references(text, report),
    };

    if references.is_empty() {
        return;
    }

    let namespace = namespace_hint(context.format, text);
    let lines = LineIndex::new(text);

    for reference in references {
        let resolved = resolve(context, &reference.key, namespace.as_deref());

        let (line, column) = lines.position(reference.offset);
        let usage = Usage {
            id: resolved.clone().unwrap_or_else(|| reference.key.clone()),
            file: path.to_path_buf(),
            line,
            column,
            snippet: lines.snippet(line),
        };

        match resolved {
            Some(id) => report.usages.entry(id).or_default().push(usage),
            None if reference.certain => report.undefined.push(usage),
            None => {}
        }
    }
}

/// Turn a key as written into a message id, if one exists.
fn resolve(context: &Context, key: &str, namespace: Option<&str>) -> Option<String> {
    let known = |id: &str| context.known.contains(id).then(|| id.to_string());

    match context.format {
        Format::MessageFormat => known(key),

        Format::I18next if context.namespaces.is_empty() => known(key),

        Format::I18next => {
            // Explicit namespace: `t("common:greeting")`.
            if key.contains(':') {
                return known(key);
            }

            // From `useTranslation("ns")`, then i18next's default namespace.
            let preferred = namespace.into_iter().chain(["translation"]);

            for ns in preferred {
                if let Some(id) = known(&format!("{ns}:{key}")) {
                    return Some(id);
                }
            }

            // Otherwise, accept it if exactly one namespace has the key.
            let mut matches = context
                .namespaces
                .iter()
                .filter_map(|ns| known(&format!("{ns}:{key}")));

            match (matches.next(), matches.next()) {
                (Some(id), None) => Some(id),
                _ => None,
            }
        }

        Format::NestedJson => {
            let scoped = namespace.and_then(|ns| known(&format!("{ns}.{key}")));
            scoped.or_else(|| known(key))
        }
    }
}

static PARAGLIDE_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"import\s*\{([^}]*)\}\s*from\s*["'][^"']*paraglide/messages[^"']*["']"#).unwrap()
});

static PARAGLIDE_NAMESPACE_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"import\s*\*\s*as\s+([A-Za-z_$][\w$]*)\s+from\s*["'][^"']*paraglide/messages[^"']*["']"#,
    )
    .unwrap()
});

/// `m.key` or `m["key"]`, called or not: message functions are often passed
/// around, as in `{ label: m.settings_title }`.
static MEMBER_ACCESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"([A-Za-z_$][\w$]*)\s*(?:\.\s*([A-Za-z_$][\w$]*)|\[\s*["']([^"'\n]+)["']\s*\])"#)
        .unwrap()
});

/// Paraglide: `m.key()`, `m["key"]()` and directly imported message functions.
fn paraglide_references(text: &str) -> Vec<Reference> {
    let imports_messages = text.contains("paraglide/messages");

    // Names the messages object goes by: `m`, plus `import * as x`.
    let mut objects: HashSet<String> = HashSet::from(["m".to_string()]);

    for capture in PARAGLIDE_NAMESPACE_IMPORT.captures_iter(text) {
        objects.insert(capture[1].to_string());
    }

    let mut references = Vec::new();

    // `m.key(...)` and `m["key"](...)`.
    for capture in MEMBER_ACCESS.captures_iter(text) {
        if !objects.contains(&capture[1]) {
            continue;
        }

        let Some(key) = capture.get(2).or_else(|| capture.get(3)) else {
            continue;
        };

        // `m` is also a common name for a local variable, as in
        // `items.map((m) => m.name)`. Only a call looks enough like a message
        // to report it as undefined when the key doesn't exist.
        let end = capture.get(0).map_or(0, |m| m.end());
        let called = text[end..].trim_start().starts_with('(');

        references.push(Reference {
            key: key.as_str().to_string(),
            offset: capture.get(0).map_or(0, |m| m.start()),
            certain: imports_messages && called,
        });
    }

    // `import { greeting, bye as farewell } from ".../paraglide/messages"`.
    for capture in PARAGLIDE_IMPORT.captures_iter(text) {
        let statement = capture.get(0).map_or(0..0, |m| m.range());

        for item in capture[1].split(',') {
            let mut parts = item.split(" as ").map(str::trim);

            let (Some(key), local) = (parts.next(), parts.next()) else {
                continue;
            };

            // Paraglide 2 exports the messages object itself as `m`.
            if key.is_empty() || key == "m" {
                continue;
            }

            let local = local.unwrap_or(key);
            let name = Regex::new(&format!(r"\b{}\b", regex::escape(local))).unwrap();

            // Every mention outside the import itself, called or not.
            for found in name.find_iter(text) {
                if statement.contains(&found.start()) {
                    continue;
                }

                references.push(Reference {
                    key: key.to_string(),
                    offset: found.start(),
                    certain: true,
                });
            }
        }
    }

    references.sort_by_key(|r| r.offset);
    references
}

static T_CALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\$?\bt\(\s*(?:"([^"\n]*)"|'([^'\n]*)'|`([^`]*)`)"#).unwrap());

static I18N_KEY_PROP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"i18nKey\s*=\s*\{?\s*(?:"([^"\n]*)"|'([^'\n]*)')"#).unwrap());

/// i18next and next-intl: `t("key")`, `$t("key")` and `i18nKey="key"`.
fn t_function_references(text: &str, report: &mut UsageReport) -> Vec<Reference> {
    let mut references = Vec::new();

    for regex in [&*T_CALL, &*I18N_KEY_PROP] {
        for capture in regex.captures_iter(text) {
            let Some(key) = capture
                .get(1)
                .or_else(|| capture.get(2))
                .or_else(|| capture.get(3))
            else {
                continue;
            };

            let key = key.as_str();

            // A key built at runtime: remember its fixed beginning.
            if let Some((prefix, _)) = key.split_once("${") {
                if !prefix.is_empty() {
                    report.dynamic_prefixes.push(prefix.to_string());
                }

                continue;
            }

            if key.is_empty() {
                continue;
            }

            references.push(Reference {
                key: key.to_string(),
                offset: capture.get(0).map_or(0, |m| m.start()),
                certain: true,
            });
        }
    }

    references.sort_by_key(|r| r.offset);
    references
}

static USE_TRANSLATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:useTranslation|getFixedT\([^,)]*,)\s*\(?\s*\[?\s*["']([^"']+)["']"#).unwrap()
});

static USE_TRANSLATIONS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:useTranslations|getTranslations)\(\s*(?:\{\s*namespace\s*:\s*)?["']([^"']+)["']"#,
    )
    .unwrap()
});

/// The namespace a file's `t` is bound to, if it says so.
fn namespace_hint(format: Format, text: &str) -> Option<String> {
    let regex = match format {
        Format::I18next => &*USE_TRANSLATION,
        Format::NestedJson => &*USE_TRANSLATIONS,
        Format::MessageFormat => return None,
    };

    regex.captures(text).map(|c| c[1].to_string())
}

// -- Positions ----------------------------------------------------------------

struct LineIndex<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));

        Self { text, starts }
    }

    /// 1-based line and column of a byte offset.
    fn position(&self, offset: usize) -> (usize, usize) {
        let line = self.starts.partition_point(|start| *start <= offset);
        let column = self.text[self.starts[line - 1]..offset].chars().count() + 1;

        (line, column)
    }

    fn snippet(&self, line: usize) -> String {
        let start = self.starts[line - 1];
        let end = self.starts.get(line).map_or(self.text.len(), |s| s - 1);

        let trimmed = self.text[start..end].trim();
        let mut snippet: String = trimmed.chars().take(120).collect();

        if trimmed.chars().count() > 120 {
            snippet.push('…');
        }

        snippet
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn load(dir: &Path) -> (Project, Catalog) {
        let project = Project::load(&dir.join("project.inlang")).unwrap();
        let catalog = project.read_messages().unwrap();

        (project, catalog)
    }

    #[test]
    fn finds_paraglide_usages() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();

        write(
            d,
            "project.inlang/settings.json",
            r#"{ "baseLocale": "en", "locales": ["en"],
                 "plugin.inlang.messageFormat": { "pathPattern": "./messages/{locale}.json" } }"#,
        );

        write(
            d,
            "messages/en.json",
            r#"{ "hello": "Hi", "bye": "Bye", "unused_one": "x" }"#,
        );

        write(
            d,
            "src/App.tsx",
            "import { m } from './paraglide/messages.js';\n\
             export const A = () => <h1>{m.hello()}</h1>;\n\
             const b = m[\"bye\"]();\n\
             const c = m.missing_key();\n\
             const nav = { label: m.unused_one };\n\
             const names = items.map((m) => m.name);\n",
        );

        write(
            d,
            "src/other.ts",
            "import { bye as farewell } from '$lib/paraglide/messages';\nfarewell();\n",
        );

        // Generated code must not count.
        write(
            d,
            "src/paraglide/messages/hello.js",
            "export const hello = () => 'Hi';\nm.hello();\n",
        );

        let (project, catalog) = load(d);
        let report = scan(&project, &catalog);

        let hello = &report.usages["hello"];
        assert_eq!(hello.len(), 1);
        assert_eq!((hello[0].line, hello[0].column), (2, 29));
        assert!(hello[0].file.ends_with("src/App.tsx"));

        assert_eq!(report.usages["bye"].len(), 2);
        assert_eq!(report.undefined.len(), 1);
        assert_eq!(report.undefined[0].id, "missing_key");

        // Passed around without being called still counts.
        assert_eq!(report.usages["unused_one"].len(), 1);

        let (unused, maybe) = report.unused(&catalog);
        assert!(unused.is_empty());
        assert!(maybe.is_empty());
    }

    #[test]
    fn resolves_i18next_namespaces_and_dynamic_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();

        write(
            d,
            "project.inlang/settings.json",
            r#"{ "baseLocale": "en", "locales": ["en"],
                 "plugin.inlang.i18next": { "pathPattern": {
                     "common": "./locales/{locale}/common.json",
                     "printers": "./locales/{locale}/printers.json" } } }"#,
        );

        write(
            d,
            "locales/en/common.json",
            r#"{ "greeting": "Hi", "items_one": "1", "items_other": "n" }"#,
        );

        write(
            d,
            "locales/en/printers.json",
            r#"{ "status": { "online": "On", "offline": "Off" }, "title": "Printers" }"#,
        );

        write(
            d,
            "src/Printers.tsx",
            "const { t } = useTranslation('printers');\n\
             t('title');\n\
             t(`status.${state}`);\n\
             t('common:items', { count: 2 });\n\
             <Trans i18nKey=\"greeting\" />\n\
             t('nope');\n",
        );

        // Ignored by .gitignore.
        write(d, ".gitignore", "generated/\n");
        write(d, "generated/x.ts", "t('title');\n");

        let (project, catalog) = load(d);
        let report = scan(&project, &catalog);

        assert_eq!(report.usages["printers:title"].len(), 1);
        assert_eq!(report.usages["common:items"].len(), 1);
        assert_eq!(report.usages["common:greeting"].len(), 1);
        assert_eq!(report.dynamic_prefixes, vec!["status."]);
        assert_eq!(report.undefined[0].id, "nope");

        let (unused, maybe) = report.unused(&catalog);
        assert!(unused.is_empty());
        assert_eq!(
            maybe,
            vec!["printers:status.online", "printers:status.offline"]
        );
    }
}
