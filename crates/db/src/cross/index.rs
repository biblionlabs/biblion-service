use crate::search_utils::{
    self, build_structured_reference_query, execute_search, extract_i64, extract_text,
    fuzzy_distance_for_term, normalize_query, parse_reference,
};
use crate::verse::FieldValue;
use serde::{Deserialize, Serialize};
use tantivy::query::{
    BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, Query, QueryParser, TermQuery,
};
use tantivy::schema::*;
use tantivy::{Index, IndexReader, IndexWriter, TantivyDocument};

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

impl CrossRefIndex {
    pub fn new(index_path: impl AsRef<std::path::Path>) -> tantivy::Result<Self> {
        let mut schema_builder = Schema::builder();
        schema_builder.add_text_field("source_book", STRING | STORED);
        schema_builder.add_text_field("source_book_name", TEXT | STORED);
        schema_builder.add_i64_field("source_chapter", STORED | INDEXED | FAST);
        schema_builder.add_i64_field("source_verse", STORED | INDEXED | FAST);
        schema_builder.add_text_field("target_book", STRING | STORED);
        schema_builder.add_text_field("target_book_name", TEXT | STORED);
        schema_builder.add_i64_field("target_chapter", STORED | INDEXED | FAST);
        schema_builder.add_i64_field("target_verse", STORED | INDEXED | FAST);
        let schema = schema_builder.build();

        let (index, is_new) = search_utils::open_or_create_index(&index_path, schema.clone())?;
        let reader = search_utils::create_reader(&index)?;

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

    pub fn has_documents(&self) -> bool {
        let searcher = self.reader.searcher();
        searcher.num_docs() > 0
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

    // ─── Build query (procesa regex/texto, NO ejecuta) ──────────────

    /// Construye la query para cross-references. Parsea referencia bíblica
    /// o genera query de texto libre. No ejecuta la búsqueda.
    pub fn build_query(
        &self,
        query: &str,
        direction: SearchDirection,
    ) -> tantivy::Result<Box<dyn Query>> {
        if let SearchDirection::Both = direction {
            return self.build_query(query, SearchDirection::FromSource);
        }

        if let Some(reference) = parse_reference(query) {
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
                SearchDirection::Both => unreachable!(),
            };

            Ok(build_structured_reference_query(
                &reference,
                self.schema.get_field(book_name_field).unwrap(),
                self.schema.get_field(book_field_id).unwrap(),
                self.schema.get_field(chapter_field).unwrap(),
                self.schema.get_field(verse_field).unwrap(),
            ))
        } else {
            self.build_free_text_query(query)
        }
    }

    /// Búsqueda libre por nombre de libro con fuzzy (busca en source y target).
    fn build_free_text_query(&self, query: &str) -> tantivy::Result<Box<dyn Query>> {
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
                vec![
                    source_book_name,
                    target_book_name,
                    source_book_id,
                    target_book_id,
                ],
            );
            Ok(parser.parse_query(&normalized)?)
        } else {
            Ok(Box::new(BooleanQuery::new(sub_queries)))
        }
    }

    // ─── Execute (recibe query ya construida, ejecuta y extrae) ─────

    /// Ejecuta una query ya construida y extrae los resultados.
    /// - limit Some(n): usa TopDocs (rápido, top N)
    /// - limit None: usa DocSetCollector (todos los docs que coinciden)
    pub fn execute(
        &self,
        query: &dyn Query,
        limit: Option<usize>,
    ) -> tantivy::Result<Vec<IndexedCrossReference>> {
        let searcher = self.reader.searcher();
        let doc_addresses = execute_search(&searcher, query, limit)?;

        let mut results = Vec::with_capacity(doc_addresses.len());
        for addr in doc_addresses {
            let doc: TantivyDocument = searcher.doc(addr)?;
            results.push(self.doc_to_cross_ref(&doc));
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

    // ─── Search (build + execute juntos, para conveniencia) ─────────

    pub fn search(
        &self,
        query: &str,
        limit: usize,
        direction: SearchDirection,
    ) -> tantivy::Result<Vec<IndexedCrossReference>> {
        if let SearchDirection::Both = direction {
            let source_results = self.search(query, limit, SearchDirection::FromSource)?;
            let target_results = self.search(query, limit, SearchDirection::FromTarget)?;
            let mut combined = source_results;
            combined.extend(target_results);
            combined.truncate(limit);
            return Ok(combined);
        }

        let built_query = self.build_query(query, direction)?;
        self.execute(built_query.as_ref(), Some(limit))
    }

    // ─── Direct field lookup (sin regex, sin fuzzy) ─────────────────

    /// Busca cross-references directamente por campos exactos.
    /// Bypasses regex parsing y fuzzy matching completamente.
    pub fn lookup_by_fields(
        &self,
        fields: Vec<(&str, FieldValue)>,
    ) -> tantivy::Result<Vec<IndexedCrossReference>> {
        let terms: Vec<(Field, Term)> = fields
            .into_iter()
            .map(|(name, val)| {
                let field = self.schema.get_field(name).unwrap();
                let term = match val {
                    FieldValue::Text(s) => Term::from_field_text(field, &s),
                    FieldValue::I64(n) => Term::from_field_i64(field, n),
                };
                (field, term)
            })
            .collect();

        let query = search_utils::build_field_query(terms);
        self.execute(query.as_ref(), None)
    }

    // ─── Doc extraction ─────────────────────────────────────────────

    fn doc_to_cross_ref(&self, doc: &TantivyDocument) -> IndexedCrossReference {
        IndexedCrossReference {
            source_book: extract_text(doc, &self.schema, "source_book"),
            source_book_name: extract_text(doc, &self.schema, "source_book_name"),
            source_chapter: extract_i64(doc, &self.schema, "source_chapter"),
            source_verse: extract_i64(doc, &self.schema, "source_verse"),
            target_book: extract_text(doc, &self.schema, "target_book"),
            target_book_name: extract_text(doc, &self.schema, "target_book_name"),
            target_chapter: extract_i64(doc, &self.schema, "target_chapter"),
            target_verse: extract_i64(doc, &self.schema, "target_verse"),
        }
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
