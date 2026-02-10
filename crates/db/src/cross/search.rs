use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

use super::{CrossRefIndex, SearchDirection};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchedCrossReference {
    pub source_book: String,
    pub source_book_name: String,
    pub source_chapter: i32,
    pub source_verse: i32,
    pub target_book: String,
    pub target_book_name: String,
    pub target_chapter: i32,
    pub target_verse: i32,
}

impl SearchedCrossReference {
    pub fn from_search(
        search: &str,
        index: Arc<Option<CrossRefIndex>>,
        limit: Option<usize>,
        direction: Option<SearchDirection>,
    ) -> Result<Vec<Self>> {
        if let Some(idx) = index.as_ref() {
            let dir = direction.unwrap_or(SearchDirection::Both);
            if let Ok(results) = idx.search(search, limit.unwrap_or(50), dir) {
                return Ok(results
                    .into_iter()
                    .map(|r| SearchedCrossReference {
                        source_book: r.source_book,
                        source_book_name: r.source_book_name,
                        source_chapter: r.source_chapter,
                        source_verse: r.source_verse,
                        target_book: r.target_book,
                        target_book_name: r.target_book_name,
                        target_chapter: r.target_chapter,
                        target_verse: r.target_verse,
                    })
                    .collect());
            }
        }

        Err(Error::NoSearchResult.into())
    }
}
