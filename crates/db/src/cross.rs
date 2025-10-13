use rusqlite::{Result, Row};
use serde::{Deserialize, Serialize};

use crate::{Crud, FromRow, Queriable};

use common::CrossReference;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbCrossReference {
    pub id: i64,
    pub source_book: String,
    pub source_chapter: i32,
    pub source_verse: i32,
    pub target_book: String,
    pub target_chapter: i32,
    pub target_verse: i32,
}

impl Queriable for DbCrossReference {
    const TABLE: &'static str = "cross_references";
    const FIELDS: &'static [&'static str] = &[
        "id",
        "source_book",
        "source_chapter",
        "source_verse",
        "target_book",
        "target_chapter",
        "target_verse",
    ];
}

impl FromRow for DbCrossReference {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            source_book: row.get("source_book")?,
            source_chapter: row.get("source_chapter")?,
            source_verse: row.get("source_verse")?,
            target_book: row.get("target_book")?,
            target_chapter: row.get("target_chapter")?,
            target_verse: row.get("target_verse")?,
        })
    }
}

impl Crud for DbCrossReference {
    fn params<'a>(&'a self) -> Vec<&'a dyn rusqlite::ToSql> {
        vec![
            &self.id,
            &self.source_book,
            &self.source_chapter,
            &self.source_verse,
            &self.target_book,
            &self.target_chapter,
            &self.target_verse,
        ]
    }
}

impl From<CrossReference> for DbCrossReference {
    fn from(c: CrossReference) -> Self {
        Self {
            id: c.id,
            source_book: String::new(),
            source_chapter: 0,
            source_verse: c.from_verse_id,
            target_book: String::new(),
            target_chapter: 0,
            target_verse: c.to_verse_id,
        }
    }
}
