use crate::error::Result;
use crate::models;

mod sqlite;

pub use sqlite::*;

/// Trait that consumers implement to receive parsed data and insert to DB.
///
/// Methods are async so sinking can perform DB I/O concurrently.
pub trait DbSink: Send + Sync {
    async fn insert_cross_reference(
        &self,
        _book: &str,
        _data: &models::CrossReference,
    ) -> Result<()> {
        Ok(())
    }

    async fn insert_language(
        &self,
        _lang_id: &str,
        _direction: &str,
        _name_local: &str,
        _name_english: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn insert_bible(
        &self,
        _bible_id: &str,
        _name_local: &str,
        _name_english: &str,
        _language_id: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn insert_header(
        &self,
        _bible_id: &str,
        _book_id: &str,
        _chapter: usize,
        _text: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn insert_book_meta(
        &self,
        _bible_id: &str,
        _book_id: &str,
        _name_normal: &str,
        _name_long: &str,
        _name_abbrev: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn insert_verse(
        &self,
        _book_id: &str,
        _chapter: usize,
        _verse: usize,
        _text: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn insert_note(
        &self,
        _book_id: &str,
        _chapter: usize,
        _verse: usize,
        _text: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn finalize(&self) -> Result<()> {
        Ok(())
    }
}
