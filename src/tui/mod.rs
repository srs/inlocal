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

        // Wake up regularly to pick up file changes and scan results.
        app.tick();

        if !event::poll(Duration::from_millis(250))? {
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

#[cfg(test)]
mod bench {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{backend::TestBackend, Terminal};
    use std::time::Instant;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// `INLOCAL_BENCH=/path/to/project cargo test --release bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn large_project() {
        let Ok(path) = std::env::var("INLOCAL_BENCH") else {
            return;
        };

        let start = Instant::now();
        let dir = crate::core::find_project(Path::new(&path)).unwrap();
        let project = Arc::new(Project::load(&dir).unwrap());
        let session = Session::load(project).unwrap();
        let mut app = App::new(session, Arc::new(AtomicBool::new(false)));
        println!("load: {:?}", start.elapsed());

        let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();
        let mut frame = |app: &App| {
            terminal.draw(|f| ui::draw(f, app)).unwrap();
        };

        let time = |label: &str, app: &mut App, frame: &mut dyn FnMut(&App), keys: &[KeyEvent]| {
            let start = Instant::now();

            for k in keys {
                app.on_key(*k);
                frame(app);
            }

            let each = start.elapsed() / keys.len() as u32;
            println!("{label}: {each:?} per key");
        };

        time(
            "move in list",
            &mut app,
            &mut frame,
            &[key(KeyCode::Char('j')); 200],
        );

        time("type in a field", &mut app, &mut frame, &{
            let mut keys = vec![key(KeyCode::Enter), key(KeyCode::Enter)];
            keys.extend("Hello world".chars().map(|c| key(KeyCode::Char(c))));
            keys.push(key(KeyCode::Enter));
            keys
        });

        time("search", &mut app, &mut frame, &{
            let mut keys = vec![key(KeyCode::Esc), key(KeyCode::Char('/'))];
            keys.extend("text 12".chars().map(|c| key(KeyCode::Char(c))));
            keys.push(key(KeyCode::Esc));
            keys
        });

        time("compare view", &mut app, &mut frame, &{
            let mut keys = vec![key(KeyCode::Char('v'))];
            keys.extend([key(KeyCode::Char('j')); 200]);
            keys.push(key(KeyCode::Char('m')));
            keys
        });
    }
}
