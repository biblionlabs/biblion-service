use std::fmt::Display;

pub type Result<T> = std::result::Result<T, Setup>;

#[derive(Debug)]
pub enum Setup {
    DB(service_db::Error),
    Network(reqwest::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
    MissingField(&'static str, &'static str),
    Other(String),
}

impl Display for Setup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Setup::DB(e) => write!(f, "Database error: {e}"),
            Setup::Network(e) => write!(f, "Network error: {e}"),
            Setup::Io(e) => write!(f, "IO Error: {e}"),
            Setup::Json(e) => write!(f, "Json Error: {e}"),
            Setup::MissingField(from, field) => write!(f, "{from:?} missing {field:?}"),
            Setup::Other(e) => write!(f, "Other: {e}"),
        }
    }
}

impl std::error::Error for Setup {}

impl From<service_db::Error> for Setup {
    fn from(value: service_db::Error) -> Self {
        Self::DB(value)
    }
}

impl From<reqwest::Error> for Setup {
    fn from(value: reqwest::Error) -> Self {
        Self::Network(value)
    }
}

impl From<std::io::Error> for Setup {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for Setup {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<String> for Setup {
    fn from(value: String) -> Self {
        Self::Other(value)
    }
}

impl<'a> From<&'a str> for Setup {
    fn from(value: &'a str) -> Self {
        Self::Other(value.to_string())
    }
}
