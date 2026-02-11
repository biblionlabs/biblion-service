use std::collections::HashMap;
use std::io::BufRead;
use std::path::Path;
use std::sync::Arc;

use tantivy::tokenizer::{Token, TokenFilter, TokenStream, Tokenizer};

/// Bidirectional synonym lookup built from `.syn` files.
///
/// File format (one group per line, comma-separated, `#` for comments):
/// ```text
/// # Warfare synonyms
/// guerra,batalla,arma,armadura,espada,lanza,escudo,combate
/// amor,cariño,afecto,ternura
/// ```
///
/// Every word in a group maps to all the other words in that group.
#[derive(Clone, Debug)]
pub struct SynonymMap {
    inner: Arc<HashMap<String, Vec<String>>>,
}

impl SynonymMap {
    /// Build from an iterator of synonym groups.
    /// Each group is a `Vec<String>` where every word is a synonym of the others.
    pub fn from_groups(groups: impl IntoIterator<Item = Vec<String>>) -> Self {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for group in groups {
            if group.len() < 2 {
                continue;
            }
            for word in &group {
                let key = word.to_lowercase();
                let synonyms: Vec<String> = group
                    .iter()
                    .filter(|w| w.to_lowercase() != key)
                    .map(|w| w.to_lowercase())
                    .collect();
                map.entry(key).or_default().extend(synonyms);
            }
        }
        // Deduplicate
        for syns in map.values_mut() {
            syns.sort();
            syns.dedup();
        }
        Self {
            inner: Arc::new(map),
        }
    }

    /// Load from a `.syn` file (one group per line, comma-separated).
    pub fn from_file(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let mut groups = Vec::new();
        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let group: Vec<String> = trimmed
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
            if group.len() >= 2 {
                groups.push(group);
            }
        }
        Ok(Self::from_groups(groups))
    }

    /// Merge another SynonymMap into this one.
    pub fn merge(&mut self, other: &SynonymMap) {
        let mut map = (*self.inner).clone();
        for (key, syns) in other.inner.iter() {
            let entry = map.entry(key.clone()).or_default();
            entry.extend(syns.iter().cloned());
            entry.sort();
            entry.dedup();
        }
        self.inner = Arc::new(map);
    }

    /// Look up synonyms for a given word. Returns empty slice if none found.
    pub fn get(&self, word: &str) -> &[String] {
        self.inner
            .get(&word.to_lowercase())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }
}

/// A [`TokenFilter`] that injects synonyms at the same token position.
///
/// For each incoming token, if the synonym map contains entries for it,
/// the original token is emitted first, then each synonym is emitted at
/// the same position.
#[derive(Clone)]
pub struct SynonymTokenFilter {
    map: SynonymMap,
}

impl SynonymTokenFilter {
    pub fn new(map: SynonymMap) -> Self {
        Self { map }
    }
}

impl TokenFilter for SynonymTokenFilter {
    type Tokenizer<T: Tokenizer> = SynonymFilteredTokenizer<T>;

    fn transform<T: Tokenizer>(self, tokenizer: T) -> Self::Tokenizer<T> {
        SynonymFilteredTokenizer {
            map: self.map,
            inner: tokenizer,
            buffer: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct SynonymFilteredTokenizer<T> {
    map: SynonymMap,
    inner: T,
    buffer: Vec<Token>,
}

impl<T: Tokenizer> Tokenizer for SynonymFilteredTokenizer<T> {
    type TokenStream<'a> = SynonymTokenStream<'a, T::TokenStream<'a>>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        self.buffer.clear();
        SynonymTokenStream {
            map: &self.map,
            tail: self.inner.token_stream(text),
            buffer: &mut self.buffer,
        }
    }
}

pub struct SynonymTokenStream<'a, T> {
    map: &'a SynonymMap,
    tail: T,
    buffer: &'a mut Vec<Token>,
}

impl<T: TokenStream> SynonymTokenStream<'_, T> {
    /// Check if the current token has synonyms and push the original + synonyms
    /// into the buffer (in reverse order for pop-based emission via `last()`).
    fn expand_current_token(&mut self) {
        let token = self.tail.token();
        let synonyms = self.map.get(&token.text);

        if synonyms.is_empty() {
            // No synonyms — buffer stays empty, token() will fall through to tail
            return;
        }

        // Push synonyms in reverse order first (they'll be emitted last)
        for syn in synonyms.iter().rev() {
            self.buffer.push(Token {
                offset_from: token.offset_from,
                offset_to: token.offset_to,
                position: token.position,
                text: syn.clone(),
                position_length: 1,
            });
        }

        // Push original token last (it will be emitted first via last())
        self.buffer.push(Token {
            offset_from: token.offset_from,
            offset_to: token.offset_to,
            position: token.position,
            text: token.text.clone(),
            position_length: 1,
        });
    }
}

