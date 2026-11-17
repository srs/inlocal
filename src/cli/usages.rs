//! `inlocal usages`, `inlocal unused` and `inlocal undefined`

use std::path::Path;
use std::process::ExitCode;

use anyhow::{bail, Result};

use crate::core::project::Catalog;
use crate::core::usages::{self, Usage, UsageReport};
use crate::core::Project;

/// Where one message is used, or a count per message without an id.
pub fn run(project: &Project, id: Option<&str>, json: bool) -> Result<ExitCode> {
    let catalog = project.read_messages()?;
    let report = usages::scan(project, &catalog);

    let Some(id) = id else {
        return summary(&report, &catalog, json);
    };

    if !catalog.messages.iter().any(|m| m.id == id) {
        bail!("no message '{id}'");
    }

    let found = report.usages.get(id).map(Vec::as_slice).unwrap_or_default();

    if json {
        println!("{}", serde_json::to_string_pretty(found)?);
        return Ok(ExitCode::SUCCESS);
    }

    for usage in found {
        print_usage(project, usage);
    }

    if found.is_empty() {
        eprintln!(
            "'{id}' is not used in {} source file(s)",
            report.files_scanned
        );
        return Ok(ExitCode::FAILURE);
    }

    Ok(ExitCode::SUCCESS)
}

/// Every message with how many places use it.
fn summary(report: &UsageReport, catalog: &Catalog, json: bool) -> Result<ExitCode> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(ExitCode::SUCCESS);
    }

    let width = catalog
        .messages
        .iter()
        .map(|m| m.id.len())
        .max()
        .unwrap_or(0);

    for message in &catalog.messages {
        let count = report.usages.get(&message.id).map_or(0, Vec::len);
        println!("{:<width$}  {count}", message.id);
    }

    println!("\n{} source file(s) scanned", report.files_scanned);

    Ok(ExitCode::SUCCESS)
}

/// Messages nothing in the code references. Exits 1 if there are any.
pub fn unused(project: &Project) -> Result<ExitCode> {
    let catalog = project.read_messages()?;
    let report = usages::scan(project, &catalog);
    let (unused, maybe) = report.unused(&catalog);

    for id in &unused {
        println!("{id}");
    }

    if !maybe.is_empty() {
        eprintln!("\nPossibly used through keys built at runtime:");

        for id in &maybe {
            eprintln!("  {id}");
        }
    }

    eprintln!(
        "\n{} unused, {} possibly used, {} source file(s) scanned",
        unused.len(),
        maybe.len(),
        report.files_scanned
    );

    let code = if unused.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    };

    Ok(code)
}

/// Keys used in code that the translation files don't have. Exits 1 if any.
pub fn undefined(project: &Project) -> Result<ExitCode> {
    let catalog = project.read_messages()?;
    let report = usages::scan(project, &catalog);

    for usage in &report.undefined {
        print_usage(project, usage);
    }

    eprintln!(
        "\n{} undefined key(s), {} source file(s) scanned",
        report.undefined.len(),
        report.files_scanned
    );

    let code = if report.undefined.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    };

    Ok(code)
}

/// `src/App.tsx:12:5  greeting  <h1>{m.greeting()}</h1>`, like a compiler,
/// so terminals and editors can jump to it.
fn print_usage(project: &Project, usage: &Usage) {
    println!(
        "{}:{}:{}  {}  {}",
        relative(&project.root, &usage.file),
        usage.line,
        usage.column,
        usage.id,
        usage.snippet
    );
}

fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .display()
        .to_string()
}
