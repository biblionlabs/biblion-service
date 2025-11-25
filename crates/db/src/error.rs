use std::fmt::Display;

pub type Result<T> = std::result::Result<T, DB>;

#[derive(Debug)]
pub enum DB {
    Tantivy(tantivy::error::TantivyError),
    Sqlite(crate::SqliteError),
    NoSearchResult,
    Other(String),
}

impl Display for DB {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DB::Tantivy(error) => write!(f, "Tantivy error: {error}"),
            DB::Sqlite(error) => write!(f, "Sqlite error: {error}"),
            DB::NoSearchResult => write!(f, "No found search result"),
            DB::Other(e) => write!(f, "Other: {e}"),
        }
    }
}

impl std::error::Error for DB {}

impl From<tantivy::error::TantivyError> for DB {
    fn from(value: tantivy::error::TantivyError) -> Self {
        Self::Tantivy(value)
    }
}

impl From<crate::SqliteError> for DB {
    fn from(value: crate::SqliteError) -> Self {
        Self::Sqlite(value)
    }
}
