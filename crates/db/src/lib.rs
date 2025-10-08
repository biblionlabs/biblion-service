use std::sync::Arc;

use limbo::{Connection, Result};

mod models;

use models::*;

pub struct Sqlite {
    conn: Arc<Connection>,
}

impl Sqlite {
    pub async fn new(db: &str) -> Result<Self> {
        let conn = limbo::Builder::new_local(db).build().await?;
        let conn = Arc::new(conn.connect()?);
        let db = Self { conn };

        db.migrate().await?;

        Ok(db)
    }

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

        CREATE TABLE IF NOT EXISTS chapters (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            book_id TEXT NOT NULL,
            chapter_number INTEGER NOT NULL,
            FOREIGN KEY(book_id) REFERENCES books(id)
        );

        CREATE TABLE IF NOT EXISTS verses (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            chapter_id INTEGER NOT NULL,
            verse_number INTEGER NOT NULL,
            text TEXT NOT NULL,
            FOREIGN KEY(chapter_id) REFERENCES chapters(id)
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

    // --- Language management ---
    pub async fn insert_language(&self, lang: &Language) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO languages (id, name_local, name_english) VALUES (?1, ?2, ?3)",
            (lang.id.as_str(), lang.name_local.as_str(), lang.name_english.as_str()),
        ).await?;
        Ok(())
    }

    pub async fn get_language(&self, id: &str) -> Result<Option<Language>> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, name_local, name_english FROM languages WHERE id = ?1",
                [id],
            )
            .await?;
        Ok(rows.next().await?.and_then(|r| {
            Some(Language {
                id: r.get_value(0).ok()?.as_text()?.clone(),
                name_local: r.get_value(1).ok()?.as_text()?.clone(),
                name_english: r.get_value(2).ok()?.as_text()?.clone(),
            })
        }))
    }

    // --- Bible management ---
    pub async fn insert_bible(&self, bible: &Bible) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO bibles (id, name_local, name_english, language_id) VALUES (?1, ?2, ?3, ?4)",
                (bible.id.as_str(), bible.name_local.as_str(), bible.name_english.as_str(), bible.language_id.as_str()),
        ).await?;
        Ok(())
    }

    pub async fn get_bible(&self, id: &str) -> Result<Option<Bible>> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, name_local, name_english, language_id FROM bibles WHERE id = ?1",
                [id],
            )
            .await?;
        Ok(rows.next().await?.and_then(|r| {
            Some(Bible {
                id: r.get_value(0).ok()?.as_text()?.clone(),
                name_local: r.get_value(1).ok()?.as_text()?.clone(),
                name_english: r.get_value(2).ok()?.as_text()?.clone(),
                language_id: r.get_value(3).ok()?.as_text()?.clone(),
            })
        }))
    }

    // --- Books ---
    pub async fn insert_book(&self, book: &Book) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO books (id, bible_id, name_normal, name_long, name_abbrev, order_index)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (book.id.as_str(),
                book.bible_id.as_str(),
                book.name_normal.as_str(),
                book.name_long.as_str(),
                book.name_abbrev.as_str(),
                book.order_index),
        ).await?;
        Ok(())
    }

    pub async fn list_books(&self, bible_id: &str) -> Result<Vec<Book>> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, bible_id, name_normal, name_long, name_abbrev, order_index FROM books WHERE bible_id = ?1 ORDER BY order_index ASC",
                [bible_id],
            )
            .await?;

        let mut out = Vec::new();
        while let Some(r) = rows.next().await? {
            out.push(Book {
                id: r.get_value(0)?.as_text().unwrap().clone(), // id
                bible_id: r.get_value(1)?.as_text().unwrap().clone(), // bible_id
                name_normal: r.get_value(2)?.as_text().unwrap().clone(), // name_normal
                name_long: r.get_value(3)?.as_text().unwrap().clone(), // name_long
                name_abbrev: r.get_value(4)?.as_text().unwrap().clone(), // name_abbrev
                order_index: *r.get_value(5)?.as_integer().unwrap() as i32, // order_index
            });
        }
        Ok(out)
    }

    // --- Chapters ---
    pub async fn insert_chapter(&self, ch: &Chapter) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO chapters (book_id, chapter_number) VALUES (?1, ?2)",
                (ch.book_id.as_str(), ch.chapter_number),
            )
            .await?;
        Ok(())
    }

    pub async fn get_chapters(&self, book_id: &str) -> Result<Vec<Chapter>> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, book_id, chapter_number FROM chapters WHERE book_id = ?1 ORDER BY chapter_number ASC",
                [book_id],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().await? {
            out.push(Chapter {
                id: *r.get_value(0)?.as_integer().unwrap(),          // id
                book_id: r.get_value(1)?.as_text().unwrap().clone(), // book_id
                chapter_number: *r.get_value(2)?.as_integer().unwrap() as _, // chapter_number
            });
        }
        Ok(out)
    }

    // --- Verses ---
    pub async fn insert_verse(&self, v: &Verse) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO verses (chapter_id, verse_number, text) VALUES (?1, ?2, ?3)",
                (v.chapter_id, v.verse_number, v.text.as_str()),
            )
            .await?;
        Ok(())
    }

    pub async fn get_verses_by_chapter(&self, chapter_id: i64) -> Result<Vec<Verse>> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, chapter_id, verse_number FROM verses WHERE chapter_id = ?1 ORDER BY verse_number ASC",
                [chapter_id],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().await? {
            out.push(Verse {
                id: *r.get_value(0)?.as_integer().unwrap(),         // id
                chapter_id: *r.get_value(1)?.as_integer().unwrap(), // chapter_id
                verse_number: *r.get_value(2)?.as_integer().unwrap() as _, // verse_number
                text: r.get_value(3)?.as_text().unwrap().clone(),   // text
            });
        }
        Ok(out)
    }

    // --- Cross References ---
    pub async fn insert_cross_reference(&self, cr: &CrossReference) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO cross_references (
                source_book, source_chapter, source_verse,
                target_book, target_chapter, target_verse
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    cr.source_book.as_str(),
                    cr.source_chapter,
                    cr.source_verse,
                    cr.target_book.as_str(),
                    cr.target_chapter,
                    cr.target_verse,
                ),
            )
            .await?;
        Ok(())
    }

    pub async fn get_cross_references(
        &self,
        book: &str,
        chapter: i32,
        verse: i32,
    ) -> Result<Vec<CrossReference>> {
        let mut rows = self.conn.query(
            "SELECT id, source_book, source_chapter, source_verse, target_book, target_chapter, target_verse FROM cross_references WHERE source_book = ?1 AND source_chapter = ?2 AND source_verse = ?3",
            (book, chapter, verse),
        ).await?;

        let mut out = Vec::new();
        while let Some(r) = rows.next().await? {
            out.push(CrossReference {
                id: *r.get_value(0)?.as_integer().unwrap(), // id
                source_book: r.get_value(1)?.as_text().unwrap().clone(), // source_book
                source_chapter: *r.get_value(2)?.as_integer().unwrap() as _, // source_chapter
                source_verse: *r.get_value(3)?.as_integer().unwrap() as _, // source_verse
                target_book: r.get_value(4)?.as_text().unwrap().clone(), // target_book
                target_chapter: *r.get_value(5)?.as_integer().unwrap() as _, // target_chapter
                target_verse: *r.get_value(6)?.as_integer().unwrap() as _, // target_verse
            });
        }
        Ok(out)
    }
}
