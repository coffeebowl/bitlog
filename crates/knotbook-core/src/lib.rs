//! Data model, file format and vault I/O for Knotbook.

mod config;
mod day;
mod error;
mod id;
mod markdown;
mod project;
mod vault;

pub use config::{DefaultsConfig, GridConfig, VaultConfig, WeekConfig};
pub use day::{Block, Day, DayWarning};
pub use error::ReadError;
pub use id::{BlockId, InvalidId, LocationKey, NotePath, ProjectSlug, TaskId};
pub use markdown::{MarkdownMode, MarkdownStyle, markdown_styles};
pub use project::{Project, ProjectStatus};
pub use vault::Vault;
