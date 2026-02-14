use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

use super::VerseIndex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchedBible {
    pub id: String,
    pub name: String,
    pub english_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchedVerse {
    pub bible: SearchedBible,
    pub book: String,
    pub book_id: String,
    pub chapter: i32,
    pub part: i32,
    pub verse: (i32, i32),
    pub text: String,
}

impl SearchedVerse {
    pub fn from_search(search: &str, index: Arc<Option<VerseIndex>>, limit: Option<usize>) -> Result<Vec<Self>> {
        if let Some(idx) = index.as_ref() {
            if let Ok(results) = idx
                .search(search, limit, true)
                .inspect_err(|e| println!("Search Fail: {e}"))
            {
                return Ok(results
                    .into_iter()
                    .map(|r| SearchedVerse {
                        bible: SearchedBible {
                            id: r.bible_id,
                            name: r.bible_name,
                            english_name: "".to_string(),
                        },
                        book: r.book_name,
                        book_id: r.book_id,
                        chapter: r.chapter,
                        part: 0,
                        verse: (r.verse, 1),
                        text: r.text,
                    })
                    .collect());
            }
        }

        Err(Error::NoSearchResult.into())
    }
}
