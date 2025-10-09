use limbo::{Result, Row, Value};
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
            id: row.get_value(0)?.as_text().unwrap().clone(),
            name_local: row.get_value(1)?.as_text().unwrap().clone(),
            name_english: row.get_value(2)?.as_text().unwrap().clone(),
            language_id: row.get_value(3)?.as_text().unwrap().clone(),
        })
    }
}

impl Crud for DbBible {
    fn to_params(&self) -> Vec<Value> {
        vec![
            Value::from(self.id.as_str()),
            Value::from(self.name_local.as_str()),
            Value::from(self.name_english.as_str()),
            Value::from(self.language_id.as_str()),
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
