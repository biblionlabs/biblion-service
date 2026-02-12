use std::collections::HashMap;
use std::sync::Arc;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{CrossRefIndex, Result, Sqlite};

/// Texto completo de un versículo referenciado por una referencia cruzada.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossReferenceVerse {
    pub book_id: String,
    pub book_name: String,
    pub chapter: i32,
    pub verse: i32,
    pub text: String,
}

/// Versículo dentro del capítulo con indicador de resaltado y sus referencias cruzadas resueltas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterVerse {
    pub verse_number: i32,
    pub text: String,
    /// `true` si este versículo coincide con la búsqueda (para resaltado).
    pub highlighted: bool,
    /// Versículos completos de las referencias cruzadas de este versículo.
    pub cross_references: Vec<CrossReferenceVerse>,
}

/// Encabezado de sección dentro del capítulo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterHeader {
    pub text: String,
}

/// Resultado completo de una búsqueda con contexto de capítulo.
///
/// Contiene todos los versículos del capítulo, con los versículos buscados
/// marcados como `highlighted`, los encabezados del capítulo, y las
/// referencias cruzadas resueltas (con texto) para cada versículo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterSearchResult {
    pub bible_id: String,
    pub bible_name: String,
    pub book_id: String,
    pub book_name: String,
    pub chapter: i32,
    pub headers: Vec<ChapterHeader>,
    pub verses: Vec<ChapterVerse>,
}

