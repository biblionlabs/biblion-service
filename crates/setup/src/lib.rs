//! Core orchestrator crate for download/cache and DB-orchestration via a DbSink trait.
//!
//! Main points:
//! - SetupBuilder to configure cache, db_path and extra bibles (manifest url + optional books URL template).
//! - Setup exposes helpers to list languages and bibles from the main manifest so frontends can prompt the user.
//! - DbSink trait: implement this in the consumer to have the orchestrator create the database using cached files.

#![warn(async_fn_in_trait)]

use parking_lot::Mutex;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

mod builder;
mod db;
mod error;
mod models;

pub use builder::*;
pub use db::*;
pub use error::{Result, Setup as Error};
pub use models::*;

/// Events for UI/logging
#[derive(Debug, Clone)]
pub enum Event {
    Message(String),
    Progress {
        step: String,
        current: u64,
        total: u64,
    },
    ManifestReady {
        manifest: Value,
        path: PathBuf,
    },
    CrossRefCached {
        book_id: String,
        path: PathBuf,
    },
    BibleManifestCached {
        bible_id: String,
        path: PathBuf,
    },
    BibleBookCached {
        bible_id: String,
        book_id: String,
        path: PathBuf,
    },
    SelectionSaved {
        path: PathBuf,
    },
    Completed,
    Error(String),
}

trait TEvent {
    type Args;
}
struct Completed;
struct Progress;
impl TEvent for Completed {
    type Args = ();
}

impl TEvent for Progress {
    type Args = (String, u64, u64);
}

/// Selection struct for the pipeline (can be constructed by UI from helper methods)
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Selection {
    pub languages: Vec<String>,
    pub bibles: Vec<String>,
    pub originals_added: Vec<String>,
    pub include_originals: bool,
}

/// The main orchestrator
#[derive(Clone)]
pub struct Setup {
    client: Client,
    cache_path: PathBuf,
    include_originals: bool,
    extra_bibles: Vec<ExtraBible>,
    callbacks: Arc<Mutex<Vec<Arc<dyn Fn(Event) + Send + Sync>>>>,
    manifest_ttl: Duration,
}

impl Setup {
    fn emit(&self, ev: Event) {
        for cb in self.callbacks.lock().iter() {
            (cb)(ev.clone());
        }
    }

    /// returns manifest Value and path
    pub async fn load_manifest(&self) -> Result<(Value, PathBuf)> {
        let cache_dir = &self.cache_path;
        let manifest_path = cache_dir.join("manifest.json");
        let need_download = if manifest_path.exists() {
            match manifest_path.metadata()?.modified() {
                Ok(modified) => match SystemTime::now().duration_since(modified) {
                    Ok(elapsed) => elapsed > self.manifest_ttl,
                    Err(_) => true,
                },
                Err(_) => true,
            }
        } else {
            true
        };

        std::fs::create_dir_all(cache_dir)?;

        let mut v: Value = if need_download {
            self.emit(Event::Message("Downloading manifest...".into()));
            let text = self
                .client
                .get("https://v1.fetch.bible/manifest.json")
                .send()
                .await?
                .text()
                .await?;
            serde_json::from_str(&text)?
        } else {
            self.emit(Event::Message("Using cached manifest".into()));
            let bytes = std::fs::read(&manifest_path)?;
            serde_json::from_slice(&bytes)?
        };

        // Merge extra bible manifests provided via builder into the main manifest `v`.
        // Each extra manifest is expected to have the same top-level structure (e.g. "bibles", "languages", etc.).
        // We try to fetch them from their manifest_url and merge; failures are logged as messages but do not abort.
        for extra in &self.extra_bibles {
            let url = &extra.desc_url;
            self.emit(Event::Message(format!(
                "Attempting to fetch extra manifest for {} from {}",
                extra.id, url
            )));
            match self.client.get(url).send().await {
                Ok(resp) if resp.status().is_success() => match resp.text().await {
                    Ok(text) => match serde_json::from_str::<Value>(&text) {
                        Ok(extra_val) => {
                            let new_bible = v.get_mut("bibles").unwrap().as_object_mut().unwrap();
                            if !new_bible.contains_key(&extra.id) {
                                if new_bible.insert(extra.id.to_string(), extra_val).is_none() {
                                    self.emit(Event::Message(format!(
                                        "Merged extra manifest for {}",
                                        extra.id
                                    )));
                                } else {
                                    self.emit(Event::Message(format!(
                                        "Merged extra manifest for {} failed!",
                                        extra.id
                                    )));
                                }
                            }
                        }
                        Err(e) => {
                            self.emit(Event::Message(format!(
                                "Failed to parse extra manifest for {}: {e}",
                                extra.id
                            )));
                        }
                    },
                    Err(e) => {
                        self.emit(Event::Message(format!(
                            "Failed to read extra manifest response for {}: {e}",
                            extra.id
                        )));
                    }
                },
                _ => {
                    self.emit(Event::Message(format!(
                        "Failed to fetch extra manifest for {} (url: {url}), skipping",
                        extra.id
                    )));
                }
            }
        }

        // Persist merged manifest back to cache path (either newly downloaded or updated cached)
        let serialized = serde_json::to_string_pretty(&v)?;
        std::fs::write(&manifest_path, serialized.as_bytes())?;

        // Emit readiness event with merged manifest
        self.emit(Event::ManifestReady {
            manifest: v.clone(),
            path: manifest_path.clone(),
        });

        Ok((v, manifest_path))
    }

