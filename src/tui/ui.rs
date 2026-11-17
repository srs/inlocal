//! Drawing the TUI. Pure rendering: reads `App`, never changes it.

use crate::core::{display_variant, FieldKey, Filter, Status};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, List, ListItem, ListState, Padding, Paragraph, Row, Table, TableState,
};

use ratatui::Frame;

use super::app::{App, ConfirmAction, Focus, View};
use super::input::TextInput;

const ACCENT: Color = Color::Green;
const WARN: Color = Color::Yellow;
const DANGER: Color = Color::Red;
const CHANGED: Color = Color::Blue;
const DIM: Color = Color::DarkGray;

pub fn draw(frame: &mut Frame, app: &App) {
    let [top, body, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let [sidebar, main] =
        Layout::horizontal([Constraint::Length(26), Constraint::Min(40)]).areas(body);

    draw_top_bar(frame, app, top);
    draw_sidebar(frame, app, sidebar);

    match app.view {
        View::Focus => {
            let [list, editor] =
                Layout::horizontal([Constraint::Percentage(45), Constraint::Min(30)]).areas(main);

            draw_list(frame, app, list);
            draw_editor(frame, app, editor);
        }

        View::Compare => draw_compare(frame, app, main),
    }

    draw_bottom_bar(frame, app, bottom);
}

// -- Top bar ------------------------------------------------------------------

fn draw_top_bar(frame: &mut Frame, app: &App, area: Rect) {
    let name = app
        .session
        .project
        .root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut spans = vec![
        Span::styled(" inlocal ", Style::new().fg(Color::Black).bg(ACCENT).bold()),
        Span::raw(format!("  {name}    ")),
    ];

    // Search box.
    let searching = app.focus == Focus::Search;
    let query = app.search.text();

    if searching || !query.is_empty() {
        spans.push(Span::styled("search: ", Style::new().fg(DIM)));

        if searching {
            spans.extend(with_cursor(&app.search));
        } else {
            spans.push(Span::styled(query.to_string(), Style::new().bold()));
        }
    } else {
        spans.push(Span::styled("press / to search", Style::new().fg(DIM)));
    }

    let edits = app.session.edit_count();

    let right = if edits > 0 {
        Line::from(Span::styled(
            format!("{edits} unsaved  "),
            Style::new().fg(CHANGED),
        ))
    } else {
        Line::from("")
    };

    frame.render_widget(Line::from(spans), area);
    frame.render_widget(right.right_aligned(), area);
}

// -- Sidebar ------------------------------------------------------------------

fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(section("Filters"));

    for (ix, filter) in Filter::ALL.into_iter().enumerate() {
        let active = app.filter == filter;
        let count = app.session.count(filter);

        lines.push(sidebar_row(
            format!("{} {}", ix + 1, filter.label()),
            count.to_string(),
            active,
        ));
    }

    lines.push(Line::raw(""));
    lines.push(section("Locales"));

    for locale in app.locales() {
        let percent = match app.session.stats(&locale) {
            Some(s) if s.total > 0 => s.translated * 100 / s.total,
            _ => 100,
        };

        let color = match percent {
            100 => ACCENT,
            80.. => WARN,
            _ => DANGER,
        };

        lines.push(Line::from(vec![
            Span::styled("● ", Style::new().fg(color)),
            Span::raw(format!("{locale:<8}")),
            Span::styled(format!("{percent:>4}%"), Style::new().fg(DIM)),
        ]));
    }

    let namespaces = app.namespaces();

    if !namespaces.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section("Namespaces  (n)"));

        let all = app.namespace.is_none();
        lines.push(sidebar_row("all".into(), String::new(), all));

        for ns in namespaces {
            let active = app.namespace.as_ref() == Some(&ns);
            lines.push(sidebar_row(ns, String::new(), active));
        }
    }

    let block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::new().fg(DIM))
        .padding(Padding::horizontal(1));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn section(title: &str) -> Line<'static> {
    Line::styled(title.to_string(), Style::new().fg(DIM))
}

fn sidebar_row(label: String, count: String, active: bool) -> Line<'static> {
    let style = if active {
        Style::new().fg(ACCENT).bold()
    } else {
        Style::new()
    };

    Line::from(vec![
        Span::styled(format!("{label:<18}"), style),
        Span::styled(format!("{count:>4}"), Style::new().fg(DIM)),
    ])
}

// -- Message list -------------------------------------------------------------

