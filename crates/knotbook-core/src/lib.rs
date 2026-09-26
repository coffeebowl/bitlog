//! Data model, file format and vault I/O for Knotbook.

mod config;
mod id;

pub use config::{ConfigError, DefaultsConfig, GridConfig, VaultConfig, WeekConfig};
pub use id::{BlockId, InvalidId, LocationKey, NotePath, ProjectSlug, TaskId};
