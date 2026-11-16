//! Format-preserving JSON editing.
//!
//! Translation files are owned by humans and by git. A normal parse/serialize
//! round-trip reorders keys, rewrites escapes and changes indentation, which
//! produces noisy diffs. This module parses JSON into a tree that remembers the
//! byte span of every node, and edits the original text in place. Everything
//! that is not touched keeps its exact bytes.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    pub key_start: usize,
    pub value: Node,
}

#[derive(Debug, Clone)]
pub enum Node {
    Object { span: Span, entries: Vec<Entry> },
    Array { span: Span, items: Vec<Node> },
    Str { span: Span, value: String },
    Other { span: Span },
}

impl Node {
    pub fn span(&self) -> Span {
        match self {
            Node::Object { span, .. }
            | Node::Array { span, .. }
            | Node::Str { span, .. }
            | Node::Other { span } => *span,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Node::Str { value, .. } => Some(value),
            _ => None,
        }
    }

    pub fn entries(&self) -> Option<&[Entry]> {
        match self {
            Node::Object { entries, .. } => Some(entries),
            _ => None,
        }
    }

    pub fn get(&self, seg: &Seg) -> Option<&Node> {
        match (self, seg) {
            (Node::Object { entries, .. }, Seg::Key(k)) => {
                entries.iter().find(|e| &e.key == k).map(|e| &e.value)
            }
            (Node::Array { items, .. }, Seg::Index(i)) => items.get(*i),
            _ => None,
        }
    }

    pub fn at(&self, path: &[Seg]) -> Option<&Node> {
        let mut node = self;
        for seg in path {
            node = node.get(seg)?;
        }
        Some(node)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

impl Seg {
    pub fn key(k: impl Into<String>) -> Self {
        Seg::Key(k.into())
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

pub fn parse(src: &str) -> Result<Node> {
    let mut p = Parser {
        src,
        bytes: src.as_bytes(),
        pos: 0,
    };
    // Tolerate a UTF-8 BOM.
    if src.starts_with('\u{feff}') {
        p.pos = 3;
    }
    p.ws();
    let node = p.value()?;
    p.ws();
    if p.pos != p.bytes.len() {
        bail!("unexpected trailing content at byte {}", p.pos);
    }
    Ok(node)
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while let Some(b) = self.bytes.get(self.pos) {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn expect(&mut self, b: u8) -> Result<()> {
        if self.peek() == Some(b) {
            self.pos += 1;
            Ok(())
        } else {
            Err(anyhow!("expected '{}' at byte {}", b as char, self.pos))
        }
    }

    fn value(&mut self) -> Result<Node> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => {
                let (span, value) = self.string()?;
                Ok(Node::Str { span, value })
            }
            Some(_) => self.scalar(),
            None => bail!("unexpected end of input"),
        }
    }

    fn object(&mut self) -> Result<Node> {
        let start = self.pos;
        self.expect(b'{')?;
        let mut entries = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Node::Object {
                span: Span {
                    start,
                    end: self.pos,
                },
                entries,
            });
        }
        loop {
            self.ws();
            let key_start = self.pos;
            let (_, key) = self.string()?;
            self.ws();
            self.expect(b':')?;
            self.ws();
            let value = self.value()?;
            entries.push(Entry {
                key,
                key_start,
                value,
            });
            self.ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => bail!("expected ',' or '}}' at byte {}", self.pos),
            }
        }
        Ok(Node::Object {
            span: Span {
                start,
                end: self.pos,
            },
            entries,
        })
    }

    fn array(&mut self) -> Result<Node> {
        let start = self.pos;
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Node::Array {
                span: Span {
                    start,
                    end: self.pos,
                },
                items,
            });
        }
        loop {
            self.ws();
            items.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => bail!("expected ',' or ']' at byte {}", self.pos),
            }
        }
        Ok(Node::Array {
            span: Span {
                start,
                end: self.pos,
            },
            items,
        })
    }

    fn string(&mut self) -> Result<(Span, String)> {
        let start = self.pos;
        self.expect(b'"')?;
        loop {
            match self.peek() {
                Some(b'\\') => self.pos += 2,
                Some(b'"') => {
                    self.pos += 1;
                    break;
                }
                Some(_) => self.pos += 1,
                None => bail!("unterminated string starting at byte {start}"),
            }
        }
        let raw = &self.src[start..self.pos];
        let value: String = serde_json::from_str(raw)
            .map_err(|e| anyhow!("invalid string at byte {start}: {e}"))?;
        Ok((
            Span {
                start,
                end: self.pos,
            },
            value,
        ))
    }

    fn scalar(&mut self) -> Result<Node> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if matches!(b, b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r') {
                break;
            }
            self.pos += 1;
        }
        let raw = &self.src[start..self.pos];
        serde_json::from_str::<Value>(raw)
            .map_err(|_| anyhow!("invalid value '{raw}' at byte {start}"))?;
        Ok(Node::Other {
            span: Span {
                start,
                end: self.pos,
            },
        })
    }
}

