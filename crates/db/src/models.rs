use serde::{Serialize, Deserialize};






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