impl Sqlite {
    /// Busca una referencia bíblica y devuelve el capítulo completo con:
    /// - Todos los versículos del capítulo
    /// - Los versículos que coinciden con la búsqueda marcados como `highlighted`
    /// - Las referencias cruzadas resueltas (con texto del versículo destino) para cada versículo
    /// - Los encabezados del capítulo
    ///
    /// # Ejemplo
    /// ```ignore
    /// let result = db.search_full_chapter("mateo 3:7", None)?;
    /// // result.verses[6].highlighted == true  (versículo 7)
    /// // result.verses[6].cross_references contiene los versículos referenciados
    /// ```
    pub fn search_full_chapter(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ChapterSearchResult>> {
        let verse_index = self.verse_index();
        let cross_index = self.cross_ref_index();

        // 1. Buscar versículos que coinciden con la query
        let idx = verse_index
            .as_ref()
            .as_ref()
            .ok_or_else(|| crate::Error::Other("Verse index not available".into()))?;

        let search_results = idx
            .search(query, limit.unwrap_or(50), true)
            .map_err(crate::Error::Tantivy)?;

        if search_results.is_empty() {
            return Err(crate::Error::NoSearchResult);
        }

        // Agrupar resultados por (bible_id, book_id, chapter)
        let mut groups: HashMap<(String, String, String, String, i32), Vec<i32>> = HashMap::new();
        for r in &search_results {
            let key = (
                r.bible_id.clone(),
                r.bible_name.clone(),
                r.book_id.clone(),
                r.book_name.clone(),
                r.chapter,
            );
            groups.entry(key).or_default().push(r.verse);
        }

        let conn = self.connection();
        let mut results = Vec::new();

        for ((bible_id, bible_name, book_id, book_name, chapter), highlighted_verses) in groups {
            // 2. Obtener todos los versículos del capítulo desde SQLite
            let chapter_verses = {
                let guard = conn.lock().unwrap();
                let mut stmt = guard.prepare(
                    "SELECT id, book_id, chapter_number, verse_number, text \
                     FROM verses \
                     WHERE book_id = ?1 AND chapter_number = ?2 \
                     ORDER BY verse_number ASC",
                )?;

                let rows = stmt.query_map(params![book_id, chapter], |row| {
                    Ok((
                        row.get::<_, i32>("verse_number")?,
                        row.get::<_, String>("text")?,
                    ))
                })?;

                rows.collect::<rusqlite::Result<Vec<(i32, String)>>>()?
            };

            // 3. Obtener encabezados del capítulo
            let headers = {
                let guard = conn.lock().unwrap();
                let mut stmt = guard.prepare(
                    "SELECT text FROM headers \
                     WHERE book_id = ?1 AND chapter_number = ?2",
                )?;

                let rows = stmt.query_map(params![book_id, chapter], |row| {
                    Ok(ChapterHeader {
                        text: row.get::<_, String>("text")?,
                    })
                })?;

                rows.collect::<rusqlite::Result<Vec<ChapterHeader>>>()?
            };

            // 4. Obtener referencias cruzadas para cada versículo del capítulo
            let cross_refs_by_verse = self.get_chapter_cross_references(
                &cross_index,
                &book_id,
                chapter,
                &chapter_verses,
            )?;

            // 5. Construir el resultado
            let verses: Vec<ChapterVerse> = chapter_verses
                .into_iter()
                .map(|(verse_num, text)| {
                    let is_highlighted = highlighted_verses.contains(&verse_num);
                    let cross_references = cross_refs_by_verse
                        .get(&verse_num)
                        .cloned()
                        .unwrap_or_default();

                    ChapterVerse {
                        verse_number: verse_num,
                        text,
                        highlighted: is_highlighted,
                        cross_references,
                    }
                })
                .collect();

            results.push(ChapterSearchResult {
                bible_id,
                bible_name,
                book_id,
                book_name,
                chapter,
                headers,
                verses,
            });
        }

        // Ordenar por capítulo
        results.sort_by_key(|r| r.chapter);

        Ok(results)
    }

    /// Obtiene las referencias cruzadas para cada versículo de un capítulo,
    /// resolviendo el texto de cada versículo destino desde SQLite.
    fn get_chapter_cross_references(
        &self,
        _cross_index: &Arc<Option<CrossRefIndex>>,
        book_id: &str,
        chapter: i32,
        _chapter_verses: &[(i32, String)],
    ) -> Result<HashMap<i32, Vec<CrossReferenceVerse>>> {
        let conn = self.connection();
        let mut result: HashMap<i32, Vec<CrossReferenceVerse>> = HashMap::new();

        // Obtener todas las cross-references del capítulo de una sola vez desde SQLite
        let cross_refs = {
            let guard = conn.lock().unwrap();
            let mut stmt = guard.prepare(
                "SELECT source_verse, target_book, target_chapter, target_verse \
                 FROM cross_references \
                 WHERE source_book = ?1 AND source_chapter = ?2 \
                 ORDER BY source_verse ASC",
            )?;

            let rows = stmt.query_map(params![book_id, chapter], |row| {
                Ok((
                    row.get::<_, i32>("source_verse")?,
                    row.get::<_, String>("target_book")?,
                    row.get::<_, i32>("target_chapter")?,
                    row.get::<_, i32>("target_verse")?,
                ))
            })?;

            rows.collect::<rusqlite::Result<Vec<(i32, String, i32, i32)>>>()?
        };

        // Agrupar las referencias por versículo source
        let mut refs_by_verse: HashMap<i32, Vec<(String, i32, i32)>> = HashMap::new();
        for (source_verse, target_book, target_chapter, target_verse) in cross_refs {
            refs_by_verse
                .entry(source_verse)
                .or_default()
                .push((target_book, target_chapter, target_verse));
        }

        // Resolver el texto de cada versículo destino
        for (verse_num, targets) in refs_by_verse {
            let mut resolved = Vec::new();

            for (target_book, target_chapter, target_verse) in targets {
                // Buscar el texto del versículo destino
                let verse_text = {
                    let guard = conn.lock().unwrap();
                    let mut stmt = guard.prepare(
                        "SELECT text FROM verses \
                         WHERE book_id = ?1 AND chapter_number = ?2 AND verse_number = ?3 \
                         LIMIT 1",
                    )?;

                    stmt.query_row(
                        params![target_book, target_chapter, target_verse],
                        |row| row.get::<_, String>("text"),
                    )
                    .ok()
                };

                // Buscar el nombre del libro destino
                let target_book_name = {
                    let guard = conn.lock().unwrap();
                    let mut stmt = guard.prepare(
                        "SELECT name_normal FROM books WHERE id = ?1 LIMIT 1",
                    )?;

                    stmt.query_row(params![target_book], |row| {
                        row.get::<_, String>("name_normal")
                    })
                    .unwrap_or_else(|_| target_book.clone())
                };

                resolved.push(CrossReferenceVerse {
                    book_id: target_book,
                    book_name: target_book_name,
                    chapter: target_chapter,
                    verse: target_verse,
                    text: verse_text.unwrap_or_default(),
                });
            }

            result.insert(verse_num, resolved);
        }

        Ok(result)
    }
}
