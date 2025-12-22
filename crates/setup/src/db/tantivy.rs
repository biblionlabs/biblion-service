use std::path::PathBuf;

use service_db::{IndexedVerse, VerseIndex};

use crate::{BibleInstallStatus, Result};

use super::DbSink;

pub struct TantivySink {
    index: Option<VerseIndex>,
}

impl From<String> for TantivySink {
    fn from(db_path: String) -> Self {
        let index_path = PathBuf::from(db_path);
        let index = VerseIndex::new(index_path).ok();
        Self { index }
    }
}

impl From<PathBuf> for TantivySink {
    fn from(index_path: PathBuf) -> Self {
        let index = VerseIndex::new(index_path).ok();
        Self { index }
    }
}

impl DbSink for TantivySink {
    fn has_cross_references(&self, _expected_books: &[String]) -> Result<bool> {
        Ok(true)
    }

    fn has_languages(&self, _expected_langs: &[String]) -> Result<bool> {
        Ok(true)
    }

    fn get_bible_install_stats(
        &self,
        _bible_id: &str,
        expected_books: &[(String, usize)],
    ) -> Result<BibleInstallStatus> {
        let Some(index) = self.index.as_ref() else {
            return Err(crate::Error::Other("Index not intializated".into()));
        };

        if index.is_new() {
            return Ok(BibleInstallStatus::NotInstalled);
        }

        Ok(BibleInstallStatus::Complete {
            total_books: expected_books.len(),
            total_verses: expected_books.iter().map(|(_, len)| *len).sum(),
        })
    }

    fn clear_index(&self) -> Result<()> {
        let Some(index) = self.index.as_ref() else {
            return Ok(());
        };

        index.clear()?;

        Ok(())
    }

    fn index_bible_verses(&self, verses: Vec<IndexedVerse>) -> Result<()> {
        let Some(index) = self.index.as_ref() else {
            return Ok(());
        };

        index
            .index_verses_batch(verses)
            .map_err(service_db::Error::from)
            .map_err(crate::Error::from)?;

        Ok(())
    }

    fn finalize(&self) -> Result<()> {
        // any final steps like rebuilding indices
        Ok(())
    }
}