fn draw_list(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::new().fg(DIM));

    if app.visible.is_empty() {
        let empty = Paragraph::new(Line::styled("  No messages match.", Style::new().fg(DIM)));
        frame.render_widget(empty.block(block), area);

        return;
    }

    let base = &app.session.project.base_locale;

    // Only build the rows that fit; each message takes two lines.
    let fits = (area.height as usize / 2).max(1);
    let offset = scroll_offset(&app.list_offset, app.selected, fits);
    let shown = &app.visible[offset..app.visible.len().min(offset + fits)];

    let items: Vec<ListItem> = shown
        .iter()
        .map(|id| {
            let status = app.session.status(id);

            let preview = app
                .session
                .message(id)
                .and_then(|m| m.variants.last())
                .map(|variant| app.session.value(&FieldKey::new(id, variant, base)))
                .unwrap_or_default();

            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(" ● ", Style::new().fg(status_color(status))),
                    Span::raw(id.clone()),
                ]),
                Line::styled(format!("   {}", one_line(&preview)), Style::new().fg(DIM)),
            ])
        })
        .collect();

    let highlight = if app.focus == Focus::List {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };

    let list = List::new(items).block(block).highlight_style(highlight);

    let mut state = ListState::default().with_selected(Some(app.selected - offset));
    frame.render_stateful_widget(list, area, &mut state);
}

/// The first row to draw so `selected` is on screen, moving the remembered
/// offset as little as possible (so the list doesn't jump around).
fn scroll_offset(remembered: &std::cell::Cell<usize>, selected: usize, fits: usize) -> usize {
    let mut offset = remembered.get();

    if selected < offset {
        offset = selected;
    }

    if selected >= offset + fits {
        offset = selected + 1 - fits;
    }

    remembered.set(offset);
    offset
}

fn status_color(status: Status) -> Color {
    match status {
        Status::Ok => ACCENT,
        Status::Warning => WARN,
        Status::Missing => DANGER,
        Status::Changed => CHANGED,
    }
}

// -- Editor -------------------------------------------------------------------

fn draw_editor(frame: &mut Frame, app: &App, area: Rect) {
    let area = area.inner(ratatui::layout::Margin::new(2, 0));

    let Some(message) = app.current() else {
        return;
    };

    let base = app.session.project.base_locale.clone();
    let fields = app.fields();
    let plural = message.is_plural();

    let mut lines: Vec<Line> = vec![
        Line::styled(message.id.clone(), Style::new().bold()),
        Line::raw(""),
    ];

    // Where the selected field is drawn, for scrolling and the cursor.
    let mut selected_line = 0;

    for (ix, key) in fields.iter().enumerate() {
        let first_of_locale = ix == 0 || fields[ix - 1].locale != key.locale;
        let selected = ix == app.field && app.focus != Focus::List;
        let editing = selected && app.focus == Focus::Editing;

        if first_of_locale && ix > 0 {
            lines.push(Line::raw(""));
        }

        // Left gutter: the locale on its first row, the variant on every row.
        let locale = if first_of_locale {
            key.locale.as_str()
        } else {
            ""
        };

        let locale_style = if key.locale == base {
            Style::new().fg(ACCENT)
        } else {
            Style::new().fg(Color::Gray)
        };

        let marker = if selected { "▌" } else { " " };

        let mut spans = vec![
            Span::styled(marker, Style::new().fg(ACCENT)),
            Span::styled(format!("{locale:<6}"), locale_style),
        ];

        if plural {
            let variant = display_variant(&key.variant);
            spans.push(Span::styled(format!("{variant:<7}"), Style::new().fg(DIM)));
        }

        let gutter_width = Line::from(spans.clone()).width() as u16;

        // The text, the text being edited, or a placeholder.
        let text_style = field_style(app, key, selected);

        if editing {
            spans.extend(with_cursor(&app.editor));
        } else {
            let value = app.session.value(key);

            if value.is_empty() {
                let source = app.session.source_text(key, &base);

                let placeholder = if source.is_empty() {
                    "—".to_string()
                } else {
                    one_line(&source)
                };

                spans.push(Span::styled(placeholder, Style::new().fg(DIM).italic()));
            } else {
                spans.push(Span::styled(one_line(&value), text_style));
            }
        }

        if selected {
            selected_line = lines.len();
        }

        lines.push(Line::from(spans));

        // Issues for this field.
        for issue in app.session.field_issues(key) {
            let mut issue_spans = vec![
                Span::raw(" ".repeat(gutter_width as usize)),
                Span::styled(format!("⚠ {}", issue.message), Style::new().fg(WARN)),
            ];

            if selected && app.has_missing(key) {
                issue_spans.push(Span::styled(
                    format!("   c copy from {base}"),
                    Style::new().fg(DIM),
                ));
            }

            lines.push(Line::from(issue_spans));
        }
    }

    lines.push(Line::raw(""));
    lines.extend(usage_lines(app));

    // Keep the selected field on screen.
    let height = area.height as usize;
    let scroll = selected_line.saturating_sub(height.saturating_sub(4));

    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), area);
}

