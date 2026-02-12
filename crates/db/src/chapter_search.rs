use serde::{Deserialize, Serialize};

/// Texto completo de un versículo referenciado por una referencia cruzada.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossReferenceVerse {
    pub book_id: String,
    pub book_name: String,
    pub chapter: i32,
    pub verse: i32,
    pub text: String,
}

/// Versículo dentro del capítulo con indicador de resaltado y sus referencias cruzadas resueltas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterVerse {
    pub verse_number: i32,
    pub text: String,
    /// `true` si este versículo coincide con la búsqueda (para resaltado).
    pub highlighted: bool,
    /// Versículos completos de las referencias cruzadas de este versículo.
    pub cross_references: Vec<CrossReferenceVerse>,
}

/// Encabezado de sección dentro del capítulo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterHeader {
    pub text: String,
}

/// Resultado completo de una búsqueda con contexto de capítulo.
///
/// Contiene todos los versículos del capítulo, con los versículos buscados
/// marcados como `highlighted`, los encabezados del capítulo, y las
/// referencias cruzadas resueltas (con texto) para cada versículo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterSearchResult {
    pub bible_id: String,
    pub bible_name: String,
    pub book_id: String,
    pub book_name: String,
    pub chapter: i32,
    pub headers: Vec<ChapterHeader>,
    pub verses: Vec<ChapterVerse>,
}
