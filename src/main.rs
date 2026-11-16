#[cfg(feature = "web")]
mod browser;
mod cli;
#[cfg(feature = "web")]
mod server;
mod tui;

use std::process::ExitCode;

fn main() -> ExitCode {
    match cli::run() {
        Ok(code) => code,

        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}
