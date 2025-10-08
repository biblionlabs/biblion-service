use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Language {
    pub id: String,
    pub name_local: String,
    pub name_english: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bible {
    pub id: String,
    pub name_local: String,
    pub name_english: String,
    pub language_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Book {
    pub id: String,
    pub bible_id: String,
    pub name_normal: String,
    pub name_long: String,
    pub name_abbrev: String,
    pub order_index: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub id: i64,
    pub book_id: String,
    pub chapter_number: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verse {
    pub id: i64,
    pub chapter_id: i64,
    pub verse_number: i32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossReference {
    pub id: i64,
    pub source_book: String,
    pub source_chapter: i32,
    pub source_verse: i32,
    pub target_book: String,
    pub target_chapter: i32,
    pub target_verse: i32,
}
