use limbo::{Result, Row, Value};
use serde::{Deserialize, Serialize};

use crate::{Crud, FromRow, Queriable};

use common::Verse;

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
            id: *row.get_value(0)?.as_integer().unwrap_or(&0),
            book_id: row.get_value(1)?.as_text().unwrap().clone(),
            chapter: *row.get_value(2)?.as_integer().unwrap_or(&0) as _,
            verse: *row.get_value(3)?.as_integer().unwrap_or(&0) as _,
            text: row.get_value(4)?.as_text().unwrap().clone(),
        })
    }
}

impl Crud for DbVerse {
    fn to_params(&self) -> Vec<Value> {
        vec![
            Value::from(self.id),
            Value::from(self.book_id.as_str()),
            Value::from(self.chapter),
            Value::from(self.verse),
            Value::from(self.text.as_str()),
        ]
    }
}

impl From<Verse> for DbVerse {
    fn from(v: Verse) -> Self {
        Self {
            id: v.id.unwrap_or_default(),
            text: v.text,
            book_id: v.book_id,
            chapter: v.chapter,
            verse: v.verse,
        }
    }
}
