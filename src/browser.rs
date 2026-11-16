//! Opening the web UI without piling up browser tabs.
//!
//! Browsers don't let other programs pick a tab, but on macOS the common
//! browsers can be scripted with AppleScript. We use that to bring an
//! existing inlocal tab to the front, and fall back to opening a new tab.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

/// Show `url` in the browser, reusing a tab that already shows it.
pub fn show(url: &str) {
    if focus_existing_tab(url) {
        return;
    }

    let _ = open::that(url);
}

/// Look for an inlocal server already serving `project_dir` on one of the
/// ports we would use. Returns its URL.
pub fn find_running_server(project_dir: &Path, ports: std::ops::Range<u16>) -> Option<String> {
    let wanted = project_dir.to_string_lossy();

    for port in ports {
        let Some(body) = get(port, "/api/project") else {
            continue;
        };

        let Ok(info) = serde_json::from_str::<serde_json::Value>(&body) else {
            continue;
        };

        if info["projectDir"].as_str() == Some(wanted.as_ref()) {
            return Some(format!("http://localhost:{port}"));
        }
    }

    None
}

/// A tiny HTTP GET against localhost, enough to talk to another inlocal.
fn get(port: u16, path: &str) -> Option<String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let timeout = Duration::from_millis(200);

    let mut stream = TcpStream::connect_timeout(&addr, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;

    let request = format!("GET {path} HTTP/1.0\r\nHost: localhost:{port}\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;

    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;

    // The status line is "HTTP/1.0 200 OK" or "HTTP/1.1 200 OK".
    let (head, body) = response.split_once("\r\n\r\n")?;
    let status = head.lines().next()?;

    status.contains(" 200 ").then(|| body.to_string())
}

// -- Finding an existing tab (macOS) -----------------------------------------

#[cfg(target_os = "macos")]
fn focus_existing_tab(url: &str) -> bool {
    use std::process::Command;

    // Browsers that share Chrome's AppleScript dictionary.
    const CHROMIUM: [&str; 6] = [
        "Google Chrome",
        "Microsoft Edge",
        "Brave Browser",
        "Arc",
        "Vivaldi",
        "Chromium",
    ];

    let mut scripts: Vec<String> = CHROMIUM
        .iter()
        .map(|app| chromium_script(app, url))
        .collect();

    scripts.push(safari_script(url));

    for script in scripts {
        let output = Command::new("osascript").arg("-e").arg(&script).output();

        let found = output
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "found")
            .unwrap_or(false);

        if found {
            return true;
        }
    }

    false
}

#[cfg(not(target_os = "macos"))]
fn focus_existing_tab(_url: &str) -> bool {
    false
}

/// Only talks to the browser if it is already running, so we never launch
/// one just to look for a tab.
#[cfg(target_os = "macos")]
fn chromium_script(app: &str, url: &str) -> String {
    format!(
        r#"
if application "{app}" is running then
    tell application "{app}"
        repeat with w in windows
            set i to 0
            repeat with t in tabs of w
                set i to i + 1
                if URL of t starts with "{url}" then
                    set active tab index of w to i
                    set index of w to 1
                    activate
                    return "found"
                end if
            end repeat
        end repeat
    end tell
end if
return "none"
"#
    )
}

#[cfg(target_os = "macos")]
fn safari_script(url: &str) -> String {
    format!(
        r#"
if application "Safari" is running then
    tell application "Safari"
        repeat with w in windows
            repeat with t in tabs of w
                if URL of t starts with "{url}" then
                    set current tab of w to t
                    set index of w to 1
                    activate
                    return "found"
                end if
            end repeat
        end repeat
    end tell
end if
return "none"
"#
    )
}
