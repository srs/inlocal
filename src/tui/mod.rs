//! `inlocal tui`: edit translations in the terminal.

mod app;
mod input;
mod ui;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use inlocal_core::{Project, Session};
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
