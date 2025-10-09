use limbo::{Connection, Result, Value};
use serde::{Deserialize, Serialize};

use crate::{Crud, FromRow, Queriable};

use common::Language;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbLanguage {
    pub id: String,
    pub name_local: String,
    pub name_english: String,
}

impl Queriable for DbLanguage {
    const TABLE: &str = "languages";
    const FIELDS: &[&str] = &["id", "name_local", "name_english"];
}

impl DbLanguage {
    pub async fn from_id(conn: &Connection, name: &str) -> Result<Option<Self>> {
        Self::select_where(conn, "name_local", [name])
            .await
            .map(|v| v.into_iter().next())
    }

    pub async fn from_name_local(conn: &Connection, id: &str) -> Result<Option<Self>> {
        Self::select_where(conn, "id", [id])
            .await
            .map(|v| v.into_iter().next())
    }

    pub async fn from_name_english(conn: &Connection, name: &str) -> Result<Option<Self>> {
        Self::select_where(conn, "name_english", [name])
            .await
            .map(|v| v.into_iter().next())
    }
}

impl FromRow for DbLanguage {
    fn from_row(row: &limbo::Row) -> Result<Self> {
        Ok(Self {
            id: row.get_value(0)?.as_text().unwrap().clone(),
            name_local: row.get_value(1)?.as_text().unwrap().clone(),
            name_english: row.get_value(2)?.as_text().unwrap().clone(),
        })
    }
}

impl Crud for DbLanguage {
    fn to_params(&self) -> Vec<Value> {
        vec![
            Value::from(self.id.as_str()),
            Value::from(self.name_local.as_str()),
            Value::from(self.name_english.as_str()),
        ]
    }
}

impl From<Language> for DbLanguage {
    fn from(v: Language) -> Self {
        Self {
            id: v.id,
            name_local: v.local,
            name_english: v.english,
        }
    }
}
