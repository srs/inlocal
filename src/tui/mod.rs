//! `inlocal tui`: edit translations in the terminal.

mod app;
mod input;
mod ui;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::core::usages::Usage;
use crate::core::{Project, Session};
use anyhow::{Context, Result};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use app::App;

pub fn run(project: Project) -> Result<()> {
    let project = Arc::new(project);
    let session = Session::load(project.clone())?;

    let files_changed = Arc::new(AtomicBool::new(false));
    let _watcher = watch_files(project.all_files(), files_changed.clone());

    let mut app = App::new(session, files_changed);

    // `ratatui::init` also restores the terminal if we panic.
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();

    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.quit {
        terminal.draw(|frame| ui::draw(frame, app))?;

        // Wake up regularly to pick up file changes, even without input.
        if !event::poll(Duration::from_millis(250))? {
            app.tick();
            continue;
        }

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                app.message = None;
                app.on_key(key);
            }
        }

        if let Some(usage) = app.open_request.take() {
            if let Err(e) = open_in_editor(terminal, &usage) {
                app.message = Some(format!("Couldn't open the editor: {e:#}"));
            }
        }
    }

    Ok(())
}

/// Open a usage in `$VISUAL` / `$EDITOR`, at its line.
///
/// Terminal editors take over the screen, so the TUI steps aside while the
/// editor runs and comes back afterwards.
fn open_in_editor(terminal: &mut ratatui::DefaultTerminal, usage: &Usage) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    // `EDITOR` may hold arguments too, like `code --wait`.
    let mut words = editor.split_whitespace();
    let program = words.next().unwrap_or("vi");
    let base_args: Vec<&str> = words.collect();

    let name = Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let file = usage.file.display().to_string();
    let location = format!("{file}:{}:{}", usage.line, usage.column);

    // How each editor wants to be told the line.
    let args: Vec<String> = match name.as_str() {
        "code" | "code-insiders" | "codium" | "cursor" | "windsurf" => {
            vec!["-g".into(), location]
        }
        "zed" | "subl" | "hx" | "helix" => vec![location],
        _ => vec![format!("+{}", usage.line), file],
    };

    ratatui::restore();

    let status = Command::new(program).args(base_args).args(&args).status();

    *terminal = ratatui::init();
    terminal.clear()?;

    let status = status.with_context(|| format!("couldn't run '{program}'"))?;
    if !status.success() {
        anyhow::bail!("'{program}' exited with {status}");
    }

    Ok(())
}

/// Watch the folders holding translation files and raise `flag` on changes.
fn watch_files(files: Vec<PathBuf>, flag: Arc<AtomicBool>) -> Option<RecommendedWatcher> {
    let watched = files.clone();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };

        if event.paths.iter().any(|p| watched.contains(p)) {
            flag.store(true, Ordering::Relaxed);
        }
    })
    .ok()?;

    let mut dirs: Vec<PathBuf> = files
        .iter()
        .filter_map(|f| f.parent().map(PathBuf::from))
        .collect();

    dirs.sort();
    dirs.dedup();

    for dir in dirs.iter().filter(|d| d.exists()) {
        let _ = watcher.watch(dir, RecursiveMode::NonRecursive);
    }

    Some(watcher)
}
