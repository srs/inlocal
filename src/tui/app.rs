//! TUI state and key handling. Drawing lives in `ui.rs`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::core::session::is_missing;
use crate::core::usages::{Usage, UsageReport};
use crate::core::{FieldKey, Filter, Message, Session};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::input::TextInput;

/// Which part of the screen has the keyboard.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Fields,
    Editing,
    Search,
    /// Waiting for y/n before throwing away unsaved edits.
    Confirm(ConfirmAction),
}

/// The two layouts: one message at a time, or two languages side by side.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Focus,
    Compare,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    Quit,
    Discard,
    Reload,
}

pub struct App {
    pub session: Session,
    pub focus: Focus,
    pub view: View,

    pub filter: Filter,
    pub namespace: Option<String>,
    pub search: TextInput,

    pub visible: Vec<String>,
    pub selected: usize,
    pub field: usize,
    pub editor: TextInput,

    // Compare view: one row per message variant, `from` next to `to`.
    pub from: String,
    pub to: String,
    pub only_missing: bool,
    pub row: usize,

    pub message: Option<String>,
    pub changed_on_disk: bool,
    pub quit: bool,

    /// Where messages are used in the source code, and a usage the user
    /// asked to open (the event loop opens it, since it owns the terminal).
    pub usages: UsageReport,
    pub open_request: Option<Usage>,

    files_changed: Arc<AtomicBool>,
}

impl App {
    pub fn new(session: Session, files_changed: Arc<AtomicBool>) -> Self {
        let project = session.project.clone();
        let from = project.base_locale.clone();
        let to = project
            .locales
            .iter()
            .find(|l| **l != from)
            .cloned()
            .unwrap_or_else(|| from.clone());

        let mut app = Self {
            session,
            focus: Focus::List,
            view: View::Focus,
            filter: Filter::All,
            namespace: None,
            search: TextInput::default(),
            visible: Vec::new(),
            selected: 0,
            field: 0,
            editor: TextInput::default(),
            from,
            to,
            only_missing: false,
            row: 0,
            message: None,
            changed_on_disk: false,
            quit: false,
            usages: UsageReport::default(),
            open_request: None,
            files_changed,
        };

        app.refresh_visible();
        app.rescan_usages();

        app
    }

    // -- What to show --------------------------------------------------------

    pub fn locales(&self) -> Vec<String> {
        let project = &self.session.project;
        let base = project.base_locale.clone();

        let mut locales = vec![base.clone()];
        locales.extend(project.locales.iter().filter(|l| **l != base).cloned());

        locales
    }

    pub fn namespaces(&self) -> Vec<String> {
        self.session
            .project
            .files
            .iter()
            .filter_map(|f| f.namespace.clone())
            .collect()
    }

    pub fn current(&self) -> Option<&Message> {
        let id = self.visible.get(self.selected)?;
        self.session.message(id)
    }

    /// Every field of the selected message, in display order.
    pub fn fields(&self) -> Vec<FieldKey> {
        let Some(message) = self.current() else {
            return Vec::new();
        };

        let mut keys = Vec::new();

        for locale in self.locales() {
            for variant in &message.variants {
                keys.push(FieldKey::new(&message.id, variant, &locale));
            }
        }

        keys
    }

    pub fn current_field(&self) -> Option<FieldKey> {
        match self.view {
            View::Focus => self.fields().into_iter().nth(self.field),
            View::Compare => self.compare_rows().into_iter().nth(self.row),
        }
    }

    /// Compare view rows: every variant of every listed message, in the
    /// target locale. With `only_missing`, just the ones still to translate.
    pub fn compare_rows(&self) -> Vec<FieldKey> {
        let mut rows = Vec::new();

        for id in &self.visible {
            let Some(message) = self.session.message(id) else {
                continue;
            };

            for variant in &message.variants {
                let key = FieldKey::new(id, variant, &self.to);

                if self.only_missing && !self.has_missing(&key) {
                    continue;
                }

                rows.push(key);
            }
        }

        rows
    }

