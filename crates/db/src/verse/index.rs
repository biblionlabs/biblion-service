use crate::search_utils::{
    self, build_structured_reference_query, build_text_query, execute_search, extract_i64,
    extract_text, parse_reference,
};
use crate::synonym::{SynonymMap, SynonymTokenFilter};
use serde::{Deserialize, Serialize};
use tantivy::query::Query;
use tantivy::schema::*;
use tantivy::tokenizer::{
    AsciiFoldingFilter, LowerCaser, RemoveLongFilter, SimpleTokenizer, TextAnalyzer,
};
use tantivy::{Index, IndexReader, IndexWriter, TantivyDocument};

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

/// Tipo de valor para lookup por fields directos.
pub enum FieldValue {
    Text(String),
    I64(i64),
}

impl VerseIndex {
    pub fn new(index_path: impl AsRef<std::path::Path>) -> tantivy::Result<Self> {
        let mut schema_builder = Schema::builder();
        schema_builder.add_text_field("bible_id", STRING | STORED);
        schema_builder.add_text_field("bible_name", TEXT | STORED);
        schema_builder.add_text_field("book_id", STRING | STORED);
        schema_builder.add_text_field("book_name", TEXT | STORED);
        schema_builder.add_i64_field("chapter", STORED | INDEXED | FAST);
        schema_builder.add_i64_field("verse", STORED | INDEXED | FAST);
        schema_builder.add_text_field("text", TEXT | STORED);
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

    /// Re-registers the "default" tokenizer with synonym expansion.
    /// Must be called before indexing so that indexed tokens include synonyms.
    pub fn load_synonyms(&self, map: SynonymMap) {
        let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(RemoveLongFilter::limit(40))
            .filter(LowerCaser)
            .filter(AsciiFoldingFilter)
            .filter(SynonymTokenFilter::new(map))
            .build();
        self.index.tokenizers().register("default", analyzer);
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

    // ─── Build query (procesa regex/texto, NO ejecuta) ──────────────

    /// Construye la query a partir de un string. Parsea la referencia bíblica
    /// o genera query de texto libre. No ejecuta la búsqueda.
    pub fn build_query(&self, query: &str, fuzzy: bool) -> tantivy::Result<Box<dyn Query>> {
        let book_name_field = self.schema.get_field("book_name").unwrap();
        let book_id_field = self.schema.get_field("book_id").unwrap();
        let chapter_field = self.schema.get_field("chapter").unwrap();
        let verse_field = self.schema.get_field("verse").unwrap();
        let text_field = self.schema.get_field("text").unwrap();

        if let Some(reference) = parse_reference(query) {
            Ok(build_structured_reference_query(
                &reference,
                book_name_field,
                book_id_field,
                chapter_field,
                verse_field,
            ))
        } else {
            build_text_query(&self.index, query, text_field, fuzzy)
        }
    }

    /// Ejecuta una query ya construida y extrae los resultados.
    /// - limit Some(n): usa TopDocs (rápido, top N)
    /// - limit None: usa DocSetCollector (todos los docs que coinciden)
    pub fn execute(
        &self,
        query: &dyn Query,
        limit: Option<usize>,
    ) -> tantivy::Result<Vec<IndexedVerse>> {
        let searcher = self.reader.searcher();
        let doc_addresses = execute_search(&searcher, query, limit)?;

        let mut results = Vec::with_capacity(doc_addresses.len());
        for addr in doc_addresses {
            let doc: TantivyDocument = searcher.doc(addr)?;
            results.push(self.doc_to_verse(&doc));
        }

        results.sort_by(|a, b| {
            a.chapter
                .cmp(&b.chapter)
                .then_with(|| a.verse.cmp(&b.verse))
        });

        Ok(results)
    }

    // ─── Search (build + execute juntos, para conveniencia) ─────────

    pub fn search(
        &self,
        query: &str,
        limit: Option<usize>,
        fuzzy: bool,
    ) -> tantivy::Result<Vec<IndexedVerse>> {
        let built_query = self.build_query(query, fuzzy)?;
        self.execute(built_query.as_ref(), limit)
    }

    // ─── Direct field lookup (sin regex, sin fuzzy) ─────────────────

    /// Busca versículos directamente por campos exactos.
    /// Bypasses regex parsing y fuzzy matching completamente.
    pub fn lookup_by_fields(
        &self,
        fields: Vec<(&str, FieldValue)>,
    ) -> tantivy::Result<Vec<IndexedVerse>> {
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

    fn doc_to_verse(&self, doc: &TantivyDocument) -> IndexedVerse {
        IndexedVerse {
            bible_id: extract_text(doc, &self.schema, "bible_id"),
            bible_name: extract_text(doc, &self.schema, "bible_name"),
            book_id: extract_text(doc, &self.schema, "book_id"),
            book_name: extract_text(doc, &self.schema, "book_name"),
            chapter: extract_i64(doc, &self.schema, "chapter"),
            verse: extract_i64(doc, &self.schema, "verse"),
            text: extract_text(doc, &self.schema, "text"),
        }
    }
}
