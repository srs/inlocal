//! Shared core of inlocal: project loading, format-preserving JSON edits and
//! translation checks, and the editing session the UIs are built on.

pub mod checks;
pub mod jsonedit;
pub mod project;
pub mod session;

pub use checks::{display_variant, variable_names, Issue, IssueKind, LocaleStats, Report};
pub use project::{find_project, Catalog, Edit, FileSet, Format, Location, Message, Project};
pub use session::{FieldKey, Filter, Session, Status};
