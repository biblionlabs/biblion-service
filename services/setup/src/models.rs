use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub type CrossReference = HashMap<String, HashMap<String, Vec<Vec<Reference>>>>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Reference {
    Integer(i32),
    String(String),
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BibleVariant {
    pub book_names: HashMap<String, Name>,
    pub chapter_headings: HashMap<String, Vec<String>>,
    pub sections: HashMap<String, Vec<Section>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Section {
    Raw(String),
    Section {
        start_chapter: i64,
        start_verse: i64,
        end_chapter: i64,
        end_verse: i64,
        heading: Option<String>,
    },
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Name {
    pub normal: String,
    pub long: String,
    pub abbrev: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    pub book: String,
    pub name: Name,
    pub contents: Vec<Vec<Vec<Content>>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Note {
        r#type: ContentType,
        contents: String,
    },
    Heading {
        r#type: ContentType,
        contents: String,
        level: u8,
    },
    Raw(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentType {
    Note,
    Heading,
}
