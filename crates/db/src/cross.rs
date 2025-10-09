use limbo::{Result, Row, Value};
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
            id: *row.get_value(0)?.as_integer().unwrap_or(&0),
            source_book: row.get_value(1)?.as_text().unwrap().clone(),
            source_chapter: *row.get_value(2)?.as_integer().unwrap_or(&0) as _,
            source_verse: *row.get_value(3)?.as_integer().unwrap_or(&0) as _,
            target_book: row.get_value(4)?.as_text().unwrap().clone(),
            target_chapter: *row.get_value(5)?.as_integer().unwrap_or(&0) as _,
            target_verse: *row.get_value(6)?.as_integer().unwrap_or(&0) as _,
        })
    }
}

impl Crud for DbCrossReference {
    fn to_params(&self) -> Vec<Value> {
        vec![
            Value::from(self.source_book.as_str()),
            Value::from(self.source_chapter),
            Value::from(self.source_verse),
            Value::from(self.target_book.as_str()),
            Value::from(self.target_chapter),
            Value::from(self.target_verse),
        ]
    }
}

impl From<CrossReference> for DbCrossReference {
    fn from(c: CrossReference) -> Self {
        Self {
            id: c.id.unwrap_or_default(),
            source_book: String::new(),
            source_chapter: 0,
            source_verse: c.from_verse_id,
            target_book: String::new(),
            target_chapter: 0,
            target_verse: c.to_verse_id,
        }
    }
}
