use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use service_db::{
    ChapterSearchResult, ChapterVerse, CrossRefIndex, CrossReferenceVerse,
    IndexedCrossReference, IndexedVerse, SearchDirection, SynonymMap, VerseIndex,
};

use crate::{BibleInstallStatus, Result};

use super::DbSink;

pub struct TantivySink {
    index: Arc<Option<VerseIndex>>,
    cross_index: Arc<Option<CrossRefIndex>>,
}

impl From<String> for TantivySink {
    fn from(db_path: String) -> Self {
        let base_path = PathBuf::from(&db_path);
        let index = Arc::new(VerseIndex::new(&base_path).ok());
        let cross_path = base_path.parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("cross_index");
        let cross_index = Arc::new(CrossRefIndex::new(cross_path).ok());
        Self { index, cross_index }
    }
}

impl From<PathBuf> for TantivySink {
    fn from(index_path: PathBuf) -> Self {
        let index = Arc::new(VerseIndex::new(&index_path).ok());
        let cross_path = index_path.parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("cross_index");
        let cross_index = Arc::new(CrossRefIndex::new(cross_path).ok());
        Self { index, cross_index }
    }
}

impl TantivySink {
    pub fn verse_index(&self) -> Arc<Option<VerseIndex>> {
        self.index.clone()
    }

    pub fn cross_ref_index(&self) -> Arc<Option<CrossRefIndex>> {
        self.cross_index.clone()
    }
}

impl DbSink for TantivySink {
    fn has_cross_references(&self, _expected_books: &[String]) -> Result<bool> {
        Ok(true)
    }

    fn has_languages(&self, _expected_langs: &[String]) -> Result<bool> {
        Ok(true)
    }

    fn get_bible_install_stats(
        &self,
        bible_id: &str,
        expected_books: &[(String, usize)],
    ) -> Result<BibleInstallStatus> {
        let maybe_index = self.index.as_ref();
        let Some(index) = maybe_index else {
            return Ok(BibleInstallStatus::NotInstalled);
        };

        if index.is_new() {
            return Ok(BibleInstallStatus::NotInstalled);
        }

        match index.search(bible_id, 1, false) {
            Ok(res) => {
                if res.is_empty() {
                    Ok(BibleInstallStatus::NotInstalled)
                } else {
                    Ok(BibleInstallStatus::Complete {
                        total_books: expected_books.len(),
                        total_verses: expected_books.iter().map(|(_, len)| *len).sum(),
                    })
                }
            }
            Err(_) => {
                Ok(BibleInstallStatus::NotInstalled)
            }
        }
    }

    fn clear_index(&self) -> Result<()> {
        if let Some(index) = self.index.as_ref() {
            index.clear()?;
        }
        Ok(())
    }

    fn index_bible_verses(&self, verses: Vec<IndexedVerse>) -> Result<()> {
        let Some(index) = self.index.as_ref() else {
            return Ok(());
        };

        index
            .index_verses_batch(verses)
            .map_err(service_db::Error::from)
            .map_err(crate::Error::from)?;

        Ok(())
    }

    fn clear_cross_ref_index(&self) -> Result<()> {
        if let Some(index) = self.cross_index.as_ref() {
            index.clear()?;
        }
        Ok(())
    }

    fn index_cross_references(&self, refs: Vec<IndexedCrossReference>) -> Result<()> {
        let Some(index) = self.cross_index.as_ref() else {
            return Ok(());
        };

        index
            .index_cross_references_batch(refs)
            .map_err(service_db::Error::from)
            .map_err(crate::Error::from)?;

        Ok(())
    }

    fn load_verse_synonyms(&self, map: SynonymMap) -> Result<()> {
        if let Some(index) = self.index.as_ref() {
            index.load_synonyms(map);
        }
        Ok(())
    }

    fn finalize(&self) -> Result<()> {
        Ok(())
    }

