use limbo::{Result, Row, Value};
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
    pub order_index: i32,
}

impl Queriable for DbBook {
    const TABLE: &'static str = "books";
    const FIELDS: &'static [&'static str] = &[
        "id",
        "bible_id",
        "name_normal",
        "name_long",
        "name_abbrev",
        "order_index",
    ];
}

impl FromRow for DbBook {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: row.get_value(0)?.as_text().unwrap().clone(),
            bible_id: row.get_value(1)?.as_text().unwrap().clone(),
            name_normal: row.get_value(2)?.as_text().unwrap().clone(),
            name_long: row.get_value(3)?.as_text().unwrap().clone(),
            name_abbrev: row.get_value(4)?.as_text().unwrap().clone(),
            order_index: *row.get_value(5)?.as_integer().unwrap_or(&0) as _,
        })
    }
}

impl Crud for DbBook {
    fn to_params(&self) -> Vec<Value> {
        vec![
            Value::from(self.id.as_str()),
            Value::from(self.bible_id.as_str()),
            Value::from(self.name_normal.as_str()),
            Value::from(self.name_long.as_str()),
            Value::from(self.name_abbrev.as_str()),
            Value::from(self.order_index),
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
            order_index: b.order,
        }
    }
}
