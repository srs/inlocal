//! The core of inlocal: project loading, format-preserving JSON edits,
//! translation checks, and the editing session the TUI is built on.

pub mod checks;
pub mod jsonedit;
pub mod project;
pub mod session;
pub mod usages;

pub use checks::display_variant;
pub use project::{find_project, Message, Project};
pub use session::{FieldKey, Filter, Session, Status};
