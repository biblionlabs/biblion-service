use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Language {
    pub id: String,
    pub direction: String,
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
    pub name_long: String,
    pub abbrev: String,
    pub testament: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub id: i64,
    pub bible_id: String,
    pub book_id: String,
    pub chapter: i32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verse {
    pub id: i64,
    pub book_id: String,
    pub chapter: i32,
    pub verse: i32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossReference {
    pub id: i64,
    pub from_verse_id: i32,
    pub to_verse_id: i32,
}
