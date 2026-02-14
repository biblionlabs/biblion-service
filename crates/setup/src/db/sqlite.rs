use std::collections::HashMap;
use std::path::PathBuf;

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use service_db::{
    ChapterHeader, ChapterSearchResult, ChapterVerse, Connection, CrossReferenceVerse, Crud,
    DbBible, DbBook, DbCrossReference, DbHeader, DbLanguage, DbVerse, DbVerseNote,
    IndexedCrossReference, IndexedVerse, Sqlite, SynonymMap,
};

use crate::{BibleInstallStatus, CrossReference, Reference, Result};

use super::DbSink;

// Example DbSink implementation that uses the repo's `db` crate to insert rows.
// Adjust imports according to your crate layout.
pub struct SqliteDbSink {
    // here we could keep a connection pool, etc.
    // For demo purposes we'll open a new connection for each operation (not optimal).
    pub conn: Connection,
    pub db: Sqlite,
}

impl From<String> for SqliteDbSink {
    fn from(db_path: String) -> Self {
        let db = Sqlite::new(db_path).unwrap();
        let conn = db.connection();
        Self { conn, db }
    }
}

impl From<PathBuf> for SqliteDbSink {
    fn from(db_path: PathBuf) -> Self {
        let db = Sqlite::new(db_path).unwrap();
        let conn = db.connection();
        Self { conn, db }
    }
}

impl DbSink for SqliteDbSink {
    fn has_cross_references(&self, expected_books: &[String]) -> Result<bool> {
        let conn = self.conn.lock().unwrap();

        // Verificar que existan referencias para cada libro esperado
        for book_id in expected_books {
            let has_refs: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM cross_references WHERE source_book = ? LIMIT 1)",
                    [book_id],
                    |row| row.get(0),
                )
                .map_err(service_db::Error::from)?;

            if !has_refs {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn has_languages(&self, expected_langs: &[String]) -> Result<bool> {
        let conn = self.conn.lock().unwrap();

        for lang_id in expected_langs {
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM languages WHERE id = ? LIMIT 1)",
                    [lang_id],
                    |row| row.get(0),
                )
                .map_err(service_db::Error::from)?;

            if !exists {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn get_bible_install_stats(
        &self,
        bible_id: &str,
        expected_books: &[(String, usize)],
    ) -> Result<BibleInstallStatus> {
        let conn = self.conn.lock().unwrap();

        let bible_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM bibles WHERE id = ? LIMIT 1)",
                [bible_id],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !bible_exists {
            return Ok(BibleInstallStatus::NotInstalled);
        }

        let mut installed_books = 0usize;
        let mut installed_verses = 0usize;
        let total_books = expected_books.len();
        let total_verses: usize = expected_books.iter().map(|(_, count)| count).sum();

        for (book_id, expected_verse_count) in expected_books {
            // Verificar si el libro existe
            let book_exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM books WHERE id = ? AND bible_id = ? LIMIT 1)",
                    [book_id, bible_id],
                    |row| row.get(0),
                )
                .unwrap_or(false);

            if !book_exists {
                continue; // Libro no instalado
            }

            let verse_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM verses WHERE book_id = ?",
                    [book_id],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            if verse_count == *expected_verse_count as i64 {
                installed_books += 1;
            }

            installed_verses += verse_count as usize;
        }

