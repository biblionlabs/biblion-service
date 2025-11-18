use std::path::PathBuf;

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use service_db::{
    Connection, Crud, DbBible, DbBook, DbCrossReference, DbHeader, DbLanguage, DbVerse,
    DbVerseNote, Sqlite,
};

use crate::{CrossReference, Reference, Result};

use super::DbSink;

// Example DbSink implementation that uses the repo's `db` crate to insert rows.
// Adjust imports according to your crate layout.
pub struct SqliteDbSink {
    // here we could keep a connection pool, etc.
    // For demo purposes we'll open a new connection for each operation (not optimal).
    conn: Connection,
}

impl From<String> for SqliteDbSink {
    fn from(db_path: String) -> Self {
        let db = Sqlite::new(db_path).unwrap();
        let conn = db.connection();
        Self { conn }
    }
}

impl From<PathBuf> for SqliteDbSink {
    fn from(db_path: PathBuf) -> Self {
        let db = Sqlite::new(db_path).unwrap();
        let conn = db.connection();
        Self { conn }
    }
}

impl DbSink for SqliteDbSink {
    async fn insert_cross_reference(&self, book: &str, data: &CrossReference) -> Result<()> {
        data.par_iter().for_each(|(source_chapter, cross)| {
            cross.par_iter().for_each(|(source_verse, cross)| {
                let mut targets = 0;
                let mut target_book = &Default::default();
                let mut target_chapter = Vec::new();
                let mut target_verse = Vec::new();
                for target in cross {
                    let mut iter = target.iter();
                    if let Some(Reference::String(book)) = iter.next() {
                        target_book = book;
                    }
                    let iter = iter.collect::<Vec<_>>();
                    let iter = iter.chunks(2);
                    targets = iter.clone().count();
                    for target in iter {
                        let mut iter = target.iter();
                        if let Some(Reference::Integer(chapter)) = iter.next() {
                            target_chapter.push(*chapter);
                        }
                        if let Some(Reference::Integer(verse)) = iter.next() {
                            target_verse.push(*verse);
                        }
                    }
                }

                for t in 0..targets {
                    let data = DbCrossReference {
                        id: 0,
                        source_book: book.to_string(),
                        source_chapter: source_chapter.parse().unwrap(),
                        source_verse: source_verse.parse().unwrap(),
                        target_book: target_book.clone(),
                        target_chapter: target_chapter[t],
                        target_verse: target_verse[t],
                    };
                    data.insert(self.conn.clone()).unwrap();
                }
            });
        });
        Ok(())
    }

    async fn insert_language(
        &self,
        lang_id: &str,
        direction: &str,
        name_local: &str,
        name_english: &str,
    ) -> Result<()> {
        let data = DbLanguage {
            id: lang_id.to_string(),
            direction: direction.to_string(),
            name_local: name_local.to_string(),
            name_english: name_english.to_string(),
        };
        data.insert_with_id(self.conn.clone()).unwrap();
        Ok(())
    }

    async fn insert_bible(
        &self,
        bible_id: &str,
        name_local: &str,
        name_english: &str,
        language_id: &str,
    ) -> Result<()> {
        let data = DbBible {
            id: bible_id.to_string(),
            name_local: name_local.to_string(),
            name_english: name_english.to_string(),
            language_id: language_id.to_string(),
        };
        data.insert_with_id(self.conn.clone()).unwrap();
        Ok(())
    }

    async fn insert_header(
        &self,
        bible_id: &str,
        book_id: &str,
        chapter: usize,
        text: &str,
    ) -> Result<()> {
        let data = DbHeader {
            id: 0,
            bible_id: bible_id.to_string(),
            book_id: book_id.to_string(),
            chapter: chapter as _,
            text: text.to_string(),
        };
        data.insert(self.conn.clone()).unwrap();
        Ok(())
    }

    async fn insert_book_meta(
        &self,
        bible_id: &str,
        book_id: &str,
        name_normal: &str,
        name_long: &str,
        name_abbrev: &str,
    ) -> Result<()> {
        let data = DbBook {
            id: book_id.to_string(),
            bible_id: bible_id.to_string(),
            name_normal: name_normal.to_string(),
            name_long: name_long.to_string(),
            name_abbrev: name_abbrev.to_string(),
        };
        data.insert_with_id(self.conn.clone()).unwrap();
        Ok(())
    }

    async fn insert_verse(
        &self,
        book_id: &str,
        chapter: usize,
        verse: usize,
        text: &str,
    ) -> Result<()> {
        let data = DbVerse {
            id: 0,
            book_id: book_id.to_string(),
            chapter: chapter as _,
            verse: verse as _,
            text: text.to_string(),
        };
        data.insert(self.conn.clone()).unwrap();
        Ok(())
    }

    async fn insert_note(
        &self,
        book_id: &str,
        chapter: usize,
        verse: usize,
        text: &str,
    ) -> Result<()> {
        let data = DbVerseNote {
            id: 0,
            book_id: book_id.to_string(),
            chapter: chapter as _,
            verse: verse as _,
            text: text.to_string(),
        };
        data.insert(self.conn.clone()).unwrap();
        Ok(())
    }

    async fn finalize(&self) -> Result<()> {
        // any final steps like rebuilding indices
        Ok(())
    }
}