// ---------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------

/// A JSON document that is edited in place.
#[derive(Debug, Clone)]
pub struct JsonDoc {
    pub text: String,
}

impl JsonDoc {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    pub fn root(&self) -> Result<Node> {
        parse(&self.text)
    }

    #[cfg(test)]
    pub fn get_str(&self, path: &[Seg]) -> Option<String> {
        self.root().ok()?.at(path)?.as_str().map(str::to_owned)
    }

    /// Set the value at `path`. Missing object keys along the way are created.
    /// Missing array indices are an error.
    pub fn set(&mut self, path: &[Seg], value: &Value) -> Result<()> {
        self.set_payload(path, Payload::Value(value))
    }

    /// Like [`set`], but inserts `raw` JSON text verbatim (re-indented from
    /// `raw_indent` to wherever it lands). Used to copy a message's layout
    /// from another file.
    pub fn set_raw(&mut self, path: &[Seg], raw: &str, raw_indent: &str) -> Result<()> {
        parse(raw)?;
        self.set_payload(
            path,
            Payload::Raw {
                text: raw,
                indent: raw_indent,
            },
        )
    }

    fn set_payload(&mut self, path: &[Seg], payload: Payload) -> Result<()> {
        if path.is_empty() {
            bail!("cannot replace the document root");
        }
        let root = self.root()?;
        let unit = detect_indent_unit(&self.text);

        // Walk as far as the path exists.
        let mut node = &root;
        let mut depth = 0;
        while depth < path.len() {
            match node.get(&path[depth]) {
                Some(next) => {
                    node = next;
                    depth += 1;
                }
                None => break,
            }
        }

        if depth == path.len() {
            // Replace an existing value.
            let span = node.span();
            let indent = line_indent(&self.text, span.start);
            let rendered = payload.render(&unit, &indent);
            self.text.replace_range(span.start..span.end, &rendered);
            return Ok(());
        }

        // Insert into the deepest existing container.
        let Node::Object { span, entries } = node else {
            bail!("cannot insert into a non-object at {:?}", &path[..depth]);
        };
        let Seg::Key(first) = &path[depth] else {
            bail!(
                "cannot create a missing array index at {:?}",
                &path[..=depth]
            );
        };
        if depth + 1 < path.len() && matches!(payload, Payload::Raw { .. }) {
            bail!("raw values can only be inserted into an existing object");
        }
        let mut nested = match payload {
            Payload::Value(v) => v.clone(),
            Payload::Raw { .. } => Value::Null,
        };
        for seg in path[depth + 1..].iter().rev() {
            let Seg::Key(k) = seg else {
                bail!("cannot create a missing array index");
            };
            let mut map = serde_json::Map::new();
            map.insert(k.clone(), nested);
            nested = Value::Object(map);
        }

        let obj_indent = line_indent(&self.text, span.start);
        let key_json = serde_json::to_string(first)?;

        if let Some(last) = entries.last() {
            let first_entry = &entries[0];
            let multiline = self.text[span.start..first_entry.key_start].contains('\n');
            let pos = last.value.span().end;
            let insertion = if multiline {
                let entry_indent = line_indent(&self.text, first_entry.key_start);
                let rendered = payload.render_nested(&nested, &unit, &entry_indent);
                format!(",\n{entry_indent}{key_json}: {rendered}")
            } else {
                let rendered = match payload {
                    Payload::Raw { text, .. } => {
                        serde_json::to_string(&serde_json::from_str::<Value>(text)?)?
                    }
                    Payload::Value(_) => serde_json::to_string(&nested)?,
                };
                format!(", {key_json}: {rendered}")
            };
            self.text.insert_str(pos, &insertion);
        } else {
            let entry_indent = format!("{obj_indent}{unit}");
            let rendered = payload.render_nested(&nested, &unit, &entry_indent);
            let replacement = format!("{{\n{entry_indent}{key_json}: {rendered}\n{obj_indent}}}");
            self.text.replace_range(span.start..span.end, &replacement);
        }
        Ok(())
    }

