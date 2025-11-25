use rusqlite::{Result, Row};
use serde::{Deserialize, Serialize};

use crate::{Crud, FromRow, Queriable};

use common::Verse;

mod index;
mod search;

pub use index::*;
pub use search::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbVerse {
    pub id: i64,
    pub book_id: String,
    pub chapter: i32,
    pub verse: i32,
    pub text: String,
}

impl Queriable for DbVerse {
    const TABLE: &'static str = "verses";
    const FIELDS: &'static [&'static str] =
        &["id", "book_id", "chapter_number", "verse_number", "text"];
}

impl FromRow for DbVerse {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            book_id: row.get("book_id")?,
            chapter: row.get("chapter")?,
            verse: row.get("verse")?,
            text: row.get("text")?,
        })
    }
}

impl Crud for DbVerse {
    fn params<'a>(&'a self) -> Vec<&'a dyn rusqlite::ToSql> {
        vec![
            &self.id,
            &self.book_id,
            &self.chapter,
            &self.verse,
            &self.text,
        ]
    }
}

impl From<Verse> for DbVerse {
    fn from(v: Verse) -> Self {
        Self {
            id: v.id,
            text: v.text,
            book_id: v.book_id,
            chapter: v.chapter,
            verse: v.verse,
        }
    }
}
