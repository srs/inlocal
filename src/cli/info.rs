//! `inlocal info`

use std::process::ExitCode;

use crate::core::Project;
use anyhow::Result;

pub fn run(project: &Project) -> Result<ExitCode> {
    let catalog = project.read_messages()?;

    println!("project   {}", project.dir.display());
    println!("format    {}", project.format.label());
    println!("source    {}", project.base_locale);
    println!("locales   {}", project.locales.join(", "));
    println!("messages  {}", catalog.messages.len());
    println!();

    let mut missing_files = 0;

    for set in &project.files {
        match &set.namespace {
            Some(ns) => println!("{ns}  {}", set.path_pattern),
            None => println!("{}", set.path_pattern),
        }

        for locale in &project.locales {
            let path = project.file_path(set, locale);

            let shown = path
                .strip_prefix(&project.root)
                .unwrap_or(&path)
                .display()
                .to_string();

            if path.exists() {
                println!("  {locale:<6} {shown}");
            } else {
                println!("  {locale:<6} {shown}  (missing, created on first save)");
                missing_files += 1;
            }
        }
    }

    if missing_files > 0 {
        println!("\n{missing_files} file(s) don't exist yet.");
    }

    Ok(ExitCode::SUCCESS)
}
