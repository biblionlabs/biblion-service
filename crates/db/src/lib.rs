use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection as SQLiteConnection, Params, Row, ToSql, params_from_iter};

mod bible;
mod book;
mod cross;
mod error;
mod header;
mod lang;
mod notes;
mod synonym;
mod verse;

pub use bible::*;
pub use book::*;
pub use cross::*;
pub use error::{DB as Error, Result};
pub use header::*;
pub use lang::*;
pub use notes::*;
pub use synonym::*;
pub use verse::*;

pub use rusqlite::{Error as SqliteError, Result as SqliteResult};

pub type Connection = Arc<Mutex<SQLiteConnection>>;

pub trait Queriable {
    const TABLE: &str;
    const FIELDS: &[&str];

    fn field_list() -> String {
        Self::FIELDS.join(", ")
    }
}

pub trait Migrate {
    fn migrate(&self) -> Result<()>;
}

pub trait FromRow: Sized {
    fn from_row(row: &Row) -> rusqlite::Result<Self>;
}

pub trait ToParams {
    fn as_params<'a>(&'a self) -> Vec<&'a dyn ToSql>;
}

pub trait Crud: Queriable + FromRow + Send + Sync + Sized {
    fn insert(&self, conn: Connection) -> Result<()> {
        let fields = Self::FIELDS.join(", ").replacen("id, ", "", 1);
        let placeholders = (1..=Self::FIELDS.len() - 1)
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");

        // Waiting for limbo support OR REPLACE || OR IGNORE
        // "INSERT OR REPLACE INTO {} ({fields}) VALUES ({placeholders})",
        let sql = format!(
            "INSERT INTO {} ({fields}) VALUES ({placeholders})",
            Self::TABLE
        );

        let res = conn
            .lock()
            .unwrap()
            .execute(&sql, params_from_iter(self.insert_params()));
        if let Err(e) = res {
            println!("SQL: {sql}");
            return Err(e.into());
        }
        Ok(())
    }

    fn insert_with_id(&self, conn: Connection) -> Result<()> {
        let fields = Self::FIELDS.join(", ");
        let placeholders = (1..=Self::FIELDS.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");

        // Waiting for limbo support OR REPLACE || OR IGNORE
        // "INSERT OR REPLACE INTO {} ({fields}) VALUES ({placeholders})",
        let sql = format!(
            "INSERT INTO {} ({fields}) VALUES ({placeholders})",
            Self::TABLE
        );

        let res = conn
            .lock()
            .unwrap()
            .execute(&sql, params_from_iter(self.params()));
        if let Err(e) = res {
            println!("SQL: {sql}");
            return Err(e.into());
        }
        Ok(())
    }

    fn update(&self, conn: Connection, key_field: &str, key_value: impl ToSql) -> Result<()> {
        let assignments = Self::FIELDS
            .iter()
            .map(|f| format!("{f} = ?"))
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            "UPDATE {} SET {} WHERE {} = ?",
            Self::TABLE,
            assignments,
            key_field
        );

        let mut params = self.params();
        let binding = key_value.to_sql()?;
        params.push(&binding);
        conn.lock()
            .unwrap()
            .execute(&sql, params_from_iter(params))?;
        Ok(())
    }

    fn delete(conn: Connection, key_field: &str, key_value: impl Params) -> Result<()> {
        let sql = format!("DELETE FROM {} WHERE {} = ?", Self::TABLE, key_field);
        conn.lock().unwrap().execute(&sql, key_value)?;
        Ok(())
    }

    fn select_all(conn: Connection) -> Result<Vec<Self>> {
        let sql = format!("SELECT {} FROM {}", Self::field_list(), Self::TABLE);
        let binding = { conn.lock().unwrap() };
        let mut stmt = binding.prepare(&sql)?;
        Ok(stmt
            .query_map([], Self::from_row)?
            .collect::<rusqlite::Result<_>>()
            .map_err(Error::from)?)
    }

    fn select_where<'a>(
        conn: Connection,
        field: &str,
        value: Vec<&'a dyn ToSql>,
    ) -> Result<Vec<Self>> {
        let sql = format!(
            "SELECT {} FROM {} WHERE {} = ?",
            Self::field_list(),
            Self::TABLE,
            field
        );

        let binding = { conn.lock().unwrap() };
        let mut stmt = binding.prepare(&sql)?;
        Ok(stmt
            .query_map(params_from_iter(value), Self::from_row)?
            .collect::<rusqlite::Result<_>>()
            .map_err(Error::from)?)
    }

    fn insert_params<'a>(&'a self) -> Vec<&'a dyn ToSql> {
        self.params().into_iter().skip(1).collect()
    }

    fn params<'a>(&'a self) -> Vec<&'a dyn ToSql>;
}

