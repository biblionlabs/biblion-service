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

    /// Buscar referencias cruzadas por referencia bíblica.
    ///
    /// Busca en ambas direcciones: como source y como target.
    /// Ejemplos de query:
    /// - "GEN 1:1" → referencias desde/hacia Génesis 1:1
    /// - "Genesis 1" → referencias desde/hacia Génesis capítulo 1
    /// - "GEN 1:1-5" → referencias desde/hacia Génesis 1:1 a 1:5
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
                    // Buscar en ambas direcciones y combinar resultados
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

            // Búsqueda por libro (nombre o ID)
            let book_parser =
                QueryParser::for_index(&self.index, vec![book_name_f, book_id_field]);
            if let Ok(book_query) = book_parser.parse_query(book_query_str) {
                sub_queries.push((Occur::Must, book_query));
            }

            // Búsqueda por capítulo
            let chapter_term = Term::from_field_i64(chapter_f, chapter_num);
            sub_queries.push((
                Occur::Must,
                Box::new(TermQuery::new(chapter_term, Default::default())),
            ));

            // Búsqueda por verso(s)
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
            // Búsqueda libre por nombre de libro en ambos campos
            let source_book_name = self.schema.get_field("source_book_name").unwrap();
            let target_book_name = self.schema.get_field("target_book_name").unwrap();
            let source_book_id = self.schema.get_field("source_book").unwrap();
            let target_book_id = self.schema.get_field("target_book").unwrap();

            let parser = QueryParser::for_index(
                &self.index,
                vec![source_book_name, target_book_name, source_book_id, target_book_id],
            );
            parser.parse_query(query)?
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