impl<T: TokenStream> TokenStream for SynonymTokenStream<'_, T> {
    fn advance(&mut self) -> bool {
        // Drain buffered synonym/original tokens first
        if !self.buffer.is_empty() {
            self.buffer.pop();
            if !self.buffer.is_empty() {
                return true;
            }
        }

        // Advance inner stream
        if !self.tail.advance() {
            return false;
        }

        // Expand synonyms for the new token
        self.expand_current_token();

        // If buffer is non-empty, we'll emit from buffer.
        // If buffer is empty (no synonyms), we emit from tail directly.
        true
    }

    fn token(&self) -> &Token {
        self.buffer.last().unwrap_or_else(|| self.tail.token())
    }

    fn token_mut(&mut self) -> &mut Token {
        self.buffer
            .last_mut()
            .unwrap_or_else(|| self.tail.token_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantivy::tokenizer::{SimpleTokenizer, TextAnalyzer};

    fn test_map() -> SynonymMap {
        SynonymMap::from_groups(vec![
            vec!["guerra".into(), "batalla".into(), "combate".into()],
            vec!["amor".into(), "cariño".into(), "afecto".into()],
        ])
    }

    #[test]
    fn synonym_map_lookup() {
        let map = test_map();
        let syns = map.get("guerra");
        assert!(syns.contains(&"batalla".to_string()));
        assert!(syns.contains(&"combate".to_string()));
        assert!(!syns.contains(&"guerra".to_string()));
    }

    #[test]
    fn synonym_filter_emits_synonyms() {
        let map = test_map();
        let mut analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(SynonymTokenFilter::new(map))
            .build();

        let mut stream = analyzer.token_stream("guerra");
        let mut tokens = Vec::new();
        while stream.advance() {
            tokens.push(stream.token().text.clone());
        }

        assert!(tokens.contains(&"guerra".to_string()));
        assert!(tokens.contains(&"batalla".to_string()));
        assert!(tokens.contains(&"combate".to_string()));
    }

    #[test]
    fn synonym_filter_no_synonyms() {
        let map = test_map();
        let mut analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(SynonymTokenFilter::new(map))
            .build();

        let mut stream = analyzer.token_stream("hello");
        let mut tokens = Vec::new();
        while stream.advance() {
            tokens.push(stream.token().text.clone());
        }

        assert_eq!(tokens, vec!["hello"]);
    }

    #[test]
    fn synonym_filter_multi_token() {
        let map = test_map();
        let mut analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(SynonymTokenFilter::new(map))
            .build();

        let mut stream = analyzer.token_stream("guerra amor");
        let mut tokens = Vec::new();
        while stream.advance() {
            tokens.push(stream.token().text.clone());
        }

        assert!(tokens.contains(&"guerra".to_string()));
        assert!(tokens.contains(&"batalla".to_string()));
        assert!(tokens.contains(&"combate".to_string()));
        assert!(tokens.contains(&"amor".to_string()));
        assert!(tokens.contains(&"cariño".to_string()));
        assert!(tokens.contains(&"afecto".to_string()));
    }
}
