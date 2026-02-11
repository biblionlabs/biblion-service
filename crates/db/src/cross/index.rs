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
pub struct IndexedCrossReference {
    pub source_book: String,
    pub source_book_name: String,
    pub source_chapter: i32,
    pub source_verse: i32,
    pub target_book: String,
    pub target_book_name: String,
    pub target_chapter: i32,
    pub target_verse: i32,
}

pub struct CrossRefIndex {
    index: Index,
    is_new: bool,
    reader: IndexReader,
    schema: Schema,
}

/// Calcula la distancia Levenshtein apropiada según la longitud del término.
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

impl CrossRefIndex {
    pub fn new(index_path: impl AsRef<Path>) -> tantivy::Result<Self> {
        let mut schema_builder = Schema::builder();
        let text_analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(RemoveLongFilter::limit(40))
            .filter(LowerCaser)
            .filter(AsciiFoldingFilter)
            .build();

        schema_builder.add_text_field("source_book", STRING | STORED);
        schema_builder.add_text_field("source_book_name", TEXT | STORED);
        schema_builder.add_i64_field("source_chapter", STORED | INDEXED | FAST);
        schema_builder.add_i64_field("source_verse", STORED | INDEXED | FAST);
        schema_builder.add_text_field("target_book", STRING | STORED);
        schema_builder.add_text_field("target_book_name", TEXT | STORED);
        schema_builder.add_i64_field("target_chapter", STORED | INDEXED | FAST);
        schema_builder.add_i64_field("target_verse", STORED | INDEXED | FAST);

        let mut is_new = true;
        let schema = schema_builder.build();

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

    pub fn index_cross_references_batch(
        &self,
        refs: Vec<IndexedCrossReference>,
    ) -> tantivy::Result<()> {
        if refs.is_empty() {
            return Ok(());
        }

        let mut writer = self.index.writer(50_000_000)?;

        let source_book = self.schema.get_field("source_book").unwrap();
        let source_book_name = self.schema.get_field("source_book_name").unwrap();
        let source_chapter = self.schema.get_field("source_chapter").unwrap();
        let source_verse = self.schema.get_field("source_verse").unwrap();
        let target_book = self.schema.get_field("target_book").unwrap();
        let target_book_name = self.schema.get_field("target_book_name").unwrap();
        let target_chapter = self.schema.get_field("target_chapter").unwrap();
        let target_verse = self.schema.get_field("target_verse").unwrap();

        for r in refs {
            let mut doc = TantivyDocument::default();
            doc.add_text(source_book, &r.source_book);
            doc.add_text(source_book_name, &r.source_book_name);
            doc.add_i64(source_chapter, r.source_chapter as i64);
            doc.add_i64(source_verse, r.source_verse as i64);
            doc.add_text(target_book, &r.target_book);
            doc.add_text(target_book_name, &r.target_book_name);
            doc.add_i64(target_chapter, r.target_chapter as i64);
            doc.add_i64(target_verse, r.target_verse as i64);

            writer.add_document(doc)?;
        }

        writer.commit()?;
        Ok(())
    }

    /// Construye una query para nombres de libros con tolerancia a errores.
    fn build_book_name_query(
        &self,
        book_str: &str,
        book_name_field: Field,
        book_id_field: Field,
    ) -> Box<dyn Query> {
        let normalized = normalize_query(book_str);
        let mut sub_queries: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        // Buscar por book_id exacto
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

            // Fuzzy para tolerar errores (mínimo distancia 1 para nombres)
            let distance = fuzzy_distance_for_term(token).max(1);
            let fuzzy = FuzzyTermQuery::new_prefix(term, distance, true);
            sub_queries.push((
                Occur::Should,
                Box::new(BoostQuery::new(Box::new(fuzzy), 2.0)),
            ));
        }

        Box::new(BooleanQuery::new(sub_queries))
    }

