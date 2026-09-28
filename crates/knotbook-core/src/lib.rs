//! Data model, file format and vault I/O for Knotbook.

mod check;
mod config;
mod conflict;
mod day;
mod device;
mod error;
mod file;
mod git_log;
mod id;
mod init;
mod markdown;
mod notes;
mod period;
mod project;
mod standup;
mod tasks;
mod toml_values;
mod vault;
mod watch;

pub use check::Problem;
pub use config::{DefaultsConfig, GridConfig, VaultConfig, WeekConfig};
pub use conflict::{ConflictCopy, ConflictVersions, Contradiction};
pub use day::{Block, Day, DayWarning, RemovedText, minute_of_day};
pub use device::check_repo_path;
pub use error::{EditError, ReadError, SaveError};
pub use file::content_hash;
pub use git_log::{Commit, GitLogError, git_log};
pub use id::{BlockId, InvalidId, LocationKey, NotePath, ProjectSlug, TaskId};
pub use init::CreateError;
pub use markdown::{
    Callout, CalloutKind, CodeBlock, Continuation, Formatting, InlineMarkup, ListIndent,
    MarkdownMode, MarkdownStyle, TableLine, TaskItem, WebLink, continue_list, continue_quote,
    escape_headings, markdown_formatting, nest_list_item,
};
pub use notes::{NoteFile, SavedNote, WikiLink, wiki_links};
pub use period::{Period, week_start};
pub use project::{Project, ProjectStatus};
pub use tasks::{Task, TaskList, TaskStatus};
pub use vault::{DayFile, Vault};
pub use watch::{VaultChange, VaultWatcher, WatchError};