    /// helpers for UI: list languages (code, english, local)
    pub async fn list_languages(&self) -> Result<Vec<(String, String, String)>> {
        let (manifest, _) = self.load_manifest().await?;
        let languages_obj = manifest
            .get("languages")
            .and_then(|v| v.as_object())
            .ok_or(Error::MissingField("manifest", "languages"))?;
        let mut language_entries: Vec<(String, String, String)> = languages_obj
            .iter()
            .map(|(k, v)| {
                let english = v
                    .get("english")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                let local = v
                    .get("local")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                (k.clone(), english, local)
            })
            .collect();
        language_entries.sort_by(|a, b| a.1.cmp(&b.1));
        Ok(language_entries)
    }

    /// helpers for UI: list bibles (id, local, english, language)
    pub async fn list_bibles(&self) -> Result<Vec<(String, String, String, String)>> {
        let (manifest, _) = self.load_manifest().await?;
        let bibles_obj = manifest
            .get("bibles")
            .and_then(|v| v.as_object())
            .ok_or(Error::MissingField("manifest", "bibles"))?;
        let mut bible_entries: Vec<(String, String, String, String)> = bibles_obj
            .iter()
            .map(|(k, v)| {
                let local = v
                    .get("name")
                    .and_then(|n| n.get("local"))
                    .and_then(|s| s.as_str())
                    .unwrap_or_default()
                    .to_string();
                let english = v
                    .get("name")
                    .and_then(|n| n.get("english"))
                    .and_then(|s| s.as_str())
                    .unwrap_or_default()
                    .to_string();
                let language = v
                    .get("language")
                    .and_then(|s| s.as_str())
                    .unwrap_or_default()
                    .to_string();
                (k.clone(), local, english, language)
            })
            .collect();
        bible_entries.sort_by(|a, b| a.2.cmp(&b.2));
        Ok(bible_entries)
    }

