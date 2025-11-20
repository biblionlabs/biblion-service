use std::cmp;

use deunicode::deunicode;
use regex::Regex;
use rusqlite::Result;
use serde::{Deserialize, Serialize};

use crate::Connection;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchedBible {
    pub id: String,
    pub name: String,
    pub english_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchedVerse {
    pub bible: SearchedBible,
    pub book: String,
    pub chapter: i32,
    pub part: i32,
    pub verse: (i32, i32),
    pub text: String,
}

fn normalize(s: &str) -> String {
    // deunicode -> lowercase -> remove punctuation -> collapse spaces
    let ascii = deunicode(s);
    // remove non-word characters except whitespace
    let re = Regex::new(r"[^\w\s]").unwrap();
    let cleaned = re.replace_all(&ascii, " ");
    cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl SearchedVerse {
    pub fn from_search(conn: Connection, search: &str) -> Result<Vec<Self>> {
        // Parse tokens separated by comma
        let tokens: Vec<String> = search
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        // regex to capture book [+ chapter [+ :verse[-end]]]
        // book name can include spaces; we take everything until we hit a number for chapter
        let pattern = Regex::new(
            r"(?i)^\s*(?P<book>.+?)\s+(?P<chapter>\d+)(?::(?P<start>\d+)(?:-(?P<end>\d+))?)?\s*$",
        )
        .unwrap();

        let mut out: Vec<SearchedVerse> = Vec::new();
        let db = conn.lock().unwrap();

        // Function to push a verse row into out, splitting into 20-word parts if needed
        let push_verse = |out: &mut Vec<SearchedVerse>,
                          bible_id: String,
                          bible_local: String,
                          bible_english: String,
                          book_name: String,
                          chapter_num: i32,
                          verse_num: i32,
                          text: String| {
            out.push(SearchedVerse {
                bible: SearchedBible {
                    id: bible_id,
                    name: bible_local,
                    english_name: bible_english,
                },
                book: book_name,
                chapter: chapter_num,
                part: 0,
                verse: (verse_num, 1),
                text,
            });
        };

        // Track if any structured token matched; if none, we'll do a content search
        let mut any_structured = false;

        for token in &tokens {
            if let Some(caps) = pattern.captures(token) {
                any_structured = true;
                let book_query = caps.name("book").unwrap().as_str().trim().to_lowercase();
                let chapter: i32 = caps.name("chapter").unwrap().as_str().parse().unwrap_or(0);
                let start_verse: Option<i32> =
                    caps.name("start").map(|m| m.as_str().parse().unwrap_or(0));
                let end_verse: Option<i32> =
                    caps.name("end").map(|m| m.as_str().parse().unwrap_or(0));

                // Find matching books across all bibles by name or id (case-insensitive, partial match)
                let like = format!("%{}%", book_query);
                let mut stmt = db.prepare(
                    "SELECT books.id as book_id, books.name_normal as book_name, bibles.id as bible_id, bibles.name_local as bible_local, bibles.name_english as bible_english
                     FROM books
                     JOIN bibles ON books.bible_id = bibles.id
                     WHERE lower(books.name_normal) LIKE ?
                        OR lower(books.name_long) LIKE ?
                        OR lower(books.name_abbrev) LIKE ?
                        OR lower(books.id) LIKE ?
                     GROUP BY books.id, bibles.id
                     ",
                )?;
                let rows = stmt.query_map(rusqlite::params![like, like, like, like], |r| {
                    Ok((
                        r.get::<_, String>("book_id")?,
                        r.get::<_, String>("book_name")?,
                        r.get::<_, String>("bible_id")?,
                        r.get::<_, String>("bible_local")?,
                        r.get::<_, String>("bible_english")?,
                    ))
                })?;

                for row_res in rows {
                    let (book_id, book_name, bible_id, bible_local, bible_english) = row_res?;
                    // Build verse selection depending on presence of start_verse/end_verse
                    if let Some(sv) = start_verse {
                        let ev = end_verse.unwrap_or(sv);
                        let mut vstmt = db.prepare(
                            "SELECT verses.chapter_number as chapter, verses.verse_number as verse, verses.text as text
                             FROM verses
                             WHERE verses.book_id = ? AND verses.chapter_number = ? AND verses.verse_number BETWEEN ? AND ?
                             ORDER BY verses.chapter_number, verses.verse_number",
                        )?;
                        let vrows =
                            vstmt.query_map(rusqlite::params![book_id, chapter, sv, ev], |vr| {
                                Ok((
                                    vr.get::<_, i32>("chapter")?,
                                    vr.get::<_, i32>("verse")?,
                                    vr.get::<_, String>("text")?,
                                ))
                            })?;
                        for vrow in vrows {
                            let (ch, vnum, text) = vrow?;
                            push_verse(
                                &mut out,
                                bible_id.clone(),
                                bible_local.clone(),
                                bible_english.clone(),
                                book_name.clone(),
                                ch,
                                vnum,
                                text,
                            );
                        }
                    } else {
                        // only chapter specified -> select all verses in chapter
                        let mut vstmt = db.prepare(
                            "SELECT verses.chapter_number as chapter, verses.verse_number as verse, verses.text as text
                             FROM verses
                             WHERE verses.book_id = ? AND verses.chapter_number = ?
                             ORDER BY verses.chapter_number, verses.verse_number",
                        )?;
                        let vrows = vstmt.query_map(rusqlite::params![book_id, chapter], |vr| {
                            Ok((
                                vr.get::<_, i32>("chapter")?,
                                vr.get::<_, i32>("verse")?,
                                vr.get::<_, String>("text")?,
                            ))
                        })?;
                        for vrow in vrows {
                            let (ch, vnum, text) = vrow?;
                            push_verse(
                                &mut out,
                                bible_id.clone(),
                                bible_local.clone(),
                                bible_english.clone(),
                                book_name.clone(),
                                ch,
                                vnum,
                                text,
                            );
                        }
                    }
                }
            } else {
                // Try to match a plain book name without chapter/verse: token like "Genesis" or "Gen"
                let book_only = token.trim();
                if !book_only.is_empty() {
                    any_structured = true;
                    let like = format!("%{}%", book_only.to_lowercase());
                    let mut stmt = db.prepare(
                        "SELECT books.id as book_id, books.name_normal as book_name, bibles.id as bible_id, bibles.name_local as bible_local, bibles.name_english as bible_english
                         FROM books
                         JOIN bibles ON books.bible_id = bibles.id
                         WHERE lower(books.name_normal) LIKE ?
                            OR lower(books.name_long) LIKE ?
                            OR lower(books.name_abbrev) LIKE ?
                            OR lower(books.id) LIKE ?
                         GROUP BY books.id, bibles.id
                         ",
                    )?;
                    let rows = stmt.query_map(rusqlite::params![like, like, like, like], |r| {
                        Ok((
                            r.get::<_, String>("book_id")?,
                            r.get::<_, String>("book_name")?,
                            r.get::<_, String>("bible_id")?,
                            r.get::<_, String>("bible_local")?,
                            r.get::<_, String>("bible_english")?,
                        ))
                    })?;
                    for row_res in rows {
                        let (book_id, book_name, bible_id, bible_local, bible_english) = row_res?;
                        let mut vstmt = db.prepare(
                            "SELECT verses.chapter_number as chapter, verses.verse_number as verse, verses.text as text
                             FROM verses
                             WHERE verses.book_id = ?
                             ORDER BY verses.chapter_number, verses.verse_number",
                        )?;
                        let vrows = vstmt.query_map(rusqlite::params![book_id], |vr| {
                            Ok((
                                vr.get::<_, i32>("chapter")?,
                                vr.get::<_, i32>("verse")?,
                                vr.get::<_, String>("text")?,
                            ))
                        })?;
                        for vrow in vrows {
                            let (ch, vnum, text) = vrow?;
                            push_verse(
                                &mut out,
                                bible_id.clone(),
                                bible_local.clone(),
                                bible_english.clone(),
                                book_name.clone(),
                                ch,
                                vnum,
                                text,
                            );
                        }
                    }
                }
            }
        }

        // If none of tokens matched a structured query, fallback to content search.
        if !any_structured {
            let qnorm = normalize(search);
            if qnorm.is_empty() {
                return Ok(out);
            }
            // Use the first word to limit SQL results, then fully filter in Rust using normalized text.
            let first_word = qnorm.split_whitespace().next().unwrap().to_string();
            let like = format!("%{}%", first_word);

            let mut stmt = db.prepare(
                "SELECT verses.book_id as book_id, books.name_normal as book_name, bibles.id as bible_id, bibles.name_local as bible_local, bibles.name_english as bible_english,
                        verses.chapter_number as chapter, verses.verse_number as verse, verses.text as text
                 FROM verses
                 JOIN books ON verses.book_id = books.id
                 JOIN bibles ON books.bible_id = bibles.id
                 WHERE lower(verses.text) LIKE ?
                 ORDER BY bibles.id, books.id, verses.chapter_number, verses.verse_number
                 ",
            )?;
            let rows = stmt.query_map(rusqlite::params![like.to_lowercase()], |r| {
                Ok((
                    r.get::<_, String>("book_id")?,
                    r.get::<_, String>("book_name")?,
                    r.get::<_, String>("bible_id")?,
                    r.get::<_, String>("bible_local")?,
                    r.get::<_, String>("bible_english")?,
                    r.get::<_, i32>("chapter")?,
                    r.get::<_, i32>("verse")?,
                    r.get::<_, String>("text")?,
                ))
            })?;

            for row_res in rows {
                let (book_id, book_name, bible_id, bible_local, bible_english, ch, vnum, text) =
                    row_res?;
                let normalized_text = normalize(&text);
                if normalized_text.contains(&qnorm) {
                    push_verse(
                        &mut out,
                        bible_id,
                        bible_local,
                        bible_english,
                        book_name,
                        ch,
                        vnum,
                        text,
                    );
                }
            }
        }

        Ok(out)
    }
}
