use serde::{Deserialize, Serialize};
use std::ops::Bound;
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, RangeQuery, TermQuery};
use tantivy::tokenizer::{
    AsciiFoldingFilter, LowerCaser, RemoveLongFilter, SimpleTokenizer, TextAnalyzer,
};
use tantivy::{Index, IndexReader, ReloadPolicy, TantivyDocument};
use tantivy::{IndexWriter, schema::*};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedVerse {
    pub bible_id: String,
    pub bible_name: String,
    pub book_id: String,
    pub book_name: String,
    pub chapter: i32,
    pub verse: i32,
    pub text: String,
}

pub struct VerseIndex {
    index: Index,
    is_new: bool,
    reader: IndexReader,
    schema: Schema,
}

impl VerseIndex {
    pub fn new(index_path: impl AsRef<Path>) -> tantivy::Result<Self> {
        let mut schema_builder = Schema::builder();
        let text_analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(RemoveLongFilter::limit(40))
            .filter(LowerCaser)
            .filter(AsciiFoldingFilter)
            .build();

        // Definir campos del índice
        schema_builder.add_text_field("bible_id", STRING | STORED);
        schema_builder.add_text_field("bible_name", TEXT | STORED);
        schema_builder.add_text_field("book_id", STRING | STORED);
        schema_builder.add_text_field("book_name", TEXT | STORED);
        schema_builder.add_i64_field("chapter", STORED | INDEXED | FAST);
        schema_builder.add_i64_field("verse", STORED | INDEXED | FAST);
        schema_builder.add_text_field("text", TEXT | STORED);

        let mut is_new = true;
        let schema = schema_builder.build();

        // Crear o abrir índice
        std::fs::create_dir_all(&index_path)?;
        let index = Index::create_in_dir(&index_path, schema.clone()).or_else(|_| {
            is_new = false;
            Index::open_in_dir(&index_path)
        })?;

        index.tokenizers().register("custom_text", text_analyzer);

        let mut writer: IndexWriter = index.writer(50_000_000)?;
        writer.commit()?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        Ok(Self {
            index,
            reader,
            schema,
            is_new,
        })
    }

    pub fn is_new(&self) -> bool {
        self.is_new
    }

    pub fn clear(&self) -> crate::Result<()> {
        let mut writer: IndexWriter = self.index.writer(500_000_000)?;
        writer.delete_all_documents()?;
        writer.commit()?;
        Ok(())
    }