    pub fn set_str(&mut self, path: &[Seg], value: &str) -> Result<()> {
        self.set(path, &Value::String(value.to_owned()))
    }

    /// Remove the object entry at `path`. Returns false if it did not exist.
    pub fn remove(&mut self, path: &[Seg]) -> Result<bool> {
        let Some((last, parent_path)) = path.split_last() else {
            bail!("cannot remove the document root");
        };
        let Seg::Key(key) = last else {
            bail!("removing array items is not supported");
        };
        let root = self.root()?;
        let Some(Node::Object { span, entries }) = root.at(parent_path) else {
            return Ok(false);
        };
        let Some(i) = entries.iter().position(|e| &e.key == key) else {
            return Ok(false);
        };
        let n = entries.len();
        let range = if n == 1 {
            (span.start + 1)..(span.end - 1)
        } else if i + 1 < n {
            entries[i].key_start..entries[i + 1].key_start
        } else {
            entries[i - 1].value.span().end..entries[i].value.span().end
        };
        self.text.replace_range(range, "");
        Ok(true)
    }
}

enum Payload<'a> {
    Value(&'a Value),
    Raw { text: &'a str, indent: &'a str },
}

impl Payload<'_> {
    fn render(&self, unit: &str, indent: &str) -> String {
        match self {
            Payload::Value(v) => render(v, unit, indent),
            Payload::Raw { text, indent: from } => reindent(text, from, indent),
        }
    }

    /// Render the value to insert. For `Value` payloads that is `nested`
    /// (the value wrapped in any missing parent objects).
    fn render_nested(&self, nested: &Value, unit: &str, indent: &str) -> String {
        match self {
            Payload::Value(_) => render(nested, unit, indent),
            Payload::Raw { .. } => self.render(unit, indent),
        }
    }
}

/// Move continuation lines of `text` from indent `from` to indent `to`.
fn reindent(text: &str, from: &str, to: &str) -> String {
    let mut lines = text.split('\n');
    let mut out = lines.next().unwrap_or_default().to_string();
    for line in lines {
        out.push('\n');
        match line.strip_prefix(from) {
            Some(rest) => {
                out.push_str(to);
                out.push_str(rest);
            }
            None => out.push_str(line),
        }
    }
    out
}

/// Render a value. Scalars render compactly; containers render pretty-printed
/// with the given indent unit, placed at `indent`.
fn render(value: &Value, unit: &str, indent: &str) -> String {
    match value {
        Value::Object(_) | Value::Array(_) => {
            let mut buf = Vec::new();
            let fmt = serde_json::ser::PrettyFormatter::with_indent(unit.as_bytes());
            let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
            serde::Serialize::serialize(value, &mut ser).expect("serializing a Value cannot fail");
            let s = String::from_utf8(buf).expect("serde_json emits UTF-8");
            s.replace('\n', &format!("\n{indent}"))
        }
        _ => serde_json::to_string(value).expect("serializing a Value cannot fail"),
    }
}

