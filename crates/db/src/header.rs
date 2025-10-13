use rusqlite::{Result, Row};
use serde::{Deserialize, Serialize};

use crate::{Crud, FromRow, Queriable};

use common::Header;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbHeader {
    pub id: i64,
    pub bible_id: String,
    pub book_id: String,
    pub chapter: i32,
    pub text: String,
}

impl Queriable for DbHeader {
    const TABLE: &'static str = "headers";
    const FIELDS: &'static [&'static str] =
        &["id", "bible_id", "book_id", "chapter_number", "text"];
}

impl FromRow for DbHeader {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            bible_id: row.get("bible_id")?,
            book_id: row.get("book_id")?,
            chapter: row.get("chapter")?,
            text: row.get("text")?,
        })
    }
}

impl Crud for DbHeader {
    fn params<'a>(&'a self) -> Vec<&'a dyn rusqlite::ToSql> {
        vec![
            &self.id,
            &self.bible_id,
            &self.book_id,
            &self.chapter,
            &self.text,
        ]
    }
}

impl From<Header> for DbHeader {
    fn from(v: Header) -> Self {
        Self {
            id: v.id,
            bible_id: v.bible_id,
            book_id: v.book_id,
            chapter: v.chapter,
            text: v.text,
        }
    }
}
