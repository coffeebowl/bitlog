//! The index of the open vault, used on worker threads so that the window
//! never waits for it.

use std::sync::{Arc, Mutex};

use gtk::{gio, glib};
use knotbook_core::{NotePath, Vault};
use knotbook_index::{Found, Index, IndexError, SearchHit};

/// Clones share the same index.
#[derive(Debug, Clone, Default)]
pub struct SearchIndex {
    /// `None` until first used. A lock also keeps a search from reading
    /// while an update writes.
    index: Arc<Mutex<Option<Index>>>,
}

impl SearchIndex {
    /// Brings the index up to date with the files of `vault`, opening or
    /// creating it first if needed. Files that cannot be read are left out
    /// with a warning; `knotbook doctor` tells more about them.
    pub async fn update(&self, vault: &Vault) -> Result<(), IndexError> {
        let skipped = self.run(vault, |index, vault| index.refresh(vault)).await?;
        for err in skipped {
            glib::g_warning!("knotbook", "{err}");
        }
        Ok(())
    }

    /// Searches the index, see [`Index::search`].
    pub async fn search(
        &self,
        vault: &Vault,
        query: String,
        limit: u32,
    ) -> Result<Vec<SearchHit>, IndexError> {
        self.run(vault, move |index, _| index.search(&query, limit))
            .await
    }

    /// Where the wiki links to `note` lie, after bringing the index up to
    /// date, see [`Index::backlinks`].
    pub async fn backlinks(&self, vault: &Vault, note: NotePath) -> Result<Vec<Found>, IndexError> {
        let (skipped, links) = self
            .run(vault, move |index, vault| {
                Ok((index.refresh(vault)?, index.backlinks(&note)?))
            })
            .await?;
        for err in skipped {
            glib::g_warning!("knotbook", "{err}");
        }
        Ok(links)
    }

    async fn run<T: Send + 'static>(
        &self,
        vault: &Vault,
        work: impl FnOnce(&mut Index, &Vault) -> Result<T, IndexError> + Send + 'static,
    ) -> Result<T, IndexError> {
        let index = self.index.clone();
        let vault = vault.clone();
        gio::spawn_blocking(move || {
            let mut index = index.lock().expect("no thread panics holding the index");
            if index.is_none() {
                *index = Some(Index::open(&vault)?);
            }
            work(index.as_mut().expect("the index was just opened"), &vault)
        })
        .await
        .expect("working on the index does not panic")
    }
}
