use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Language {
    pub id: String,
    pub english: String,
    pub local: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BibleVersion {
    pub id: String,
    pub name: String,
    pub language_id: String,
    pub year: Option<i32>,
    pub has_nt: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Book {
    pub id: String,
    pub name: String,
    pub order: i32,
    pub testament: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verse {
    pub id: Option<i64>,
    pub book_id: String,
    pub chapter: i32,
    pub verse: i32,
    pub text: String,
    pub bible_version_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossReference {
    pub id: Option<i64>,
    pub from_verse_id: i64,
    pub to_verse_id: i64,
}