/// Leading whitespace of the line containing byte `pos`.
pub fn line_indent(text: &str, pos: usize) -> String {
    let line_start = text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    text[line_start..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// Guess the file's indent unit from its first indented line.
pub fn detect_indent_unit(text: &str) -> String {
    for line in text.lines() {
        let ws: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        if !ws.is_empty() && ws.len() < line.len() {
            if ws.starts_with('\t') {
                return "\t".into();
            }
            return ws;
        }
    }
    "  ".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn k(s: &str) -> Seg {
        Seg::key(s)
    }

    #[test]
    fn replaces_only_the_edited_value() {
        let src = "{\n    \"b\": \"B\",\n    \"a\":   \"caf\\u00e9\",\n    \"c\": 1\n}\n";
        let mut doc = JsonDoc::new(src);
        doc.set_str(&[k("b")], "Bee").unwrap();
        assert_eq!(
            doc.text,
            "{\n    \"b\": \"Bee\",\n    \"a\":   \"caf\\u00e9\",\n    \"c\": 1\n}\n"
        );
    }

    #[test]
    fn inserts_with_detected_indent() {
        let src = "{\n\t\"a\": \"A\"\n}\n";
        let mut doc = JsonDoc::new(src);
        doc.set_str(&[k("b")], "B").unwrap();
        assert_eq!(doc.text, "{\n\t\"a\": \"A\",\n\t\"b\": \"B\"\n}\n");
    }

    #[test]
    fn creates_nested_objects() {
        let src = "{\n  \"x\": {\n    \"a\": \"A\"\n  }\n}";
        let mut doc = JsonDoc::new(src);
        doc.set_str(&[k("x"), k("y"), k("z")], "Z").unwrap();
        assert_eq!(
            doc.text,
            "{\n  \"x\": {\n    \"a\": \"A\",\n    \"y\": {\n      \"z\": \"Z\"\n    }\n  }\n}"
        );
        assert_eq!(doc.get_str(&[k("x"), k("y"), k("z")]).as_deref(), Some("Z"));
    }

    #[test]
    fn inserts_into_empty_object() {
        let mut doc = JsonDoc::new("{}\n");
        doc.set_str(&[k("a")], "A").unwrap();
        assert_eq!(doc.text, "{\n  \"a\": \"A\"\n}\n");
    }

    #[test]
    fn keeps_single_line_style() {
        let mut doc = JsonDoc::new("{\"a\": \"A\"}");
        doc.set_str(&[k("b")], "B").unwrap();
        assert_eq!(doc.text, "{\"a\": \"A\", \"b\": \"B\"}");
    }

    #[test]
    fn removes_entries() {
        let src = "{\n  \"a\": \"A\",\n  \"b\": \"B\",\n  \"c\": \"C\"\n}";
        let mut doc = JsonDoc::new(src);
        assert!(doc.remove(&[k("b")]).unwrap());
        assert_eq!(doc.text, "{\n  \"a\": \"A\",\n  \"c\": \"C\"\n}");
        assert!(doc.remove(&[k("c")]).unwrap());
        assert_eq!(doc.text, "{\n  \"a\": \"A\"\n}");
        assert!(doc.remove(&[k("a")]).unwrap());
        assert_eq!(doc.text, "{}");
        assert!(!doc.remove(&[k("zzz")]).unwrap());
    }

    #[test]
    fn edits_inside_arrays() {
        let src = "{\n  \"n\": [{ \"match\": { \"c=one\": \"1\", \"c=other\": \"n\" } }]\n}";
        let mut doc = JsonDoc::new(src);
        let path = [k("n"), Seg::Index(0), k("match"), k("c=other")];
        doc.set_str(&path, "many").unwrap();
        assert!(doc.text.contains("\"c=other\": \"many\""));
        assert!(doc.text.contains("\"c=one\": \"1\""));
    }

    #[test]
    fn escapes_and_keeps_unicode() {
        let mut doc = JsonDoc::new("{\"a\": \"x\"}");
        doc.set_str(&[k("a")], "Fant \"{n}\" på nettet\n").unwrap();
        assert_eq!(doc.text, "{\"a\": \"Fant \\\"{n}\\\" på nettet\\n\"}");
        assert_eq!(doc.get_str(&[k("a")]).unwrap(), "Fant \"{n}\" på nettet\n");
    }

    #[test]
    fn inserts_complex_values() {
        let mut doc = JsonDoc::new("{\n  \"a\": \"A\"\n}");
        doc.set(&[k("b")], &json!([{ "match": { "x": "y" } }]))
            .unwrap();
        assert_eq!(
            doc.text,
            "{\n  \"a\": \"A\",\n  \"b\": [\n    {\n      \"match\": {\n        \"x\": \"y\"\n      }\n    }\n  ]\n}"
        );
    }

    #[test]
    fn inserts_raw_text_with_its_layout() {
        let mut doc = JsonDoc::new("{\n    \"a\": \"A\"\n}");
        let raw = "[{ \"x\": [\"1\", \"2\"],\n  \"m\": { \"k\": \"\" } }]";
        doc.set_raw(&[k("b")], raw, "").unwrap();
        assert_eq!(
            doc.text,
            "{\n    \"a\": \"A\",\n    \"b\": [{ \"x\": [\"1\", \"2\"],\n      \"m\": { \"k\": \"\" } }]\n}"
        );
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(parse("{\"a\": }").is_err());
        assert!(parse("{\"a\": \"b\"} x").is_err());
    }
}
