#![allow(async_fn_in_trait)]

use std::sync::Arc;

use limbo::params::IntoParams;
use limbo::{Connection, IntoValue, Result, Row, Value};

mod bible;
mod book;
mod cross;
mod lang;
mod verse;

pub use bible::*;
pub use book::*;
pub use cross::*;
pub use lang::*;
pub use verse::*;

pub trait Queriable {
    const TABLE: &str;
    const FIELDS: &[&str];

    fn field_list() -> String {
        Self::FIELDS.join(", ")
    }
}

pub trait Migrate {
    async fn migrate(&self) -> Result<()>;
}

pub trait FromRow: Sized {
    fn from_row(row: &Row) -> Result<Self>;
}

pub trait Crud: Queriable + FromRow + Send + Sync + Sized {
    async fn insert(&self, conn: &Connection) -> Result<()> {
        let fields = Self::FIELDS.join(", ");
        let placeholders = (1..=Self::FIELDS.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            "INSERT OR REPLACE INTO {} ({fields}) VALUES ({placeholders})",
            Self::TABLE
        );

        conn.execute(&sql, self.to_params()).await?;
        Ok(())
    }

    async fn update(
        &self,
        conn: &Connection,
        key_field: &str,
        key_value: impl IntoValue,
    ) -> Result<()> {
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

        let mut params = self.to_params();
        params.push(key_value.into_value()?);
        conn.execute(&sql, params).await?;
        Ok(())
    }

    async fn delete(conn: &Connection, key_field: &str, key_value: impl IntoParams) -> Result<()> {
        let sql = format!("DELETE FROM {} WHERE {} = ?", Self::TABLE, key_field);
        conn.execute(&sql, key_value).await?;
        Ok(())
    }

    async fn select_all(conn: &Connection) -> Result<Vec<Self>> {
        let sql = format!("SELECT {} FROM {}", Self::field_list(), Self::TABLE);
        let mut rows = conn.query(&sql, ()).await?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().await? {
            out.push(Self::from_row(&r)?);
        }
        Ok(out)
    }

    async fn select_where(
        conn: &Connection,
        field: &str,
        value: impl IntoParams,
    ) -> Result<Vec<Self>> {
        let sql = format!(
            "SELECT {} FROM {} WHERE {} = ?",
            Self::field_list(),
            Self::TABLE,
            field
        );

        let mut rows = conn.query(&sql, value).await?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().await? {
            out.push(Self::from_row(&r)?);
        }
        Ok(out)
    }

    fn to_params(&self) -> Vec<Value>;
}

pub trait Joinable<A: Queriable + FromRow, B: Queriable + FromRow> {
    async fn inner_join(
        conn: &Connection,
        left_field: &str,
        right_field: &str,
        where_clause: Option<&str>,
    ) -> Result<Vec<(A, B)>> {
        let sql = format!(
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

        let mut rows = conn.query(&sql, ()).await?;
        let mut out = Vec::new();

        while let Some(r) = rows.next().await? {
            let a = A::from_row(&r)?;
            let b = B::from_row(&r)?;
            out.push((a, b));
        }

        Ok(out)
    }
}

pub struct Sqlite {
    conn: Arc<Connection>,
}

impl Migrate for Sqlite {
    async fn migrate(&self) -> Result<()> {
        let schema = r#"
        CREATE TABLE IF NOT EXISTS languages (
            id TEXT PRIMARY KEY,
            name_local TEXT NOT NULL,
            name_english TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS bibles (
            id TEXT PRIMARY KEY,
            name_local TEXT NOT NULL,
            name_english TEXT NOT NULL,
            language_id TEXT NOT NULL,
            FOREIGN KEY(language_id) REFERENCES languages(id)
        );

        CREATE TABLE IF NOT EXISTS books (
            id TEXT PRIMARY KEY,
            bible_id TEXT NOT NULL,
            name_normal TEXT NOT NULL,
            name_long TEXT,
            name_abbrev TEXT,
            order_index INTEGER,
            FOREIGN KEY(bible_id) REFERENCES bibles(id)
        );

        CREATE TABLE IF NOT EXISTS verses (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            book_id TEXT NOT NULL,
            chapter_number INTEGER NOT NULL,
            verse_number INTEGER NOT NULL,
            text TEXT NOT NULL,
            FOREIGN KEY(book_id) REFERENCES books(id)
        );

        CREATE TABLE IF NOT EXISTS cross_references (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_book TEXT NOT NULL,
            source_chapter INTEGER NOT NULL,
            source_verse INTEGER NOT NULL,
            target_book TEXT NOT NULL,
            target_chapter INTEGER NOT NULL,
            target_verse INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_verses_chapter ON verses(chapter_id);
        CREATE INDEX IF NOT EXISTS idx_books_bible ON books(bible_id);
        "#;

        self.conn.execute(schema, ()).await?;
        Ok(())
    }
}

impl Sqlite {
    pub async fn new(db: &str) -> Result<Self> {
        let conn = limbo::Builder::new_local(db).build().await?;
        let conn = Arc::new(conn.connect()?);
        let db = Self { conn };

        db.migrate().await?;

        Ok(db)
    }
}