        if installed_books == total_books && installed_verses == total_verses {
            Ok(BibleInstallStatus::Complete {
                total_books,
                total_verses,
            })
        } else if installed_books > 0 || installed_verses > 0 {
            Ok(BibleInstallStatus::Partial {
                installed_books,
                total_books,
                installed_verses,
                total_verses,
            })
        } else {
            Ok(BibleInstallStatus::NotInstalled)
        }
    }

    fn insert_cross_reference(&self, book: &str, data: &CrossReference) -> Result<()> {
        data.par_iter().for_each(|(source_chapter, cross)| {
            cross.par_iter().for_each(|(source_verse, cross)| {
                let mut targets = 0;
                let mut target_book = &Default::default();
                let mut target_chapter = Vec::new();
                let mut target_verse = Vec::new();
                for target in cross {
                    let mut iter = target.iter();
                    if let Some(Reference::String(book)) = iter.next() {
                        target_book = book;
                    }
                    let iter = iter.collect::<Vec<_>>();
                    let iter = iter.chunks(2);
                    targets = iter.clone().count();
                    for target in iter {
                        let mut iter = target.iter();
                        if let Some(Reference::Integer(chapter)) = iter.next() {
                            target_chapter.push(*chapter);
                        }
                        if let Some(Reference::Integer(verse)) = iter.next() {
                            target_verse.push(*verse);
                        }
                    }
                }

                for t in 0..targets {
                    let data = DbCrossReference {
                        id: 0,
                        source_book: book.to_string(),
                        source_chapter: source_chapter.parse().unwrap(),
                        source_verse: source_verse.parse().unwrap(),
                        target_book: target_book.clone(),
                        target_chapter: target_chapter[t],
                        target_verse: target_verse[t],
                    };
                    data.insert(self.conn.clone()).unwrap();
                }
            });
        });
        Ok(())
    }

    fn insert_language(
        &self,
        lang_id: &str,
        direction: &str,
        name_local: &str,
        name_english: &str,
    ) -> Result<()> {
        let data = DbLanguage {
            id: lang_id.to_string(),
            direction: direction.to_string(),
            name_local: name_local.to_string(),
            name_english: name_english.to_string(),
        };
        data.insert_with_id(self.conn.clone()).unwrap();
        Ok(())
    }

    fn insert_bible(
        &self,
        bible_id: &str,
        name_local: &str,
        name_english: &str,
        language_id: &str,
    ) -> Result<()> {
        let data = DbBible {
            id: bible_id.to_string(),
            name_local: name_local.to_string(),
            name_english: name_english.to_string(),
            language_id: language_id.to_string(),
        };
        data.insert_with_id(self.conn.clone()).unwrap();
        Ok(())
    }

    fn insert_header(
        &self,
        bible_id: &str,
        book_id: &str,
        chapter: usize,
        text: &str,
    ) -> Result<()> {
        let data = DbHeader {
            id: 0,
            bible_id: bible_id.to_string(),
            book_id: book_id.to_string(),
            chapter: chapter as _,
            text: text.to_string(),
        };
        data.insert(self.conn.clone()).unwrap();
        Ok(())
    }

    fn insert_book_meta(
        &self,
        bible_id: &str,
        book_id: &str,
        name_normal: &str,
        name_long: &str,
        name_abbrev: &str,
    ) -> Result<()> {
        let data = DbBook {
            id: book_id.to_string(),
            bible_id: bible_id.to_string(),
            name_normal: name_normal.to_string(),
            name_long: name_long.to_string(),
            name_abbrev: name_abbrev.to_string(),
        };
        data.insert_with_id(self.conn.clone()).unwrap();
        Ok(())
    }

    fn insert_verse(&self, book_id: &str, chapter: usize, verse: usize, text: &str) -> Result<()> {
        let data = DbVerse {
            id: 0,
            book_id: book_id.to_string(),
            chapter: chapter as _,
            verse: verse as _,
            text: text.to_string(),
        };
        data.insert(self.conn.clone()).unwrap();

        Ok(())
    }

    fn clear_index(&self) -> Result<()> {
        let binding = self.db.verse_index();
        let Some(index) = binding.as_ref() else {
            return Ok(());
        };

        index.clear()?;

        Ok(())
    }

    fn index_bible_verses(&self, verses: Vec<IndexedVerse>) -> Result<()> {
        let binding = self.db.verse_index();
        let Some(index) = binding.as_ref() else {
            return Ok(());
        };

        index
            .index_verses_batch(verses)
            .map_err(service_db::Error::from)
            .map_err(crate::Error::from)?;

        Ok(())
    }

    fn clear_cross_ref_index(&self) -> Result<()> {
        let binding = self.db.cross_ref_index();
        let Some(index) = binding.as_ref() else {
            return Ok(());
        };

        index.clear()?;

        Ok(())
    }

    fn index_cross_references(&self, refs: Vec<IndexedCrossReference>) -> Result<()> {
        let binding = self.db.cross_ref_index();
        let Some(index) = binding.as_ref() else {
            return Ok(());
        };

        index
            .index_cross_references_batch(refs)
            .map_err(service_db::Error::from)
            .map_err(crate::Error::from)?;

        Ok(())
    }

    fn insert_note(&self, book_id: &str, chapter: usize, verse: usize, text: &str) -> Result<()> {
        let data = DbVerseNote {
            id: 0,
            book_id: book_id.to_string(),
            chapter: chapter as _,
            verse: verse as _,
            text: text.to_string(),
        };
        data.insert(self.conn.clone()).unwrap();
        Ok(())
    }

    fn load_verse_synonyms(&self, map: SynonymMap) -> Result<()> {
        let binding = self.db.verse_index();
        if let Some(index) = binding.as_ref() {
            index.load_synonyms(map);
        }
        Ok(())
    }

    fn finalize(&self) -> Result<()> {
        // any final steps like rebuilding indices
        Ok(())
    }

    // fn get_crossreferences(
    //     &self,
    //     bible_id: &str,
    //     book_id: &str,
    //     chapter_idx: i32,
    //     verse_idx: i32,
    // ) -> Result<ChapterSearchResult> {
    //     let verse_index = self.db.verse_index();
    //     let idx = verse_index
    //         .as_ref()
    //         .as_ref()
    //         .ok_or_else(|| crate::Error::Other("Verse index not available".into()))?;
    //
    //     // 1. Buscar versículos que coinciden con la query via Tantivy
    //     let search_results = idx
    //         .search(query, limit.unwrap_or(50), true)
    //         .map_err(service_db::Error::from)?;
    //
    //     if search_results.is_empty() {
    //         return Err(service_db::Error::NoSearchResult.into());
    //     }
    //
    //     // Agrupar resultados por (bible_id, bible_name, book_id, book_name, chapter)
    //     let mut groups: HashMap<(String, String, String, String, i32), Vec<i32>> = HashMap::new();
    //     for r in &search_results {
    //         groups
    //             .entry((
    //                 r.bible_id.clone(),
    //                 r.bible_name.clone(),
    //                 r.book_id.clone(),
    //                 r.book_name.clone(),
    //                 r.chapter,
    //             ))
    //             .or_default()
    //             .push(r.verse);
    //     }
    //
    //     let conn = self.conn.clone();
    //     let mut results = Vec::new();
    //
    //     for ((bible_id, bible_name, book_id, book_name, chapter), highlighted_verses) in groups {
    //         // 2. Obtener todos los versículos del capítulo desde SQLite
    //         let chapter_verses = {
    //             let guard = conn.lock().unwrap();
    //             let mut stmt = guard
    //                 .prepare(
    //                     "SELECT verse_number, text FROM verses \
    //                      WHERE book_id = ?1 AND chapter_number = ?2 \
    //                      ORDER BY verse_number ASC",
    //                 )
    //                 .map_err(service_db::Error::from)?;
    //
    //             let rows = stmt
    //                 .query_map(
    //                     [&book_id as &dyn service_db::rusqlite::ToSql, &chapter],
    //                     |row| {
    //                         Ok((
    //                             row.get::<_, i32>("verse_number")?,
    //                             row.get::<_, String>("text")?,
    //                         ))
    //                     },
    //                 )
    //                 .map_err(service_db::Error::from)?;
    //
    //             rows.collect::<service_db::SqliteResult<Vec<(i32, String)>>>()
    //                 .map_err(service_db::Error::from)?
    //         };
    //
    //         // 3. Obtener encabezados del capítulo desde SQLite
    //         let headers = {
    //             let guard = conn.lock().unwrap();
    //             let mut stmt = guard
    //                 .prepare(
    //                     "SELECT text FROM headers \
    //                      WHERE book_id = ?1 AND chapter_number = ?2",
    //                 )
    //                 .map_err(service_db::Error::from)?;
    //
    //             let rows = stmt
    //                 .query_map(
    //                     [&book_id as &dyn service_db::rusqlite::ToSql, &chapter],
    //                     |row| {
    //                         Ok(ChapterHeader {
    //                             text: row.get::<_, String>("text")?,
    //                         })
    //                     },
    //                 )
    //                 .map_err(service_db::Error::from)?;
    //
    //             rows.collect::<service_db::SqliteResult<Vec<ChapterHeader>>>()
    //                 .map_err(service_db::Error::from)?
    //         };
    //
    //         // 4. Obtener cross-references del capítulo desde SQLite
    //         let cross_refs = {
    //             let guard = conn.lock().unwrap();
    //             let mut stmt = guard
    //                 .prepare(
    //                     "SELECT source_verse, target_book, target_chapter, target_verse \
    //                      FROM cross_references \
    //                      WHERE source_book = ?1 AND source_chapter = ?2 \
    //                      ORDER BY source_verse ASC",
    //                 )
    //                 .map_err(service_db::Error::from)?;
    //
    //             let rows = stmt
    //                 .query_map(
    //                     [&book_id as &dyn service_db::rusqlite::ToSql, &chapter],
    //                     |row| {
    //                         Ok((
    //                             row.get::<_, i32>("source_verse")?,
    //                             row.get::<_, String>("target_book")?,
    //                             row.get::<_, i32>("target_chapter")?,
    //                             row.get::<_, i32>("target_verse")?,
    //                         ))
    //                     },
    //                 )
    //                 .map_err(service_db::Error::from)?;
    //
    //             rows.collect::<service_db::SqliteResult<Vec<(i32, String, i32, i32)>>>()
    //                 .map_err(service_db::Error::from)?
    //         };
    //
    //         // Agrupar refs por versículo source
    //         let mut refs_by_verse: HashMap<i32, Vec<(String, i32, i32)>> = HashMap::new();
    //         for (source_verse, target_book, target_chapter, target_verse) in cross_refs {
    //             refs_by_verse.entry(source_verse).or_default().push((
    //                 target_book,
    //                 target_chapter,
    //                 target_verse,
    //             ));
    //         }
    //
    //         // 5. Resolver texto de cada versículo destino
    //         let mut cross_refs_by_verse: HashMap<i32, Vec<CrossReferenceVerse>> = HashMap::new();
    //         for (verse_num, targets) in refs_by_verse {
    //             let mut resolved = Vec::new();
    //             for (target_book, target_chapter, target_verse) in targets {
    //                 let guard = conn.lock().unwrap();
    //
    //                 let verse_text = guard
    //                     .query_row(
    //                         "SELECT text FROM verses \
    //                          WHERE book_id = ?1 AND chapter_number = ?2 AND verse_number = ?3 \
    //                          LIMIT 1",
    //                         [
    //                             &target_book as &dyn service_db::rusqlite::ToSql,
    //                             &target_chapter,
    //                             &target_verse,
    //                         ],
    //                         |row| row.get::<_, String>("text"),
    //                     )
    //                     .ok()
    //                     .unwrap_or_default();
    //
    //                 let target_book_name = guard
    //                     .query_row(
    //                         "SELECT name_normal FROM books WHERE id = ?1 LIMIT 1",
    //                         [&target_book],
    //                         |row| row.get::<_, String>("name_normal"),
    //                     )
    //                     .unwrap_or_else(|_| target_book.clone());
    //
    //                 resolved.push(CrossReferenceVerse {
    //                     book_id: target_book,
    //                     book_name: target_book_name,
    //                     chapter: target_chapter,
    //                     verse: target_verse,
    //                     text: verse_text,
    //                 });
    //             }
    //             cross_refs_by_verse.insert(verse_num, resolved);
    //         }
    //
    //         // 6. Construir resultado
    //         let verses: Vec<ChapterVerse> = chapter_verses
    //             .into_iter()
    //             .map(|(verse_num, text)| ChapterVerse {
    //                 verse_number: verse_num,
    //                 text,
    //                 highlighted: highlighted_verses.contains(&verse_num),
    //                 cross_references: cross_refs_by_verse.remove(&verse_num).unwrap_or_default(),
    //             })
    //             .collect();
    //
    //         results.push(ChapterSearchResult {
    //             bible_id,
    //             bible_name,
    //             book_id,
    //             book_name,
    //             chapter,
    //             headers,
    //             verses,
    //         });
    //     }
    //
    //     results.sort_by_key(|r| r.chapter);
    //     Ok(results)
    // }
}
