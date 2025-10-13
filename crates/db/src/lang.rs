use rusqlite::{Result, Row};
use serde::{Deserialize, Serialize};

use crate::{Connection, Crud, FromRow, Queriable};

use common::Language;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbLanguage {
    pub id: String,
    pub direction: String,
    pub name_local: String,
    pub name_english: String,
}

impl Queriable for DbLanguage {
    const TABLE: &str = "languages";
    const FIELDS: &[&str] = &["id", "direction", "name_local", "name_english"];
}

impl DbLanguage {
    pub fn from_id(conn: Connection, name: &str) -> Result<Option<Self>> {
        Self::select_where(conn, "name_local", vec![&name]).map(|v| v.into_iter().next())
    }

    pub fn from_name_local(conn: Connection, id: &str) -> Result<Option<Self>> {
        Self::select_where(conn, "id", vec![&id]).map(|v| v.into_iter().next())
    }

    pub fn from_name_english(conn: Connection, name: &str) -> Result<Option<Self>> {
        Self::select_where(conn, "name_english", vec![&name]).map(|v| v.into_iter().next())
    }
}

impl FromRow for DbLanguage {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            direction: row.get("direction")?,
            name_local: row.get("name_local")?,
            name_english: row.get("name_english")?,
        })
    }
}

impl Crud for DbLanguage {
    fn params<'a>(&'a self) -> Vec<&'a dyn rusqlite::ToSql> {
        vec![
            &self.id,
            &self.direction,
            &self.name_local,
            &self.name_english,
        ]
    }
}

impl From<Language> for DbLanguage {
    fn from(v: Language) -> Self {
        Self {
            id: v.id,
            direction: v.direction,
            name_local: v.local,
            name_english: v.english,
        }
    }
}