    fn search_full_chapter(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ChapterSearchResult>> {
        let idx = self
            .index
            .as_ref()
            .as_ref()
            .ok_or_else(|| crate::Error::Other("Verse index not available".into()))?;

        // 1. Buscar versículos que coinciden con la query
        let search_results = idx
            .search(query, limit.unwrap_or(50), true)
            .map_err(service_db::Error::from)?;

        if search_results.is_empty() {
            return Err(service_db::Error::NoSearchResult.into());
        }

        // Agrupar por (bible_id, bible_name, book_id, book_name, chapter)
        let mut groups: HashMap<(String, String, String, String, i32), Vec<i32>> = HashMap::new();
        for r in &search_results {
            groups
                .entry((
                    r.bible_id.clone(),
                    r.bible_name.clone(),
                    r.book_id.clone(),
                    r.book_name.clone(),
                    r.chapter,
                ))
                .or_default()
                .push(r.verse);
        }

        let mut results = Vec::new();

        for ((bible_id, bible_name, book_id, book_name, chapter), highlighted_verses) in groups {
            // 2. Obtener todos los versículos del capítulo via Tantivy
            //    Buscamos "{book_id} {chapter}" que el VerseIndex parsea como referencia estructurada
            let chapter_query = format!("{} {}", book_name, chapter);
            let chapter_verses = idx
                .search(&chapter_query, 200, false)
                .map_err(service_db::Error::from)?
                .into_iter()
                .filter(|v| v.book_id == book_id && v.chapter == chapter)
                .map(|v| (v.verse, v.text))
                .collect::<Vec<_>>();

            // 3. Obtener cross-references del capítulo via Tantivy CrossRefIndex
            let mut refs_by_verse: HashMap<i32, Vec<(String, String, i32, i32)>> = HashMap::new();
            if let Some(cross_idx) = self.cross_index.as_ref().as_ref() {
                let cross_query = format!("{} {}", book_name, chapter);
                if let Ok(cross_results) =
                    cross_idx.search(&cross_query, 500, SearchDirection::FromSource)
                {
                    for cr in cross_results {
                        if cr.source_book == book_id && cr.source_chapter == chapter {
                            refs_by_verse.entry(cr.source_verse).or_default().push((
                                cr.target_book,
                                cr.target_book_name,
                                cr.target_chapter,
                                cr.target_verse,
                            ));
                        }
                    }
                }
            }

            // 4. Resolver texto de versículos destino de cross-references via Tantivy
            let mut cross_refs_by_verse: HashMap<i32, Vec<CrossReferenceVerse>> = HashMap::new();
            for (verse_num, targets) in refs_by_verse {
                let mut resolved = Vec::new();
                for (target_book, target_book_name, target_chapter, target_verse) in targets {
                    let target_query = format!(
                        "{} {}:{}",
                        target_book_name, target_chapter, target_verse
                    );
                    let verse_text = idx
                        .search(&target_query, 1, false)
                        .ok()
                        .and_then(|r| r.into_iter().next())
                        .map(|v| v.text)
                        .unwrap_or_default();

                    resolved.push(CrossReferenceVerse {
                        book_id: target_book,
                        book_name: target_book_name,
                        chapter: target_chapter,
                        verse: target_verse,
                        text: verse_text,
                    });
                }
                cross_refs_by_verse.insert(verse_num, resolved);
            }

            // 5. Construir resultado (TantivySink no tiene headers almacenados)
            let verses: Vec<ChapterVerse> = chapter_verses
                .into_iter()
                .map(|(verse_num, text)| ChapterVerse {
                    verse_number: verse_num,
                    text,
                    highlighted: highlighted_verses.contains(&verse_num),
                    cross_references: cross_refs_by_verse
                        .remove(&verse_num)
                        .unwrap_or_default(),
                })
                .collect();

            results.push(ChapterSearchResult {
                bible_id,
                bible_name,
                book_id,
                book_name,
                chapter,
                headers: vec![],
                verses,
            });
        }

        results.sort_by_key(|r| r.chapter);
        Ok(results)
    }
}
