use std::path::PathBuf;
use std::sync::Arc;

use service_db::{
    ChapterSearchResult, ChapterVerse, CrossRefIndex, CrossReferenceVerse, FieldValue,
    IndexedCrossReference, IndexedVerse, SynonymMap, VerseIndex,
};

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
        let cross_path = base_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("cross_index");
        let cross_index = Arc::new(CrossRefIndex::new(cross_path).ok());
        Self { index, cross_index }
    }
}

impl From<PathBuf> for TantivySink {
    fn from(index_path: PathBuf) -> Self {
        let index = Arc::new(VerseIndex::new(&index_path).ok());
        let cross_path = index_path
            .parent()
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
        if let Some(cross_idx) = self.cross_index.as_ref().as_ref() {
            Ok(cross_idx.has_documents())
        } else {
            Ok(false)
        }
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

        match index.search(bible_id, Some(1), false) {
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
            Err(_) => Ok(BibleInstallStatus::NotInstalled),
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

    fn get_crossreferences(
        &self,
        bible_id: &str,
        book_id: &str,
        chapter_idx: i32,
        verse_idx: i32,
    ) -> Result<Option<ChapterSearchResult>> {
        let idx = self
            .index
            .as_ref()
            .as_ref()
            .ok_or_else(|| crate::Error::Other("Verse index not available".into()))?;

        let search_results = idx
            .lookup_by_fields(vec![
                ("bible_id", FieldValue::Text(bible_id.to_string())),
                ("book_id", FieldValue::Text(book_id.to_string())),
                ("chapter", FieldValue::I64(chapter_idx as i64)),
                ("verse", FieldValue::I64(chapter_idx as i64)),
            ])
            .map_err(service_db::Error::from)?;

        let Some(result) = search_results.first() else {
            return Err(service_db::Error::NoSearchResult.into());
        };

        let chapter_verses = idx
            .lookup_by_fields(vec![
                ("book_id", FieldValue::Text(book_id.to_string())),
                ("chapter", FieldValue::I64(chapter_idx as i64)),
            ])
            .map_err(service_db::Error::from)?
            .into_iter()
            .map(|v| (v.verse, v.text))
            .collect::<Vec<_>>();

        let cross_idx = self.cross_index.as_ref().as_ref().unwrap();

        let verses = chapter_verses
            .into_iter()
            .map(|(verse_num, text)| {
                let mut cross_references = Vec::new();
                if let Ok(cross_results) = cross_idx.lookup_by_fields(vec![
                    ("source_book", FieldValue::Text(book_id.to_string())),
                    ("source_chapter", FieldValue::I64(chapter_idx as i64)),
                    ("source_verse", FieldValue::I64(verse_num as i64)),
                ]) {
                    for cr in cross_results {
                        let text = idx
                            .lookup_by_fields(vec![
                                ("book_id", FieldValue::Text(cr.target_book.clone())),
                                ("chapter", FieldValue::I64(cr.target_chapter as i64)),
                                ("verse", FieldValue::I64(cr.target_verse as i64)),
                            ])
                            .ok()
                            .and_then(|r| r.into_iter().next())
                            .map(|v| v.text)
                            .unwrap_or_default();

                        cross_references.push(CrossReferenceVerse {
                            text,
                            book_id: cr.target_book,
                            book_name: cr.target_book_name,
                            chapter: cr.target_chapter,
                            verse: cr.target_verse,
                        });
                    }
                }

                ChapterVerse {
                    verse_number: verse_num,
                    text,
                    highlighted: verse_num == verse_idx,
                    cross_references,
                }
            })
            .collect::<Vec<_>>();

        Ok(Some(ChapterSearchResult {
            bible_id: bible_id.to_string(),
            bible_name: result.bible_name.clone(),
            book_id: book_id.to_string(),
            book_name: result.book_name.clone(),
            chapter: chapter_idx,
            headers: vec![],
            verses,
        }))
    }
}
