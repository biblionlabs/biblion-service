use rusqlite::{Result, Row};
use serde::{Deserialize, Serialize};

use crate::{Crud, FromRow, Queriable};

use common::BibleVersion;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbBible {
    pub id: String,
    pub name_local: String,
    pub name_english: String,
    pub language_id: String,
}

impl Queriable for DbBible {
    const TABLE: &'static str = "bibles";
    const FIELDS: &'static [&'static str] = &["id", "name_local", "name_english", "language_id"];
}

impl FromRow for DbBible {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name_local: row.get("name_local")?,
            name_english: row.get("name_english")?,
            language_id: row.get("language_id")?,
        })
    }
}

impl Crud for DbBible {
    fn params<'a>(&'a self) -> Vec<&'a dyn rusqlite::ToSql> {
        vec![
            &self.id,
            &self.name_local,
            &self.name_english,
            &self.language_id,
        ]
    }
}

impl From<BibleVersion> for DbBible {
    fn from(b: BibleVersion) -> Self {
        Self {
            id: b.id,
            name_local: b.name.clone(),
            name_english: b.name,
            language_id: b.language_id,
        }
    }
}