    /// Save selection
    pub fn save_selection(&self, sel: &Selection) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.cache_path)?;
        let path = self.cache_path.join("selection.json");
        let bytes = serde_json::to_vec_pretty(sel)?;
        std::fs::write(&path, &bytes)?;
        self.emit(Event::SelectionSaved { path: path.clone() });
        Ok(path)
    }

    /// cache crossrefs (returns paths)
    pub async fn cache_crossrefs(&self, manifest: &Value) -> Result<Vec<PathBuf>> {
        let cache_dir = self.cache_path.join("cross");
        std::fs::create_dir_all(&cache_dir)?;
        let mut saved = Vec::new();

        let book_keys = manifest
            .get("book_names_english")
            .and_then(|v| v.as_object())
            .map(|o| o.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        let total = book_keys.len() as u64;
        let mut current = 0u64;

        for book_id in book_keys {
            current += 1;
            let target = cache_dir.join(format!("{book_id}.json"));
            if !target.exists() {
                match self
                    .client
                    .get(format!(
                        "https://v1.fetch.bible/crossref/large/{book_id}.json"
                    ))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        let text = resp.text().await?;
                        std::fs::write(&target, text.as_bytes())?;
                        self.emit(Event::CrossRefCached {
                            book_id: book_id.clone(),
                            path: target.clone(),
                        });
                        saved.push(target);
                    }
                    _ => {
                        self.emit(Event::Message(format!(
                            "Skipping crossref {book_id} (network failed, no cache)"
                        )));
                    }
                }
            } else {
                self.emit(Event::CrossRefCached {
                    book_id: book_id.clone(),
                    path: target.clone(),
                });
                saved.push(target);
            }
            self.emit(Event::Progress {
                step: "crossrefs".to_string(),
                current,
                total,
            });
        }
        Ok(saved)
    }

    /// Cache bible manifests (including extra manifests provided via extra_bibles)
    /// returns Vec<(bible_id, manifest_path)>
    pub async fn cache_bible_manifests(
        &self,
        manifest: &Value,
        bible_ids: &[String],
    ) -> Result<Vec<(String, PathBuf)>> {
        let mut results = Vec::new();
        // build a fast lookup for extra_bibles
        let extra_map: HashMap<String, &ExtraBible> = self
            .extra_bibles
            .iter()
            .map(|b| (b.id.clone(), b))
            .collect();

        let global_bibles_obj = manifest.get("bibles").and_then(|v| v.as_object());

        for bible_id in bible_ids {
            let bible_dir = self.cache_path.join("bibles").join(bible_id);
            std::fs::create_dir_all(&bible_dir)?;
            let manifest_path = bible_dir.join("manifest.json");

            // If the manifest param doesn't contain this bible, emit an informative message.
            if let Some(g) = global_bibles_obj {
                if !g.contains_key(bible_id) {
                    self.emit(Event::Message(format!(
                    "Bible id {bible_id} not present in global manifest; treating as external or using provided sources"
                )));
                }
            }

            // First prefer explicit extra_bibles configured via builder
            if let Some(extra) = extra_map.get(bible_id) {
                // attempt to fetch from provided manifest URL
                self.emit(Event::Message(format!(
                    "Downloading extra bible manifest for {bible_id} from {}",
                    extra.manifest_url
                )));
                match self.client.get(&extra.manifest_url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        let text = resp.text().await?;
                        std::fs::write(&manifest_path, text.as_bytes())?;
                        results.push((bible_id.clone(), manifest_path.clone()));
                        self.emit(Event::BibleManifestCached {
                            bible_id: bible_id.clone(),
                            path: manifest_path.clone(),
                        });
                        continue;
                    }
                    _ => {
                        self.emit(Event::Message(format!(
                        "Failed to fetch extra manifest for {bible_id}, will try default endpoint or cached file"
                    )));
                        // fallthrough
                    }
                }
            }

            // If the global manifest contains a URL for this bible, try to use it.
            let manifest_url_from_global = global_bibles_obj
                .and_then(|g| g.get(bible_id))
                .and_then(|entry| {
                    // common possible keys that might contain a remote manifest location
                    entry
                        .get("manifest_url")
                        .or_else(|| entry.get("url"))
                        .and_then(|v| v.as_str())
                })
                .map(|s| s.to_string());

            if let Some(url) = manifest_url_from_global {
                self.emit(Event::Message(format!(
                "Downloading bible manifest for {bible_id} from manifest URL declared in global manifest: {url}"
            )));
                match self.client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        let text = resp.text().await?;
                        std::fs::write(&manifest_path, text.as_bytes())?;
                        results.push((bible_id.clone(), manifest_path.clone()));
                        self.emit(Event::BibleManifestCached {
                            bible_id: bible_id.clone(),
                            path: manifest_path.clone(),
                        });
                        continue;
                    }
                    _ => {
                        self.emit(Event::Message(format!(
                        "Failed to fetch manifest for {bible_id} from declared URL; will try default endpoint or cached file"
                    )));
                        // fallthrough
                    }
                }
            }

            // default behaviour: attempt to fetch from fetch.bible endpoint
            self.emit(Event::Message(format!(
                "Fetching manifest for {bible_id} from fetch.bible"
            )));
            match self
                .client
                .get(format!(
                    "https://v1.fetch.bible/bibles/{bible_id}/extra.json"
                ))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    let text = resp.text().await?;
                    std::fs::write(&manifest_path, text.as_bytes())?;
                    results.push((bible_id.clone(), manifest_path.clone()));
                    self.emit(Event::BibleManifestCached {
                        bible_id: bible_id.clone(),
                        path: manifest_path.clone(),
                    });
                }
                _ => {
                    if manifest_path.exists() {
                        self.emit(Event::Message(format!(
                            "Using cached manifest for {bible_id}"
                        )));
                        results.push((bible_id.clone(), manifest_path.clone()));
                        self.emit(Event::BibleManifestCached {
                            bible_id: bible_id.clone(),
                            path: manifest_path.clone(),
                        });
                    } else {
                        self.emit(Event::Message(format!(
                            "Failed to fetch manifest for {bible_id} and no cache present"
                        )));
                    }
                }
            }
        }
        Ok(results)
    }

    /// Cache books for bibles; uses manifest to find book ids, and falls back to extra book_url_template when manifest doesn't provide file content.
    /// returns tuples (bible_id, book_id, path)
    pub async fn cache_bible_books(
        &self,
        manifests: &[(String, PathBuf)],
    ) -> Result<Vec<(String, String, PathBuf)>> {
        let mut out = Vec::new();
        // build map of extra templates
        let mut template_map: HashMap<String, String> = HashMap::new();
        for b in &self.extra_bibles {
            if let Some(t) = &b.book_url_template {
                template_map.insert(b.id.clone(), t.clone());
            }
        }

        for (bible_id, manifest_path) in manifests {
            let text = std::fs::read_to_string(manifest_path)?;
            let v: Value = serde_json::from_str(&text)?;
            // Try multiple locations for book ids
            let books: Vec<String> = v
                .get("book_names")
                .and_then(|b| b.as_object())
                .map(|o| o.keys().cloned().collect::<Vec<_>>())
                .or_else(|| {
                    v.get("chapter_headings")
                        .and_then(|b| b.as_object())
                        .map(|o| o.keys().cloned().collect::<Vec<_>>())
                })
                .or_else(|| {
                    v.get("book_names_english")
                        .and_then(|b| b.as_object())
                        .map(|o| o.keys().cloned().collect::<Vec<_>>())
                })
                .unwrap_or_default();

            let bible_dir = manifest_path.parent().unwrap().to_path_buf();
            let books_dir = bible_dir.join("books");
            std::fs::create_dir_all(&books_dir)?;

            let total = books.len() as u64;
            let mut current = 0u64;
            for book_id in books {
                current += 1;
                let target = books_dir.join(format!("{book_id}.json"));
                if !target.exists() {
                    // If an extra template exists for this bible, use it:
                    if let Some(template) = template_map.get(bible_id) {
                        // replace placeholders
                        let url = template
                            .replace("{bible_id}", bible_id)
                            .replace("{book}", &book_id);
                        match self.client.get(&url).send().await {
                            Ok(resp) if resp.status().is_success() => {
                                let text = resp.text().await?;
                                std::fs::write(&target, text.as_bytes())?;
                                out.push((bible_id.clone(), book_id.clone(), target.clone()));
                                self.emit(Event::BibleBookCached {
                                    bible_id: bible_id.clone(),
                                    book_id: book_id.clone(),
                                    path: target.clone(),
                                });
                            }
                            _ => {
                                self.emit(Event::Message(format!(
                                    "Skipped book {book_id} of {bible_id} (fetch failed at template)"
                                )));
                            }
                        }
                    } else {
                        // fallback to fetch.bible default endpoint
                        let url =
                            format!("https://v1.fetch.bible/bibles/{bible_id}/txt/{book_id}.json");
                        match self.client.get(&url).send().await {
                            Ok(resp) if resp.status().is_success() => {
                                let text = resp.text().await?;
                                std::fs::write(&target, text.as_bytes())?;
                                out.push((bible_id.clone(), book_id.clone(), target.clone()));
                                self.emit(Event::BibleBookCached {
                                    bible_id: bible_id.clone(),
                                    book_id: book_id.clone(),
                                    path: target.clone(),
                                });
                            }
                            _ => {
                                self.emit(Event::Message(format!(
                                    "Skipped book {book_id} of {bible_id} (fetch failed)"
                                )));
                            }
                        }
                    }
                } else {
                    out.push((bible_id.clone(), book_id.clone(), target.clone()));
                    self.emit(Event::BibleBookCached {
                        bible_id: bible_id.clone(),
                        book_id: book_id.clone(),
                        path: target.clone(),
                    });
                }
                self.emit(Event::Progress {
                    step: format!("download_books_{bible_id}"),
                    current,
                    total,
                });
            }
        }
        Ok(out)
    }

    /// Full run that pipes parsed data to a DbSink so the orchestrator itself can build the DB.
    /// This will:
    ///  - load manifest
    ///  - cache crossrefs, bible manifests, books
    ///  - parse and call DbSink hooks to insert crossrefs, languages, bibles, headers, books, verses
    pub async fn run_with_sink(&self, selection: Selection, sink: impl DbSink) -> Result<()> {
        // Load manifest
        let (manifest, _manifest_path) = self.load_manifest().await?;

        // Save selection
        self.save_selection(&selection)?;

        // Cache crossrefs
        self.emit(Event::Message("Starting crossref caching".into()));
        let cross_paths = self.cache_crossrefs(&manifest).await?;
        // parse crossrefs and call sink
        for path in cross_paths {
            let book_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let text = std::fs::read_to_string(&path)?;
            if let Ok(parsed) = serde_json::from_str::<models::CrossReference>(&text) {
                // consumer decides how to insert the structure
                sink.insert_cross_reference(&book_id, &parsed).await?;
            } else {
                self.emit(Event::Message(
                    format!("Failed to parse crossref {path:?}",),
                ));
            }
        }

        // Cache bible manifests
        self.emit(Event::Message("Starting bible manifests caching".into()));
        let manifests = self
            .cache_bible_manifests(&manifest, &selection.bibles)
            .await?;

        // Cache bible books
        self.emit(Event::Message("Starting bible books caching".into()));
        let book_files = self.cache_bible_books(&manifests).await?;

        // Insert languages
        self.emit(Event::Message("Inserting languages via DbSink".into()));
        for lang in &selection.languages {
            if let Some(lang_obj) = manifest.get("languages").and_then(|v| v.get(lang)) {
                let direction = lang_obj
                    .get("direction")
                    .and_then(|v| v.as_str())
                    .unwrap_or("ltr");
                let local = lang_obj.get("local").and_then(|v| v.as_str()).unwrap_or("");
                let english = lang_obj
                    .get("english")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                sink.insert_language(lang, direction, local, english)
                    .await?;
            } else {
                sink.insert_language(lang, "ltr", "", "").await?;
            }
        }

        // For each bible, call insert_bible, insert headers, insert book metadata and verses
        for (bible_id, manifest_path) in &manifests {
            // manifest_bible info from global manifest if present
            let maybe_global = manifest
                .get("bibles")
                .and_then(|b| b.get(bible_id))
                .cloned();
            let (name_local, name_english, language_id) = if let Some(g) = maybe_global {
                let local = g
                    .get("name")
                    .and_then(|n| n.get("local"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                let english = g
                    .get("name")
                    .and_then(|n| n.get("english"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                let language = g
                    .get("language")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                (local, english, language)
            } else {
                ("".into(), "".into(), "".into())
            };

            sink.insert_bible(bible_id, &name_local, &name_english, &language_id)
                .await?;

            // parse local bible manifest for chapter_headings
            let text = std::fs::read_to_string(manifest_path)?;
            if let Ok(bv) = serde_json::from_str::<models::BibleVariant>(&text) {
                // chapter headings
                for (book, chs) in bv.chapter_headings.iter() {
                    for (i, header) in chs.iter().enumerate() {
                        if header.is_empty() {
                            continue;
                        }
                        sink.insert_header(bible_id, book, i, header).await?;
                    }
                }
            }

            // insert books/verses for this bible by scanning book_files
            for (bb_id, book_id, path) in book_files.iter().filter(|t| &t.0 == bible_id) {
                let data = std::fs::read_to_string(path)?;
                if let Ok(book_obj) = serde_json::from_str::<models::Book>(&data) {
                    // insert book meta
                    sink.insert_book_meta(
                        bible_id,
                        &book_id,
                        &book_obj.name.normal,
                        &book_obj.name.long,
                        &book_obj.name.abbrev,
                    )
                    .await?;
                    // iterate contents: contents[chapter][verse] -> Vec<Content>
                    for (chapter_idx, chapter) in book_obj.contents.iter().enumerate() {
                        for (verse_idx, verse_contents) in chapter.iter().enumerate() {
                            // combine Raw content segments into a simple text for DB insertion
                            // original code inserted notes as separate rows; here we only insert plain verse text for simplicity
                            for c in verse_contents {
                                match c {
                                    models::Content::Raw(s) => {
                                        sink.insert_verse(&book_id, chapter_idx, verse_idx, s)
                                            .await?;
                                    }
                                    models::Content::Note { contents, .. } => {
                                        // Optionally insert notes as verses or separate table; keep it as verse insertion for demo
                                        sink.insert_note(
                                            &book_id,
                                            chapter_idx,
                                            verse_idx,
                                            contents,
                                        )
                                        .await?;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                } else {
                    self.emit(Event::Message(format!(
                        "Failed to parse book file {}",
                        path.display()
                    )));
                }
            }
        }

        // finalize sink (commit or vacuum etc)
        sink.finalize().await?;

        self.emit(Event::Completed);
        Ok(())
    }
}
