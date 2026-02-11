use crate::error::Result;
use crate::{BibleInstallStatus, models};

mod sqlite;
mod tantivy;

use service_db::{IndexedCrossReference, IndexedVerse, SynonymMap};
pub use sqlite::*;
pub use tantivy::*;

/// Trait that consumers implement to receive parsed data and insert to DB.
///
/// Methods are so sinking can perform DB I/O concurrently.
pub trait DbSink: Send + Sync {
    fn has_cross_references(&self, _expected_books: &[String]) -> Result<bool> {
        Ok(false)
    }

    fn has_languages(&self, _expected_langs: &[String]) -> Result<bool> {
        Ok(false)
    }

    fn is_bible_installed(
        &self,
        bible_id: &str,
        expected_books: &[(String, usize)],
    ) -> Result<bool> {
        self.get_bible_install_stats(bible_id, expected_books)
            .map(|s| s.is_complete())
    }

    fn get_bible_install_stats(
        &self,
        _bible_id: &str,
        _expected_books: &[(String, usize)],
    ) -> Result<BibleInstallStatus> {
        Ok(BibleInstallStatus::NotInstalled)
    }

    fn insert_cross_reference(&self, _book: &str, _data: &models::CrossReference) -> Result<()> {
        Ok(())
    }

    fn insert_language(
        &self,
        _lang_id: &str,
        _direction: &str,
        _name_local: &str,
        _name_english: &str,
    ) -> Result<()> {
        Ok(())
    }

    fn insert_bible(
        &self,
        _bible_id: &str,
        _name_local: &str,
        _name_english: &str,
        _language_id: &str,
    ) -> Result<()> {
        Ok(())
    }

    fn insert_header(
        &self,
        _bible_id: &str,
        _book_id: &str,
        _chapter: usize,
        _text: &str,
    ) -> Result<()> {
        Ok(())
    }

    fn insert_book_meta(
        &self,
        _bible_id: &str,
        _book_id: &str,
        _name_normal: &str,
        _name_long: &str,
        _name_abbrev: &str,
    ) -> Result<()> {
        Ok(())
    }

    fn insert_verse(
        &self,
        _book_id: &str,
        _chapter: usize,
        _verse: usize,
        _text: &str,
    ) -> Result<()> {
        Ok(())
    }

    fn clear_index(&self) -> Result<()> {
        Ok(())
    }

    fn index_bible_verses(&self, _verses: Vec<IndexedVerse>) -> Result<()> {
        Ok(())
    }

    fn clear_cross_ref_index(&self) -> Result<()> {
        Ok(())
    }

    fn index_cross_references(&self, _refs: Vec<IndexedCrossReference>) -> Result<()> {
        Ok(())
    }

    fn load_verse_synonyms(&self, _map: SynonymMap) -> Result<()> {
        Ok(())
    }

    fn insert_note(
        &self,
        _book_id: &str,
        _chapter: usize,
        _verse: usize,
        _text: &str,
    ) -> Result<()> {
        Ok(())
    }

    fn finalize(&self) -> Result<()> {
        Ok(())
    }
}