fn field_style(app: &App, key: &FieldKey, selected: bool) -> Style {
    let mut style = if app.session.is_changed(key) {
        Style::new().fg(CHANGED)
    } else {
        Style::new()
    };

    if selected {
        style = style.bold();
    }

    style
}

/// The text being edited, underlined, with a solid block on the character
/// under the cursor. Easier to see than the terminal's own thin cursor.
fn with_cursor(input: &TextInput) -> Vec<Span<'static>> {
    let (before, after) = input.split_at_cursor();

    let mut rest = after.chars();

    let under = match rest.next() {
        Some('\n') => "⏎".to_string(),
        Some(c) => c.to_string(),
        // At the end of the text: a block after the last character.
        None => " ".to_string(),
    };

    let text = Style::new().add_modifier(Modifier::UNDERLINED | Modifier::BOLD);
    let cursor = Style::new().fg(Color::Black).bg(ACCENT);

    vec![
        Span::styled(one_line(before), text),
        Span::styled(under, cursor),
        Span::styled(one_line(rest.as_str()), text),
    ]
}

/// Show newlines as ⏎ so every value fits on one row.
fn one_line(text: &str) -> String {
    text.replace('\n', "⏎")
}

/// "Used in 2 places" and a numbered list; the number opens it.
fn usage_lines(app: &App) -> Vec<Line<'static>> {
    let usages = app.current_usages();
    let root = &app.session.project.root;

    if usages.is_empty() {
        let dynamic = app
            .current()
            .and_then(|m| app.usages.dynamic_prefix_for(&m.id));

        let text = match dynamic {
            Some(prefix) => format!("Possibly used through a runtime key ({prefix}…)"),
            None => "Not used in the source code".to_string(),
        };

        return vec![Line::styled(text, Style::new().fg(DIM))];
    }

    let title = match usages.len() {
        1 => "Used in 1 place".to_string(),
        n => format!("Used in {n} places"),
    };

    let mut lines = vec![Line::styled(title, Style::new().fg(DIM))];

    for (ix, usage) in usages.iter().take(9).enumerate() {
        let file = usage.file.strip_prefix(root).unwrap_or(&usage.file);

        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", ix + 1), Style::new().fg(ACCENT).bold()),
            Span::raw(format!("{}:{}  ", file.display(), usage.line)),
            Span::styled(usage.snippet.clone(), Style::new().fg(DIM)),
        ]));
    }

    if usages.len() > 9 {
        lines.push(Line::styled(
            format!(
                "   … and {} more (inlocal usages {})",
                usages.len() - 9,
                usages[0].id
            ),
            Style::new().fg(DIM),
        ));
    }

    lines
}

// -- Compare view -------------------------------------------------------------

