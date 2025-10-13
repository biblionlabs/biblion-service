use rusqlite::{Result, Row};
use serde::{Deserialize, Serialize};

use crate::{Crud, FromRow, Queriable};

use common::Book;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbBook {
    pub id: String,
    pub bible_id: String,
    pub name_normal: String,
    pub name_long: String,
    pub name_abbrev: String,
}

impl Queriable for DbBook {
    const TABLE: &'static str = "books";
    const FIELDS: &'static [&'static str] =
        &["id", "bible_id", "name_normal", "name_long", "name_abbrev"];
}

impl FromRow for DbBook {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            bible_id: row.get("bible_id")?,
            name_normal: row.get("name_normal")?,
            name_long: row.get("name_long")?,
            name_abbrev: row.get("name_abbrev")?,
        })
    }
}

impl Crud for DbBook {
    fn params<'a>(&'a self) -> Vec<&'a dyn rusqlite::ToSql> {
        vec![
            &self.id,
            &self.bible_id,
            &self.name_normal,
            &self.name_long,
            &self.name_abbrev,
        ]
    }
}

impl From<Book> for DbBook {
    fn from(b: Book) -> Self {
        Self {
            id: b.id.clone(),
            bible_id: b.id,
            name_normal: b.name,
            name_long: b.name_long,
            name_abbrev: b.abbrev,
        }
    }
}