    /// Buscar referencias cruzadas por referencia bíblica.
    ///
    /// Busca en ambas direcciones: como source y como target.
    /// Ejemplos de query:
    /// - "GEN 1:1" -> referencias desde/hacia Génesis 1:1
    /// - "Genesis 1" -> referencias desde/hacia Génesis capítulo 1
    /// - "GEN 1:1-5" -> referencias desde/hacia Génesis 1:1 a 1:5
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        direction: SearchDirection,
    ) -> tantivy::Result<Vec<IndexedCrossReference>> {
        let searcher = self.reader.searcher();

        let reference_pattern = regex::Regex::new(
            r"(?i)^\s*(?P<book>.+?)\s+(?P<chapter>\d+)(?::(?P<verse_start>\d+)(?:-(?P<verse_end>\d+))?)?\s*$"
        ).unwrap();

        let final_query: Box<dyn Query> = if let Some(caps) = reference_pattern.captures(query) {
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

            let (book_field_id, book_name_field, chapter_field, verse_field) = match direction {
                SearchDirection::FromSource => (
                    "source_book",
                    "source_book_name",
                    "source_chapter",
                    "source_verse",
                ),
                SearchDirection::FromTarget => (
                    "target_book",
                    "target_book_name",
                    "target_chapter",
                    "target_verse",
                ),
                SearchDirection::Both => {
                    let source_results =
                        self.search(query, limit, SearchDirection::FromSource)?;
                    let target_results =
                        self.search(query, limit, SearchDirection::FromTarget)?;

                    let mut combined = source_results;
                    combined.extend(target_results);
                    combined.truncate(limit);
                    return Ok(combined);
                }
            };

            let book_id_field = self.schema.get_field(book_field_id).unwrap();
            let book_name_f = self.schema.get_field(book_name_field).unwrap();
            let chapter_f = self.schema.get_field(chapter_field).unwrap();
            let verse_f = self.schema.get_field(verse_field).unwrap();

            let mut sub_queries: Vec<(Occur, Box<dyn Query>)> = Vec::new();

            // Búsqueda por libro con tolerancia a errores
            let book_query = self.build_book_name_query(book_query_str, book_name_f, book_id_field);
            sub_queries.push((Occur::Must, book_query));

            // Capítulo exacto
            let chapter_term = Term::from_field_i64(chapter_f, chapter_num);
            sub_queries.push((
                Occur::Must,
                Box::new(TermQuery::new(chapter_term, Default::default())),
            ));

            // Verso(s)
            match (verse_start, verse_end) {
                (Some(start), Some(end)) => {
                    let range_query = RangeQuery::new(
                        Bound::Included(Term::from_field_i64(verse_f, start)),
                        Bound::Included(Term::from_field_i64(verse_f, end)),
                    );
                    sub_queries.push((Occur::Must, Box::new(range_query)));
                }
                (Some(start), None) => {
                    let verse_term = Term::from_field_i64(verse_f, start);
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
            // Búsqueda libre por nombre de libro con fuzzy
            let source_book_name = self.schema.get_field("source_book_name").unwrap();
            let target_book_name = self.schema.get_field("target_book_name").unwrap();
            let source_book_id = self.schema.get_field("source_book").unwrap();
            let target_book_id = self.schema.get_field("target_book").unwrap();

            let normalized = normalize_query(query);
            let mut sub_queries: Vec<(Occur, Box<dyn Query>)> = Vec::new();

            for token in normalized.split_whitespace() {
                if token.is_empty() {
                    continue;
                }

                let distance = fuzzy_distance_for_term(token).max(1);

                // Source book name
                let term_sn = Term::from_field_text(source_book_name, token);
                let exact_sn = TermQuery::new(term_sn.clone(), IndexRecordOption::WithFreqs);
                sub_queries.push((
                    Occur::Should,
                    Box::new(BoostQuery::new(Box::new(exact_sn), 5.0)),
                ));
                let fuzzy_sn = FuzzyTermQuery::new_prefix(term_sn, distance, true);
                sub_queries.push((
                    Occur::Should,
                    Box::new(BoostQuery::new(Box::new(fuzzy_sn), 1.0)),
                ));

                // Target book name
                let term_tn = Term::from_field_text(target_book_name, token);
                let exact_tn = TermQuery::new(term_tn.clone(), IndexRecordOption::WithFreqs);
                sub_queries.push((
                    Occur::Should,
                    Box::new(BoostQuery::new(Box::new(exact_tn), 5.0)),
                ));
                let fuzzy_tn = FuzzyTermQuery::new_prefix(term_tn, distance, true);
                sub_queries.push((
                    Occur::Should,
                    Box::new(BoostQuery::new(Box::new(fuzzy_tn), 1.0)),
                ));

                // Source book ID exacto
                let term_si = Term::from_field_text(source_book_id, token);
                let exact_si = TermQuery::new(term_si, IndexRecordOption::Basic);
                sub_queries.push((
                    Occur::Should,
                    Box::new(BoostQuery::new(Box::new(exact_si), 10.0)),
                ));

                // Target book ID exacto
                let term_ti = Term::from_field_text(target_book_id, token);
                let exact_ti = TermQuery::new(term_ti, IndexRecordOption::Basic);
                sub_queries.push((
                    Occur::Should,
                    Box::new(BoostQuery::new(Box::new(exact_ti), 10.0)),
                ));
            }

            if sub_queries.is_empty() {
                let parser = QueryParser::for_index(
                    &self.index,
                    vec![source_book_name, target_book_name, source_book_id, target_book_id],
                );
                parser.parse_query(&normalized)?
            } else {
                Box::new(BooleanQuery::new(sub_queries))
            }
        };

        let top_docs = searcher.search(&final_query, &TopDocs::with_limit(limit))?;

        let mut results = Vec::new();
        for (_score, doc_address) in top_docs {
            let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;

            let source_book = retrieved_doc
                .get_first(self.schema.get_field("source_book").unwrap())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let source_book_name = retrieved_doc
                .get_first(self.schema.get_field("source_book_name").unwrap())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let source_chapter = retrieved_doc
                .get_first(self.schema.get_field("source_chapter").unwrap())
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            let source_verse = retrieved_doc
                .get_first(self.schema.get_field("source_verse").unwrap())
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            let target_book = retrieved_doc
                .get_first(self.schema.get_field("target_book").unwrap())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let target_book_name = retrieved_doc
                .get_first(self.schema.get_field("target_book_name").unwrap())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let target_chapter = retrieved_doc
                .get_first(self.schema.get_field("target_chapter").unwrap())
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            let target_verse = retrieved_doc
                .get_first(self.schema.get_field("target_verse").unwrap())
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;

            results.push(IndexedCrossReference {
                source_book,
                source_book_name,
                source_chapter,
                source_verse,
                target_book,
                target_book_name,
                target_chapter,
                target_verse,
            });
        }

        results.sort_by(|a, b| {
            a.source_chapter
                .cmp(&b.source_chapter)
                .then_with(|| a.source_verse.cmp(&b.source_verse))
                .then_with(|| a.target_chapter.cmp(&b.target_chapter))
                .then_with(|| a.target_verse.cmp(&b.target_verse))
        });

        Ok(results)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchDirection {
    /// Buscar donde la referencia es el source (desde)
    FromSource,
    /// Buscar donde la referencia es el target (hacia)
    FromTarget,
    /// Buscar en ambas direcciones
    Both,
}
