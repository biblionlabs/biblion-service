use std::collections::HashSet;
use std::ops::Bound;
use std::path::Path;

use tantivy::collector::{DocSetCollector, TopDocs};
use tantivy::query::{
    BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, Query, QueryParser, RangeQuery, TermQuery,
};
use tantivy::tokenizer::{
    AsciiFoldingFilter, LowerCaser, RemoveLongFilter, SimpleTokenizer, TextAnalyzer,
};
use tantivy::{DocAddress, Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument};
use tantivy::schema::*;

// ─── Shared helpers ──────────────────────────────────────────────────

/// Calcula la distancia Levenshtein apropiada según la longitud del término.
pub fn fuzzy_distance_for_term(term: &str) -> u8 {
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
pub fn normalize_query(query: &str) -> String {
    deunicode::deunicode(&query.to_lowercase())
}

/// Crea el TextAnalyzer estándar para todos los índices.
pub fn create_text_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .build()
}

/// Abre o crea un índice Tantivy. Devuelve (Index, is_new).
pub fn open_or_create_index(
    index_path: impl AsRef<Path>,
    schema: Schema,
) -> tantivy::Result<(Index, bool)> {
    std::fs::create_dir_all(&index_path)?;
    let mut is_new = true;
    let index = Index::create_in_dir(&index_path, schema).or_else(|_| {
        is_new = false;
        Index::open_in_dir(&index_path)
    })?;
    index
        .tokenizers()
        .register("custom_text", create_text_analyzer());

    let mut writer: IndexWriter = index.writer(50_000_000)?;
    writer.commit()?;

    Ok((index, is_new))
}

/// Crea un IndexReader con reload on commit.
pub fn create_reader(index: &Index) -> tantivy::Result<IndexReader> {
    index
        .reader_builder()
        .reload_policy(ReloadPolicy::OnCommitWithDelay)
        .try_into()
}

// ─── Parsed reference ────────────────────────────────────────────────

pub struct ParsedReference {
    pub book: String,
    pub chapter: i64,
    pub verse_start: Option<i64>,
    pub verse_end: Option<i64>,
}

/// Parsea un query string como referencia bíblica (ej: "Genesis 1:1-5").
/// Retorna None si no coincide con el patrón.
pub fn parse_reference(query: &str) -> Option<ParsedReference> {
    let pattern = regex::Regex::new(
        r"(?i)^\s*(?P<book>.+?)\s+(?P<chapter>\d+)(?::(?P<verse_start>\d+)(?:-(?P<verse_end>\d+))?)?\s*$"
    ).unwrap();

    let caps = pattern.captures(query)?;
    Some(ParsedReference {
        book: caps.name("book").unwrap().as_str().trim().to_string(),
        chapter: caps.name("chapter").unwrap().as_str().parse().unwrap_or(0),
        verse_start: caps
            .name("verse_start")
            .map(|v| v.as_str().parse().unwrap_or(0)),
        verse_end: caps
            .name("verse_end")
            .map(|v| v.as_str().parse().unwrap_or(0)),
    })
}

// ─── Shared query builders ───────────────────────────────────────────

/// Construye una query para nombres de libros con tolerancia a errores.
pub fn build_book_name_query(
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

    for token in normalized.split_whitespace() {
        if token.is_empty() {
            continue;
        }

        let term = Term::from_field_text(book_name_field, token);

        let exact = TermQuery::new(term.clone(), IndexRecordOption::WithFreqs);
        sub_queries.push((
            Occur::Should,
            Box::new(BoostQuery::new(Box::new(exact), 8.0)),
        ));

        let distance = fuzzy_distance_for_term(token).max(1);
        let fuzzy = FuzzyTermQuery::new_prefix(term, distance, true);
        sub_queries.push((
            Occur::Should,
            Box::new(BoostQuery::new(Box::new(fuzzy), 2.0)),
        ));
    }

    Box::new(BooleanQuery::new(sub_queries))
}