    pub fn index_verses_batch(&self, verses: Vec<IndexedVerse>) -> tantivy::Result<()> {
        if verses.is_empty() {
            return Ok(());
        }

        let mut writer = self.index.writer(50_000_000)?;

        let bible_id = self.schema.get_field("bible_id").unwrap();
        let bible_name = self.schema.get_field("bible_name").unwrap();
        let book_id = self.schema.get_field("book_id").unwrap();
        let book_name = self.schema.get_field("book_name").unwrap();
        let chapter = self.schema.get_field("chapter").unwrap();
        let verse_field = self.schema.get_field("verse").unwrap();
        let text = self.schema.get_field("text").unwrap();

        for verse in verses {
            let mut doc = TantivyDocument::default();
            doc.add_text(bible_id, &verse.bible_id);
            doc.add_text(bible_name, &verse.bible_name);
            doc.add_text(book_id, &verse.book_id);
            doc.add_text(book_name, &verse.book_name);
            doc.add_i64(chapter, verse.chapter as i64);
            doc.add_i64(verse_field, verse.verse as i64);
            doc.add_text(text, &verse.text);

            writer.add_document(doc)?;
        }

        writer.commit()?;
        Ok(())
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
        fuzzy: bool,
    ) -> tantivy::Result<Vec<IndexedVerse>> {
        let searcher = self.reader.searcher();

        // Obtener todos los campos para búsqueda
        let text_field = self.schema.get_field("text").unwrap();
        let bible_name_field = self.schema.get_field("bible_name").unwrap();
        let book_name_field = self.schema.get_field("book_name").unwrap();
        let book_id_field = self.schema.get_field("book_id").unwrap();
        let chapter_field = self.schema.get_field("chapter").unwrap();
        let verse_field = self.schema.get_field("verse").unwrap();

        // Regex mejorado para capturar rangos de versos
        // Ejemplos que captura:
        // - "Genesis 1" → libro + capítulo
        // - "Genesis 1:20" → libro + capítulo + verso
        // - "Genesis 1:20-25" → libro + capítulo + rango de versos
        // - "Juan 3:16-18" → libro + capítulo + rango de versos
        let reference_pattern = regex::Regex::new(
            r"(?i)^\s*(?P<book>.+?)\s+(?P<chapter>\d+)(?::(?P<verse_start>\d+)(?:-(?P<verse_end>\d+))?)?\s*$"
        ).unwrap();

        let final_query: Box<dyn Query> = if let Some(caps) = reference_pattern.captures(query) {
            // ============================================
            // BÚSQUEDA ESTRUCTURADA (con referencia bíblica)
            // ============================================
            let book_query_str = caps.name("book").unwrap().as_str().trim();
            let chapter_num = caps
                .name("chapter")
                .unwrap()
                .as_str()
                .parse::<i64>()
                .unwrap_or(0);
            let verse_start = caps
                .name("verse_start")
                .map(|v| v.as_str().parse::<i64>().unwrap_or(0));
            let verse_end = caps
                .name("verse_end")
                .map(|v| v.as_str().parse::<i64>().unwrap_or(0));

            let mut sub_queries: Vec<(Occur, Box<dyn Query>)> = Vec::new();

            // 1. Búsqueda en nombre del libro (fuzzy o exacta)
            if fuzzy {
                let book_parser =
                    QueryParser::for_index(&self.index, vec![book_name_field, book_id_field]);
                if let Ok(book_query) = book_parser.parse_query(book_query_str) {
                    sub_queries.push((Occur::Must, book_query));
                }
            } else {
                // Búsqueda exacta: debe coincidir con nombre O id del libro
                let mut book_sub: Vec<(Occur, Box<dyn Query>)> = Vec::new();

                let book_name_term = Term::from_field_text(book_name_field, book_query_str);
                book_sub.push((
                    Occur::Should,
                    Box::new(TermQuery::new(book_name_term, Default::default())),
                ));

                let book_id_term = Term::from_field_text(book_id_field, book_query_str);
                book_sub.push((
                    Occur::Should,
                    Box::new(TermQuery::new(book_id_term, Default::default())),
                ));

                sub_queries.push((Occur::Must, Box::new(BooleanQuery::new(book_sub))));
            }

            // 2. Búsqueda por capítulo (exacta, siempre requerida)
            let chapter_term = Term::from_field_i64(chapter_field, chapter_num);
            sub_queries.push((
                Occur::Must,
                Box::new(TermQuery::new(chapter_term, Default::default())),
            ));

            // 3. Búsqueda por verso(s)
            match (verse_start, verse_end) {
                // Caso 1: Rango de versos (ej: "Genesis 1:20-25")
                (Some(start), Some(end)) => {
                    let range_query = RangeQuery::new(
                        Bound::Included(Term::from_field_i64(verse_field, start)),
                        Bound::Included(Term::from_field_i64(verse_field, end)),
                    );
                    sub_queries.push((Occur::Must, Box::new(range_query)));
                }
                // Caso 2: Verso único (ej: "Genesis 1:20")
                (Some(start), None) => {
                    let verse_term = Term::from_field_i64(verse_field, start);
                    sub_queries.push((
                        Occur::Must,
                        Box::new(TermQuery::new(verse_term, Default::default())),
                    ));
                }
                // Caso 3: Solo capítulo (ej: "Genesis 1") - todos los versos del capítulo
                (None, None) => {
                    // No agregamos filtro de verso, devolverá todos los versos del capítulo
                }
                // Caso 4: Inválido (verso_end sin verso_start) - ignorar
                (None, Some(_)) => {}
            }

            Box::new(BooleanQuery::new(sub_queries))
        } else {
            // ============================================
            // BÚSQUEDA DE CONTENIDO (texto libre)
            // ============================================
            let mut sub_queries: Vec<(Occur, Box<dyn Query>)> = Vec::new();

            if fuzzy {
                // Buscar en texto de versos con fuzzy
                let text_parser = QueryParser::for_index(&self.index, vec![text_field]);
                if let Ok(text_query) = text_parser.parse_query(query) {
                    sub_queries.push((Occur::Should, text_query));
                }

                // Buscar en nombres de biblia con fuzzy
                let bible_parser = QueryParser::for_index(&self.index, vec![bible_name_field]);
                if let Ok(bible_query) = bible_parser.parse_query(query) {
                    sub_queries.push((Occur::Should, bible_query));
                }

                // Buscar en nombres de libro con fuzzy
                let book_parser =
                    QueryParser::for_index(&self.index, vec![book_name_field, book_id_field]);
                if let Ok(book_query) = book_parser.parse_query(query) {
                    sub_queries.push((Occur::Should, book_query));
                }
            } else {
                // Búsqueda exacta en todos los campos de texto
                let parser = QueryParser::for_index(
                    &self.index,
                    vec![text_field, bible_name_field, book_name_field, book_id_field],
                );
                if let Ok(parsed) = parser.parse_query(query) {
                    sub_queries.push((Occur::Should, parsed));
                }
            }

            if sub_queries.is_empty() {
                // Fallback: búsqueda simple en texto
                let parser = QueryParser::for_index(&self.index, vec![text_field]);
                parser.parse_query(query)?
            } else {
                Box::new(BooleanQuery::new(sub_queries))
            }
        };

        let top_docs = searcher.search(&final_query, &TopDocs::with_limit(limit))?;

        let mut results = Vec::new();
        for (_score, doc_address) in top_docs {
            let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;

            let bible_id = retrieved_doc
                .get_first(self.schema.get_field("bible_id").unwrap())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let bible_name = retrieved_doc
                .get_first(self.schema.get_field("bible_name").unwrap())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let book_id = retrieved_doc
                .get_first(self.schema.get_field("book_id").unwrap())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let book_name = retrieved_doc
                .get_first(self.schema.get_field("book_name").unwrap())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let chapter = retrieved_doc
                .get_first(self.schema.get_field("chapter").unwrap())
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            let verse = retrieved_doc
                .get_first(self.schema.get_field("verse").unwrap())
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            let text = retrieved_doc
                .get_first(self.schema.get_field("text").unwrap())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            results.push(IndexedVerse {
                bible_id,
                bible_name,
                book_id,
                book_name,
                chapter,
                verse,
                text,
            });
        }

        results.sort_by(|a, b| {
            a.chapter
                .cmp(&b.chapter)
                .then_with(|| a.verse.cmp(&b.verse))
        });

        Ok(results)
    }
}
