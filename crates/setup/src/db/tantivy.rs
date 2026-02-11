use std::path::PathBuf;
use std::sync::Arc;

use service_db::{CrossRefIndex, IndexedCrossReference, IndexedVerse, SynonymMap, VerseIndex};

use crate::{BibleInstallStatus, Result};

use super::DbSink;

pub struct TantivySink {
    index: Arc<Option<VerseIndex>>,
    cross_index: Arc<Option<CrossRefIndex>>,
}

impl From<String> for TantivySink {
    fn from(db_path: String) -> Self {
        let base_path = PathBuf::from(&db_path);
        let index = Arc::new(VerseIndex::new(&base_path).ok());
        let cross_path = base_path.parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("cross_index");
        let cross_index = Arc::new(CrossRefIndex::new(cross_path).ok());
        Self { index, cross_index }
    }
}

impl From<PathBuf> for TantivySink {
    fn from(index_path: PathBuf) -> Self {
        let index = Arc::new(VerseIndex::new(&index_path).ok());
        let cross_path = index_path.parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("cross_index");
        let cross_index = Arc::new(CrossRefIndex::new(cross_path).ok());
        Self { index, cross_index }
    }
}

impl TantivySink {
    pub fn verse_index(&self) -> Arc<Option<VerseIndex>> {
        self.index.clone()
    }

    pub fn cross_ref_index(&self) -> Arc<Option<CrossRefIndex>> {
        self.cross_index.clone()
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
        bible_id: &str,
        expected_books: &[(String, usize)],
    ) -> Result<BibleInstallStatus> {
        let maybe_index = self.index.as_ref();
        let Some(index) = maybe_index else {
            return Ok(BibleInstallStatus::NotInstalled);
        };

        if index.is_new() {
            return Ok(BibleInstallStatus::NotInstalled);
        }

        match index.search(bible_id, 1, false) {
            Ok(res) => {
                if res.is_empty() {
                    Ok(BibleInstallStatus::NotInstalled)
                } else {
                    Ok(BibleInstallStatus::Complete {
                        total_books: expected_books.len(),
                        total_verses: expected_books.iter().map(|(_, len)| *len).sum(),
                    })
                }
            }
            Err(_) => {
                Ok(BibleInstallStatus::NotInstalled)
            }
        }
    }

    fn clear_index(&self) -> Result<()> {
        if let Some(index) = self.index.as_ref() {
            index.clear()?;
        }
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

    fn clear_cross_ref_index(&self) -> Result<()> {
        if let Some(index) = self.cross_index.as_ref() {
            index.clear()?;
        }
        Ok(())
    }

    fn index_cross_references(&self, refs: Vec<IndexedCrossReference>) -> Result<()> {
        let Some(index) = self.cross_index.as_ref() else {
            return Ok(());
        };

        index
            .index_cross_references_batch(refs)
            .map_err(service_db::Error::from)
            .map_err(crate::Error::from)?;

        Ok(())
    }

    fn load_verse_synonyms(&self, map: SynonymMap) -> Result<()> {
        if let Some(index) = self.index.as_ref() {
            index.load_synonyms(map);
        }
        Ok(())
    }

    fn finalize(&self) -> Result<()> {
        Ok(())
    }
}