/// Construye queries fuzzy por cada token del input.
pub fn build_fuzzy_text_query(
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

        let exact_query = TermQuery::new(term.clone(), IndexRecordOption::WithFreqs);
        let boosted_exact = BoostQuery::new(Box::new(exact_query), 5.0);
        sub_queries.push((Occur::Should, Box::new(boosted_exact)));

        if distance > 0 {
            let fuzzy_query = FuzzyTermQuery::new_prefix(term, distance, true);
            let boosted_fuzzy = BoostQuery::new(Box::new(fuzzy_query), 1.0);
            sub_queries.push((Occur::Should, Box::new(boosted_fuzzy)));
        }
    }

    sub_queries
}

/// Construye una query de referencia estructurada a partir de un ParsedReference
/// usando los fields indicados.
pub fn build_structured_reference_query(
    reference: &ParsedReference,
    book_name_field: Field,
    book_id_field: Field,
    chapter_field: Field,
    verse_field: Field,
) -> Box<dyn Query> {
    let mut sub_queries: Vec<(Occur, Box<dyn Query>)> = Vec::new();

    let book_query = build_book_name_query(&reference.book, book_name_field, book_id_field);
    sub_queries.push((Occur::Must, book_query));

    let chapter_term = Term::from_field_i64(chapter_field, reference.chapter);
    sub_queries.push((
        Occur::Must,
        Box::new(TermQuery::new(chapter_term, Default::default())),
    ));

    match (reference.verse_start, reference.verse_end) {
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
        _ => {}
    }

    Box::new(BooleanQuery::new(sub_queries))
}

/// Construye una query de texto libre con o sin fuzzy, usando el QueryParser como fallback.
pub fn build_text_query(
    index: &Index,
    query_str: &str,
    text_field: Field,
    fuzzy: bool,
) -> tantivy::Result<Box<dyn Query>> {
    if fuzzy {
        let text_queries = build_fuzzy_text_query(query_str, text_field);
        if text_queries.is_empty() {
            let mut parser = QueryParser::for_index(index, vec![text_field]);
            parser.set_field_fuzzy(text_field, true, 1, true);
            Ok(parser.parse_query(&normalize_query(query_str))?)
        } else {
            Ok(Box::new(BooleanQuery::new(text_queries)))
        }
    } else {
        let parser = QueryParser::for_index(index, vec![text_field]);
        Ok(parser.parse_query(&normalize_query(query_str))?)
    }
}

// ─── Execute search ──────────────────────────────────────────────────

/// Ejecuta una query en el searcher.
/// - Si limit es Some(n), usa TopDocs::with_limit (más rápido, limitado).
/// - Si limit es None, usa DocSetCollector (retorna TODOS los docs que coinciden).
pub fn execute_search(
    searcher: &tantivy::Searcher,
    query: &dyn Query,
    limit: Option<usize>,
) -> tantivy::Result<Vec<DocAddress>> {
    match limit {
        Some(limit @ 0..) => {
            let top_docs = searcher.search(query, &TopDocs::with_limit(limit))?;
            Ok(top_docs.into_iter().map(|(_score, addr)| addr).collect())
        }
        _ => {
            let doc_set: HashSet<DocAddress> = searcher.search(query, &DocSetCollector)?;
            Ok(doc_set.into_iter().collect())
        }
    }
}

// ─── Direct field query builder ──────────────────────────────────────

/// Construye una BooleanQuery con Must para buscar por campos exactos.
/// Cada tupla es (Field, Term) — se combinan con AND (Occur::Must).
pub fn build_field_query(terms: Vec<(Field, Term)>) -> Box<dyn Query> {
    let sub_queries: Vec<(Occur, Box<dyn Query>)> = terms
        .into_iter()
        .map(|(_field, term)| {
            (
                Occur::Must,
                Box::new(TermQuery::new(term, IndexRecordOption::Basic)) as Box<dyn Query>,
            )
        })
        .collect();
    Box::new(BooleanQuery::new(sub_queries))
}

// ─── Doc field extraction ────────────────────────────────────────────

pub fn extract_text(doc: &TantivyDocument, schema: &Schema, field_name: &str) -> String {
    doc.get_first(schema.get_field(field_name).unwrap())
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

pub fn extract_i64(doc: &TantivyDocument, schema: &Schema, field_name: &str) -> i32 {
    doc.get_first(schema.get_field(field_name).unwrap())
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32
}