    fn refresh_visible(&mut self) {
        let selected_id = self.visible.get(self.selected).cloned();

        self.visible =
            self.session
                .visible_ids(self.filter, self.namespace.as_deref(), self.search.text());

        // Keep the same message selected if it is still in the list.
        self.selected = selected_id
            .and_then(|id| self.visible.iter().position(|v| *v == id))
            .unwrap_or(0);

        self.field = self.field.min(self.fields().len().saturating_sub(1));
    }

    // -- Ticks and keys ------------------------------------------------------

    /// Called regularly from the event loop.
    pub fn tick(&mut self) {
        if !self.files_changed.swap(false, Ordering::Relaxed) {
            return;
        }

        if !self.session.changed_on_disk() {
            return;
        }

        if self.session.has_edits() {
            self.changed_on_disk = true;
        } else {
            self.reload();
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Save works everywhere except while typing in a field.
        if ctrl && key.code == KeyCode::Char('s') && self.focus != Focus::Editing {
            self.save();

            return;
        }

        if ctrl && key.code == KeyCode::Char('c') {
            self.request(ConfirmAction::Quit);

            return;
        }

        match self.focus {
            Focus::List if self.view == View::Compare => self.on_compare_key(key),
            Focus::List => self.on_list_key(key),
            Focus::Fields => self.on_fields_key(key),
            Focus::Editing => self.on_editing_key(key),
            Focus::Search => self.on_search_key(key),
            Focus::Confirm(action) => self.on_confirm_key(key, action),
        }
    }

    fn on_list_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::Char('g') | KeyCode::Home => self.selected = 0,
            KeyCode::Char('G') | KeyCode::End => {
                self.selected = self.visible.len().saturating_sub(1)
            }

            KeyCode::Enter | KeyCode::Tab | KeyCode::Char('l') | KeyCode::Right => {
                if self.current().is_some() {
                    self.focus = Focus::Fields;
                    self.field = self.first_field_to_translate();

                    // The code may have changed since we last looked.
                    self.rescan_usages();
                }
            }

            KeyCode::Char('v') => self.toggle_view(),
            KeyCode::Char('/') => self.focus = Focus::Search,
            KeyCode::Char('1') => self.set_filter(Filter::All),
            KeyCode::Char('2') => self.set_filter(Filter::NeedsAttention),
            KeyCode::Char('3') => self.set_filter(Filter::Missing),
            KeyCode::Char('4') => self.set_filter(Filter::Changed),
            KeyCode::Char('n') => self.cycle_namespace(),

            KeyCode::Char('s') => self.save(),
            KeyCode::Char('u') => self.request(ConfirmAction::Discard),
            KeyCode::Char('r') => self.request(ConfirmAction::Reload),
            KeyCode::Char('q') | KeyCode::Esc => self.request(ConfirmAction::Quit),
            _ => {}
        }
    }

    fn on_compare_key(&mut self, key: KeyEvent) {
        let last = self.compare_rows().len().saturating_sub(1);

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.row = (self.row + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => self.row = self.row.saturating_sub(1),
            KeyCode::PageDown => self.row = (self.row + 10).min(last),
            KeyCode::PageUp => self.row = self.row.saturating_sub(10),
            KeyCode::Char('g') | KeyCode::Home => self.row = 0,
            KeyCode::Char('G') | KeyCode::End => self.row = last,

            KeyCode::Enter | KeyCode::Char('e') | KeyCode::Char('i') => self.start_editing(),
            KeyCode::Char('c') => self.copy_from_base(),
            KeyCode::Char('x') => self.set_current(String::new()),

            KeyCode::Char('f') => self.from = self.next_locale(&self.from, &self.to),
            KeyCode::Char('t') => self.to = self.next_locale(&self.to, &self.from),
            KeyCode::Char('w') => std::mem::swap(&mut self.from, &mut self.to),
            KeyCode::Char('m') => self.only_missing = !self.only_missing,
            KeyCode::Char('v') => self.toggle_view(),

            KeyCode::Char('/') => self.focus = Focus::Search,
            KeyCode::Char('1') => self.set_filter(Filter::All),
            KeyCode::Char('2') => self.set_filter(Filter::NeedsAttention),
            KeyCode::Char('3') => self.set_filter(Filter::Missing),
            KeyCode::Char('4') => self.set_filter(Filter::Changed),
            KeyCode::Char('n') => self.cycle_namespace(),

            KeyCode::Char('s') => self.save(),
            KeyCode::Char('u') => self.request(ConfirmAction::Discard),
            KeyCode::Char('r') => self.request(ConfirmAction::Reload),
            KeyCode::Char('q') | KeyCode::Esc => self.request(ConfirmAction::Quit),
            _ => return,
        }

        // Switching languages or filters can shrink the list.
        let last = self.compare_rows().len().saturating_sub(1);
        self.row = self.row.min(last);
    }

    fn toggle_view(&mut self) {
        self.view = match self.view {
            View::Focus => View::Compare,
            View::Compare => View::Focus,
        };

        self.focus = Focus::List;
        self.row = 0;
    }

    fn focus_after_editing(&self) -> Focus {
        match self.view {
            View::Focus => Focus::Fields,
            View::Compare => Focus::List,
        }
    }

    /// The locale after `current` in project order, skipping `other`.
    fn next_locale(&self, current: &str, other: &str) -> String {
        let locales = self.locales();
        let start = locales.iter().position(|l| l == current).unwrap_or(0);

        (1..=locales.len())
            .map(|step| &locales[(start + step) % locales.len()])
            .find(|l| *l != other)
            .cloned()
            .unwrap_or_else(|| current.to_string())
    }

    fn on_fields_key(&mut self, key: KeyEvent) {
        let count = self.fields().len();

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.field = (self.field + 1).min(count.saturating_sub(1));
            }

            KeyCode::Char('k') | KeyCode::Up => self.field = self.field.saturating_sub(1),

            KeyCode::Enter | KeyCode::Char('e') | KeyCode::Char('i') => self.start_editing(),
            KeyCode::Char('c') => self.copy_from_base(),
            KeyCode::Char('x') => self.set_current(String::new()),

            // Open the n-th place the message is used.
            KeyCode::Char(digit @ '1'..='9') => {
                let n = digit as usize - '0' as usize;
                self.open_usage(n);
            }

            // Jump to the next or previous message without leaving the fields.
            KeyCode::Char('J') => self.move_selection(1),
            KeyCode::Char('K') => self.move_selection(-1),

            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left | KeyCode::BackTab => {
                self.focus = Focus::List;
            }

            KeyCode::Char('s') => self.save(),
            KeyCode::Char('q') => self.request(ConfirmAction::Quit),
            _ => {}
        }
    }

    fn on_editing_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        match key.code {
            KeyCode::Enter if alt => self.editor.insert('\n'),
            KeyCode::Enter => {
                self.set_current(self.editor.text().to_string());
                self.focus = self.focus_after_editing();
            }

            KeyCode::Esc => self.focus = self.focus_after_editing(),

            KeyCode::Char('u') if ctrl => self.editor.clear(),
            KeyCode::Char('a') if ctrl => self.editor.home(),
            KeyCode::Char('e') if ctrl => self.editor.end(),
            KeyCode::Char(c) => self.editor.insert(c),

            KeyCode::Backspace => self.editor.backspace(),
            KeyCode::Delete => self.editor.delete(),
            KeyCode::Left => self.editor.left(),
            KeyCode::Right => self.editor.right(),
            KeyCode::Home => self.editor.home(),
            KeyCode::End => self.editor.end(),
            _ => {}
        }
    }

    fn on_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Down => self.focus = Focus::List,
            KeyCode::Esc => {
                self.search.clear();
                self.focus = Focus::List;
            }

            KeyCode::Char(c) => self.search.insert(c),
            KeyCode::Backspace => self.search.backspace(),
            KeyCode::Left => self.search.left(),
            KeyCode::Right => self.search.right(),
            _ => return,
        }

        self.refresh_visible();
    }

    fn on_confirm_key(&mut self, key: KeyEvent, action: ConfirmAction) {
        self.focus = Focus::List;

        if !matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
            return;
        }

        self.run(action);
    }

    // -- Actions -------------------------------------------------------------

    /// Run an action, asking first if it would throw away unsaved edits.
    fn request(&mut self, action: ConfirmAction) {
        if self.session.has_edits() {
            self.focus = Focus::Confirm(action);
        } else {
            self.run(action);
        }
    }

    fn run(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Quit => self.quit = true,
            ConfirmAction::Discard => {
                self.session.discard();
                self.refresh_visible();
                self.message = Some("Edits discarded".into());
            }

            ConfirmAction::Reload => {
                self.session.discard();
                self.reload();
            }
        }
    }

    fn move_selection(&mut self, step: isize) {
        if self.visible.is_empty() {
            return;
        }

        let last = self.visible.len() - 1;
        self.selected = self.selected.saturating_add_signed(step).min(last);
        self.field = self.field.min(self.fields().len().saturating_sub(1));
    }

    fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
        self.refresh_visible();
    }

    fn cycle_namespace(&mut self) {
        let namespaces = self.namespaces();

        if namespaces.is_empty() {
            return;
        }

        let next = match &self.namespace {
            None => Some(0),
            Some(ns) => namespaces
                .iter()
                .position(|n| n == ns)
                .map(|i| i + 1)
                .filter(|i| *i < namespaces.len()),
        };

        self.namespace = next.map(|i| namespaces[i].clone());
        self.refresh_visible();
    }

    /// The first field that needs work, or the first non-source field.
    fn first_field_to_translate(&self) -> usize {
        let fields = self.fields();
        let base = &self.session.project.base_locale;

        let needs_work = fields
            .iter()
            .position(|key| !self.session.field_issues(key).is_empty());

        let first_target = fields.iter().position(|key| key.locale != *base);

        needs_work.or(first_target).unwrap_or(0)
    }

    fn start_editing(&mut self) {
        let Some(key) = self.current_field() else {
            return;
        };

        self.editor = TextInput::new(&self.session.value(&key));
        self.focus = Focus::Editing;
    }

    /// Copy the source text into the current field: the base locale in the
    /// focus view, the "from" locale in the compare view.
    fn copy_from_base(&mut self) {
        let Some(key) = self.current_field() else {
            return;
        };

        let source_locale = match self.view {
            View::Focus => self.session.project.base_locale.clone(),
            View::Compare => self.from.clone(),
        };

        let source = self
            .session
            .value(&FieldKey::new(&key.id, &key.variant, &source_locale));

        if key.locale != source_locale && !source.is_empty() {
            self.set_current(source);
        }
    }

    fn set_current(&mut self, text: String) {
        if let Some(key) = self.current_field() {
            self.session.set_value(key, text);
        }
    }

    fn save(&mut self) {
        let count = self.session.edit_count();

        if count == 0 {
            self.message = Some("Nothing to save".into());

            return;
        }

        self.message = Some(match self.session.save() {
            Ok(()) => format!("Saved {count} change(s)"),
            Err(e) => format!("Save failed: {e:#}"),
        });

        self.changed_on_disk = false;
        self.refresh_visible();
        self.rescan_usages();
    }

    fn reload(&mut self) {
        if let Err(e) = self.session.reload() {
            self.message = Some(format!("Reload failed: {e:#}"));
        }

        self.changed_on_disk = false;
        self.refresh_visible();
        self.rescan_usages();
    }

    fn rescan_usages(&mut self) {
        self.usages = self.session.scan_usages();
    }

    /// Usages of the selected message.
    pub fn current_usages(&self) -> &[Usage] {
        self.current()
            .and_then(|m| self.usages.usages.get(&m.id))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn open_usage(&mut self, n: usize) {
        match self.current_usages().get(n - 1) {
            Some(usage) => self.open_request = Some(usage.clone()),
            None => self.message = Some(format!("There is no usage {n}")),
        }
    }

    pub fn has_missing(&self, key: &FieldKey) -> bool {
        self.session.field_issues(key).into_iter().any(is_missing)
    }
}
