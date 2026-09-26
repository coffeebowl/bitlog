//! Data model, file format and vault I/O for Knotbook.

mod check;
mod config;
mod day;
mod error;
mod file;
mod id;
mod init;
mod markdown;
mod project;
mod vault;
mod watch;

pub use check::Problem;
pub use config::{DefaultsConfig, GridConfig, VaultConfig, WeekConfig};
pub use day::{Block, Day, DayWarning, RemovedText};
pub use error::{EditError, ReadError, SaveError};
pub use id::{BlockId, InvalidId, LocationKey, NotePath, ProjectSlug, TaskId};
pub use init::CreateError;
pub use markdown::{MarkdownMode, MarkdownStyle, markdown_styles};
pub use project::{Project, ProjectStatus};
pub use vault::{DayFile, Vault};
pub use watch::{VaultChange, VaultWatcher, WatchError};
