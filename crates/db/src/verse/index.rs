use serde::{Deserialize, Serialize};
use std::ops::Bound;
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::{
    BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, Query, QueryParser, RangeQuery, TermQuery,
};
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

/// Calcula la distancia Levenshtein apropiada según la longitud del término.
/// - Palabras cortas (1-3 chars): distancia 0 (exacta, evitar ruido)
/// - Palabras medianas (4-6 chars): distancia 1
/// - Palabras largas (7+ chars): distancia 2
fn fuzzy_distance_for_term(term: &str) -> u8 {
    let len = term.chars().count();
    if len <= 3 {
        0
    } else if len <= 6 {
        1
    } else {
        2
    }
}

/// Normaliza texto para búsqueda: minúsculas + ascii folding
fn normalize_query(query: &str) -> String {
    deunicode::deunicode(&query.to_lowercase())
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

    /// Construye queries fuzzy por cada token del input.
    /// Combina una query exacta (boost alto) con una fuzzy (boost bajo)
    /// para que las coincidencias exactas aparezcan primero y las fuzzy
    /// solo complementen sin introducir ruido.
    fn build_fuzzy_text_query(
        &self,
        query_str: &str,
        field: Field,
    ) -> Vec<(Occur, Box<dyn Query>)> {
        let normalized = normalize_query(query_str);
        let tokens: Vec<&str> = normalized.split_whitespace().collect();
        let mut sub_queries: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        for token in &tokens {
            if token.is_empty() {
                continue;
            }

            let distance = fuzzy_distance_for_term(token);
            let term = Term::from_field_text(field, token);

            // Query exacta con boost alto - prioriza coincidencias exactas
            let exact_query = TermQuery::new(term.clone(), IndexRecordOption::WithFreqs);
            let boosted_exact = BoostQuery::new(Box::new(exact_query), 5.0);
            sub_queries.push((Occur::Should, Box::new(boosted_exact)));

            // Query fuzzy (solo si la distancia > 0, evitando ruido en palabras cortas)
            if distance > 0 {
                let fuzzy_query = FuzzyTermQuery::new_prefix(term, distance, true);
                let boosted_fuzzy = BoostQuery::new(Box::new(fuzzy_query), 1.0);
                sub_queries.push((Occur::Should, Box::new(boosted_fuzzy)));
            }
        }

        sub_queries
    }

    /// Construye una query para nombres de libros con tolerancia a errores.
    /// Los nombres de libros pueden escribirse de muchas formas
    /// (Génesis, Genesis, Gen, GEN) así que se usa fuzzy más permisivo.
    fn build_book_name_query(
        &self,
        book_str: &str,
        book_name_field: Field,
        book_id_field: Field,
    ) -> Box<dyn Query> {
        let normalized = normalize_query(book_str);
        let mut sub_queries: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        // Buscar por book_id exacto (ej: "GEN", "MAT")
        let id_term = Term::from_field_text(book_id_field, &normalized);
        let id_exact = TermQuery::new(id_term, IndexRecordOption::Basic);
        sub_queries.push((
            Occur::Should,
            Box::new(BoostQuery::new(Box::new(id_exact), 10.0)),
        ));

        // Buscar por book_id uppercase
        let id_upper = Term::from_field_text(book_id_field, &normalized.to_uppercase());
        let id_exact_upper = TermQuery::new(id_upper, IndexRecordOption::Basic);
        sub_queries.push((
            Occur::Should,
            Box::new(BoostQuery::new(Box::new(id_exact_upper), 10.0)),
        ));

        // Cada token del nombre del libro
        for token in normalized.split_whitespace() {
            if token.is_empty() {
                continue;
            }

            let term = Term::from_field_text(book_name_field, token);

            // Exacta con boost alto
            let exact = TermQuery::new(term.clone(), IndexRecordOption::WithFreqs);
            sub_queries.push((
                Occur::Should,
                Box::new(BoostQuery::new(Box::new(exact), 8.0)),
            ));

            // Fuzzy para tolerar errores en nombres de libros (mínimo distancia 1)
            let distance = fuzzy_distance_for_term(token).max(1);
            let fuzzy = FuzzyTermQuery::new_prefix(term, distance, true);
            sub_queries.push((
                Occur::Should,
                Box::new(BoostQuery::new(Box::new(fuzzy), 2.0)),
            ));
        }

        Box::new(BooleanQuery::new(sub_queries))
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
        fuzzy: bool,
    ) -> tantivy::Result<Vec<IndexedVerse>> {
        let searcher = self.reader.searcher();

        let text_field = self.schema.get_field("text").unwrap();
        let book_name_field = self.schema.get_field("book_name").unwrap();
        let book_id_field = self.schema.get_field("book_id").unwrap();
        let chapter_field = self.schema.get_field("chapter").unwrap();
        let verse_field = self.schema.get_field("verse").unwrap();

        // Regex para capturar referencias bíblicas
        let reference_pattern = regex::Regex::new(
            r"(?i)^\s*(?P<book>.+?)\s+(?P<chapter>\d+)(?::(?P<verse_start>\d+)(?:-(?P<verse_end>\d+))?)?\s*$"
        ).unwrap();

        let final_query: Box<dyn Query> = if let Some(caps) = reference_pattern.captures(query) {
            // ============================================
            // BÚSQUEDA ESTRUCTURADA (referencia bíblica)
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

            // 1. Nombre del libro con tolerancia a errores
            let book_query =
                self.build_book_name_query(book_query_str, book_name_field, book_id_field);
            sub_queries.push((Occur::Must, book_query));

            // 2. Capítulo exacto
            let chapter_term = Term::from_field_i64(chapter_field, chapter_num);
            sub_queries.push((
                Occur::Must,
                Box::new(TermQuery::new(chapter_term, Default::default())),
            ));

            // 3. Verso(s)
            match (verse_start, verse_end) {
                (Some(start), Some(end)) => {
                    let range_query = RangeQuery::new(
                        Bound::Included(Term::from_field_i64(verse_field, start)),
                        Bound::Included(Term::from_field_i64(verse_field, end)),
                    );
                    sub_queries.push((Occur::Must, Box::new(range_query)));
                }
                (Some(start), None) => {
                    let verse_term = Term::from_field_i64(verse_field, start);
                    sub_queries.push((
                        Occur::Must,
                        Box::new(TermQuery::new(verse_term, Default::default())),
                    ));
                }
                (None, None) => {}
                (None, Some(_)) => {}
            }

            Box::new(BooleanQuery::new(sub_queries))
        } else {
            // ============================================
            // BÚSQUEDA DE CONTENIDO (texto libre)
            // ============================================
            if fuzzy {
                // Búsqueda con FuzzyTermQuery real por cada token
                let text_queries = self.build_fuzzy_text_query(query, text_field);

                if text_queries.is_empty() {
                    // Fallback: QueryParser con fuzzy por campo
                    let mut parser = QueryParser::for_index(&self.index, vec![text_field]);
                    parser.set_field_fuzzy(text_field, true, 1, true);
                    parser.parse_query(&normalize_query(query))?
                } else {
                    Box::new(BooleanQuery::new(text_queries))
                }
            } else {
                // Búsqueda exacta con QueryParser estándar
                let parser = QueryParser::for_index(&self.index, vec![text_field]);
                parser.parse_query(&normalize_query(query))?
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