pub trait Joinable<A: Queriable + FromRow, B: Queriable + FromRow> {
    fn inner_join(
        _conn: &Connection,
        left_field: &str,
        right_field: &str,
        where_clause: Option<&str>,
    ) -> Result<Vec<(A, B)>> {
        let _sql = format!(
            "SELECT a.{}, b.{} FROM {} AS a INNER JOIN {} AS b ON a.{} = b.{} {}",
            A::field_list(),
            B::field_list(),
            A::TABLE,
            B::TABLE,
            left_field,
            right_field,
            where_clause
                .map(|w| format!("WHERE {w}"))
                .unwrap_or_default()
        );

        // let mut stmt = conn.prepare_cached(&sql, ())?;
        let out = Vec::new();

        // while let Some(r) = rows.next().await? {
        //     let a = A::from_row(&r)?;
        //     let b = B::from_row(&r)?;
        //     out.push((a, b));
        // }

        Ok(out)
    }
}

pub struct Sqlite {
    conn: Connection,
    index: Arc<Option<VerseIndex>>,
    cross_index: Arc<Option<CrossRefIndex>>,
}

impl Migrate for Sqlite {
    fn migrate(&self) -> Result<()> {
        let sql = r#"BEGIN;
CREATE TABLE IF NOT EXISTS languages (
    id TEXT NOT NULL,
    direction TEXT NOT NULL,
    name_local TEXT NOT NULL,
    name_english TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_languages_id ON languages(id);
CREATE INDEX IF NOT EXISTS idx_languages_local ON languages(name_local);
CREATE TABLE IF NOT EXISTS bibles (
    id TEXT NOT NULL,
    name_local TEXT NOT NULL,
    name_english TEXT NOT NULL,
    language_id TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_bibles_id ON bibles(id);
CREATE INDEX IF NOT EXISTS idx_bibles_language ON bibles(language_id);
CREATE TABLE IF NOT EXISTS books (
    id TEXT NOT NULL,
    bible_id TEXT NOT NULL,
    name_normal TEXT NOT NULL,
    name_long TEXT,
    name_abbrev TEXT
);
CREATE INDEX IF NOT EXISTS idx_books_id ON books(id);
CREATE INDEX IF NOT EXISTS idx_books_bible ON books(bible_id);
CREATE TABLE IF NOT EXISTS verses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    book_id TEXT NOT NULL,
    chapter_number INTEGER NOT NULL,
    verse_number INTEGER NOT NULL,
    text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_verses_book_chapter ON verses(book_id, chapter_number, verse_number);
CREATE TABLE IF NOT EXISTS verse_notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    book_id TEXT NOT NULL,
    chapter_number INTEGER NOT NULL,
    verse_number INTEGER NOT NULL,
    text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_verse_notes_book_chapter ON verse_notes(book_id, chapter_number, verse_number);
CREATE TABLE IF NOT EXISTS headers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    bible_id TEXT NOT NULL,
    book_id TEXT NOT NULL,
    chapter_number INTEGER NOT NULL,
    text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_headers_bible_book ON headers(bible_id, book_id);
CREATE TABLE IF NOT EXISTS cross_references (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_book TEXT NOT NULL,
    source_chapter INTEGER NOT NULL,
    source_verse INTEGER NOT NULL,
    target_book TEXT NOT NULL,
    target_chapter INTEGER NOT NULL,
    target_verse INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_cross_source ON cross_references(source_book, source_chapter, source_verse);
CREATE INDEX IF NOT EXISTS idx_cross_target ON cross_references(target_book, target_chapter, target_verse);
COMMIT;
"#;
        self.conn.lock().unwrap().execute_batch(&sql)?;
        Ok(())
    }
}

impl Sqlite {
    pub fn new(db: impl AsRef<Path>) -> Result<Self> {
        let conn = rusqlite::Connection::open(&db)?;
        let conn = Arc::new(Mutex::new(conn));

        let db_path = db.as_ref();
        let index_path = db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("index");

        let index = Arc::new(VerseIndex::new(&index_path).ok());

        let cross_index_path = db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("cross_index");

        let cross_index = Arc::new(CrossRefIndex::new(cross_index_path).ok());

        let db = Self {
            conn,
            index,
            cross_index,
        };

        db.migrate()?;

        Ok(db)
    }

    pub fn connection(&self) -> Connection {
        self.conn.clone()
    }

    pub fn verse_index(&self) -> Arc<Option<VerseIndex>> {
        self.index.clone()
    }

    pub fn cross_ref_index(&self) -> Arc<Option<CrossRefIndex>> {
        self.cross_index.clone()
    }
}