fn draw_compare(frame: &mut Frame, app: &App, area: Rect) {
    let [bar, table_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(area);

    draw_compare_bar(frame, app, bar);

    let rows = app.compare_rows();

    if rows.is_empty() {
        let text = if app.only_missing {
            format!("  Nothing missing in {}.", app.to)
        } else {
            "  No messages match.".to_string()
        };

        frame.render_widget(Line::styled(text, Style::new().fg(DIM)), table_area);

        return;
    }

    // Only build the rows that fit, below the header line.
    let fits = (table_area.height as usize).saturating_sub(1).max(1);
    let offset = scroll_offset(&app.compare_offset, app.row, fits);
    let shown = &rows[offset..rows.len().min(offset + fits)];

    let editing = (app.focus == Focus::Editing).then_some(app.row - offset);

    let table_rows: Vec<Row> = shown
        .iter()
        .enumerate()
        .map(|(ix, key)| compare_row(app, key, editing == Some(ix)))
        .collect();

    let header = Row::new([
        Cell::from("Key"),
        Cell::from(app.from.clone()),
        Cell::from(app.to.clone()),
    ])
    .style(Style::new().fg(DIM));

    let widths = [
        Constraint::Percentage(26),
        Constraint::Percentage(37),
        Constraint::Percentage(37),
    ];

    // While editing, the row keeps its normal colors so the cursor stands out.
    let highlight = if app.focus == Focus::Editing {
        Style::new()
    } else {
        Style::new().add_modifier(Modifier::REVERSED)
    };

    let table = Table::new(table_rows, widths)
        .header(header)
        .column_spacing(2)
        .row_highlight_style(highlight);

    let mut state = TableState::default().with_selected(Some(app.row - offset));
    frame.render_stateful_widget(table, table_area, &mut state);
}

fn draw_compare_bar(frame: &mut Frame, app: &App, area: Rect) {
    let key = |k: &str| Span::styled(k.to_string(), Style::new().fg(ACCENT).bold());
    let dim = |t: &str| Span::styled(t.to_string(), Style::new().fg(DIM));

    let missing = if app.only_missing { "on" } else { "off" };

    let line = Line::from(vec![
        dim(" from "),
        Span::styled(app.from.clone(), Style::new().bold()),
        dim(" ("),
        key("f"),
        dim(")   to "),
        Span::styled(app.to.clone(), Style::new().bold()),
        dim(" ("),
        key("t"),
        dim(")   "),
        key("w"),
        dim(" swap   "),
        key("m"),
        dim(&format!(" only missing: {missing}")),
    ]);

    frame.render_widget(line, area);
}

fn compare_row<'a>(app: &App, target: &FieldKey, editing: bool) -> Row<'a> {
    let source = app.session.source_text(target, &app.from);

    let key_text = if target.variant.is_empty() {
        target.id.clone()
    } else {
        format!("{} [{}]", target.id, display_variant(&target.variant))
    };

    let target_cell = if editing {
        Cell::from(Line::from(with_cursor(&app.editor)))
    } else {
        let value = app.session.value(target);
        let issues = app.session.field_issues(target);

        if value.is_empty() {
            Cell::from("— missing").style(Style::new().fg(DANGER))
        } else if !issues.is_empty() {
            Cell::from(format!("⚠ {}", one_line(&value))).style(Style::new().fg(WARN))
        } else {
            Cell::from(one_line(&value)).style(field_style(app, target, false))
        }
    };

    Row::new([
        Cell::from(key_text).style(Style::new().fg(Color::Gray)),
        Cell::from(one_line(&source)),
        target_cell,
    ])
}

// -- Bottom bar ---------------------------------------------------------------

fn draw_bottom_bar(frame: &mut Frame, app: &App, area: Rect) {
    if let Focus::Confirm(action) = app.focus {
        let question = match action {
            ConfirmAction::Quit => "Quit and lose unsaved edits?",
            ConfirmAction::Discard => "Discard all unsaved edits?",
            ConfirmAction::Reload => "Reload from disk and lose unsaved edits?",
        };

        let line = Line::from(vec![
            Span::styled(
                format!(" {question} "),
                Style::new().fg(Color::Black).bg(WARN),
            ),
            Span::raw("  y yes   any other key: no"),
        ]);

        frame.render_widget(line, area);

        return;
    }

    if app.changed_on_disk {
        let line = Line::styled(
            " Files changed on disk while you have unsaved edits. r reloads (drops edits), s saves yours.",
            Style::new().fg(WARN),
        );

        frame.render_widget(line, area);

        return;
    }

    let hints = match (app.focus, app.view) {
        (Focus::List, View::Focus) => {
            "j/k move  enter edit  / search  1-4 filter  n namespace  v compare  s save  q quit"
        }

        (Focus::List, View::Compare) => {
            "j/k move  enter edit  c copy  f/t languages  m only missing  v focus view  s save  q quit"
        }

        (Focus::Fields, _) => {
            "j/k field  J/K message  enter edit  c copy source  1-9 open usage  esc back  s save"
        }

        (Focus::Editing, _) => "enter keep  esc cancel  alt+enter newline  ctrl+u clear",
        (Focus::Search, _) => "type to filter  enter done  esc clear",
        (Focus::Confirm(_), _) => "",
    };

    let mut spans = vec![Span::styled(format!(" {hints}"), Style::new().fg(DIM))];

    if let Some(message) = &app.message {
        spans = vec![
            Span::styled(format!(" {message}   "), Style::new().fg(ACCENT)),
            spans.remove(0),
        ];
    }

    frame.render_widget(Line::from(spans), area);
}
