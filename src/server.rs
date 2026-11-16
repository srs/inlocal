//! Local HTTP server: JSON API, file-change events and the embedded UI.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use notify::{RecursiveMode, Watcher};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use inlocal_core::checks::{self, Issue, LocaleStats};

use crate::browser;
use inlocal_core::{Edit, Message, Project};

#[derive(RustEmbed)]
#[folder = "ui/dist"]
struct Assets;

#[derive(Clone)]
struct AppState {
    project: Arc<Project>,
    changes: broadcast::Sender<()>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectInfo<'a> {
    name: String,
    project_dir: String,
    base_locale: &'a str,
    locales: &'a [String],
    format: inlocal_core::Format,
    files: &'a [inlocal_core::FileSet],
    git_branch: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MessagesResponse {
    messages: Vec<Message>,
    issues: Vec<Issue>,
    stats: std::collections::BTreeMap<String, LocaleStats>,
}

#[derive(Deserialize)]
struct SaveRequest {
    edits: Vec<Edit>,
}

struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({ "error": format!("{:#}", self.0) }));
        (StatusCode::UNPROCESSABLE_ENTITY, body).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(e.into())
    }
}

/// Start the web editor and block until Ctrl+C.
pub fn run(project: Project, port: u16, open_browser: bool) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(serve(project, port, open_browser))?;

    // Don't wait for lingering tasks (file watcher, open streams).
    runtime.shutdown_timeout(Duration::from_millis(200));

    Ok(())
}

async fn serve(project: Project, port: u16, open_browser: bool) -> Result<()> {
    let project = Arc::new(project);
    let (changes, _) = broadcast::channel(16);
    let _watcher = watch(&project, changes.clone())?;

    let state = AppState {
        project: project.clone(),
        changes,
    };
    let app = Router::new()
        .route("/api/project", get(project_info))
        .route("/api/messages", get(messages))
        .route("/api/save", post(save))
        .route("/api/events", get(events))
        .fallback(static_file)
        .layer(middleware::from_fn(guard))
        .with_state(state);

    // Already serving this project? Show that one instead of a second server.
    let ports = port..port.saturating_add(20);

    if let Some(url) = browser::find_running_server(&project.dir, ports) {
        println!("inlocal is already running for this project at {url}");

        if open_browser {
            browser::show(&url);
        }

        return Ok(());
    }

    let listener = bind(port).await?;
    let addr = listener.local_addr()?;
    let url = format!("http://localhost:{}", addr.port());
    println!("inlocal  {}", project.dir.display());
    println!("         {url}");
    println!("         press Ctrl+C to stop");
    if open_browser {
        browser::show(&url);
    }
    // Don't use graceful shutdown: the browser keeps the /api/events stream
    // open forever, so waiting for connections to drain would hang Ctrl+C.
    // Saves write files synchronously within a request, so stopping here
    // can't leave a half-written file (writes go through a temp file + rename).
    tokio::select! {
        res = axum::serve(listener, app) => res?,
        _ = tokio::signal::ctrl_c() => println!("\nstopped"),
    }
    Ok(())
}

async fn bind(port: u16) -> Result<tokio::net::TcpListener> {
    let mut last_err = None;
    for p in port..port.saturating_add(20) {
        match tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], p))).await {
            Ok(l) => return Ok(l),
            Err(e) => last_err = Some(e),
        }
    }
    Err(anyhow::anyhow!(
        "no free port from {port}: {}",
        last_err.map(|e| e.to_string()).unwrap_or_default()
    ))
}

/// The server only listens on 127.0.0.1, but a web page in the browser can
/// still send requests to it. Reject foreign Host headers (DNS rebinding) and
/// require a custom header on writes, which browsers won't send cross-origin
/// without a CORS preflight that this server never approves.
async fn guard(headers: HeaderMap, req: Request, next: Next) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let hostname = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    if !matches!(hostname, "localhost" | "127.0.0.1" | "[::1]") {
        return (StatusCode::FORBIDDEN, "forbidden host").into_response();
    }
    if req.method() != Method::GET && headers.get("x-inlocal").is_none() {
        return (StatusCode::FORBIDDEN, "missing x-inlocal header").into_response();
    }
    next.run(req).await
}

async fn project_info(State(s): State<AppState>) -> Json<serde_json::Value> {
    let p = &s.project;
    let name = p
        .root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let info = ProjectInfo {
        name,
        project_dir: p.dir.display().to_string(),
        base_locale: &p.base_locale,
        locales: &p.locales,
        format: p.format,
        files: &p.files,
        git_branch: git_branch(&p.root),
    };
    Json(serde_json::to_value(info).expect("serializable"))
}

async fn messages(State(s): State<AppState>) -> Result<Json<MessagesResponse>, ApiError> {
    Ok(Json(load(&s.project)?))
}

async fn save(
    State(s): State<AppState>,
    Json(req): Json<SaveRequest>,
) -> Result<Json<MessagesResponse>, ApiError> {
    s.project.apply_edits(&req.edits)?;
    Ok(Json(load(&s.project)?))
}

fn load(project: &Project) -> anyhow::Result<MessagesResponse> {
    let catalog = project.read_messages()?;
    let report = checks::run(project, &catalog);
    Ok(MessagesResponse {
        messages: catalog.messages,
        issues: report.issues,
        stats: report.stats,
    })
}

async fn events(
    State(s): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(s.changes.subscribe())
        .map(|_| Ok(Event::default().event("changed").data("files")));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn static_file(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let (path, file) = match Assets::get(path) {
        Some(f) => (path, f),
        None => match Assets::get("index.html") {
            Some(f) => ("index.html", f),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    "UI not built. Run `bun run build` in ui/.",
                )
                    .into_response()
            }
        },
    };
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    (
        [(header::CONTENT_TYPE, mime.as_ref().to_string())],
        file.data.into_owned(),
    )
        .into_response()
}

/// Watch the translation directories and broadcast a debounced "changed".
fn watch(project: &Project, changes: broadcast::Sender<()>) -> Result<notify::RecommendedWatcher> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let files: Vec<PathBuf> = project.all_files();
    let watched = files.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            if event.paths.iter().any(|p| watched.iter().any(|f| f == p)) {
                let _ = tx.send(());
            }
        }
    })?;

    let mut dirs: Vec<PathBuf> = files
        .iter()
        .filter_map(|f| f.parent().map(|p| p.to_path_buf()))
        .collect();
    dirs.sort();
    dirs.dedup();
    for dir in dirs {
        if dir.exists() {
            watcher.watch(&dir, RecursiveMode::NonRecursive)?;
        }
    }

    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            tokio::time::sleep(Duration::from_millis(150)).await;
            while rx.try_recv().is_ok() {}
            let _ = changes.send(());
        }
    });
    Ok(watcher)
}

fn git_branch(root: &std::path::Path) -> Option<String> {
    for dir in root.ancestors() {
        let head = dir.join(".git/HEAD");
        if let Ok(text) = std::fs::read_to_string(head) {
            return Some(
                text.trim()
                    .strip_prefix("ref: refs/heads/")
                    .unwrap_or(text.trim())
                    .to_string(),
            );
        }
    }
    None
}
