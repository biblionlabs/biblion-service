//! Core orchestrator crate for download/cache and DB-orchestration via a DbSink trait.
//!
//! Main points:
//! - SetupBuilder to configure cache, db_path and extra bibles (manifest url + optional books URL template).
//! - Setup exposes helpers to list languages and bibles from the main manifest so frontends can prompt the user.
//! - DbSink trait: implement this in the consumer to have the orchestrator create the database using cached files.

#![warn(async_fn_in_trait)]

use reqwest::blocking::Client;
use serde_json::Value;
use service_db::IndexedVerse;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

mod builder;
mod db;
mod error;
mod models;

pub use service_db;
pub mod event;

pub use builder::*;
pub use db::*;
pub use error::{Result, Setup as Error};
pub use models::*;

pub type Callback<Args> = Box<dyn Fn(Args) + Send + Sync>;

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
    callbacks: Arc<HashMap<TypeId, Box<dyn Any + Send + Sync>>>,
    manifest_ttl: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BibleInstallStatus {
    NotInstalled,
    Partial {
        installed_books: usize,
        total_books: usize,
        installed_verses: usize,
        total_verses: usize,
    },
    Complete {
        total_books: usize,
        total_verses: usize,
    },
}

impl BibleInstallStatus {
    pub fn is_complete(&self) -> bool {
        matches!(self, BibleInstallStatus::Complete { .. })
    }

    pub fn is_partial(&self) -> bool {
        matches!(self, BibleInstallStatus::Partial { .. })
    }

    pub fn is_not_installed(&self) -> bool {
        matches!(self, BibleInstallStatus::NotInstalled)
    }

    /// 0.0 - 100.0
    pub fn completion_percentage(&self) -> f32 {
        match self {
            BibleInstallStatus::NotInstalled => 0.0,
            BibleInstallStatus::Complete { .. } => 100.0,
            BibleInstallStatus::Partial {
                installed_verses,
                total_verses,
                ..
            } => {
                if *total_verses == 0 {
                    0.0
                } else {
                    (*installed_verses as f32 / *total_verses as f32) * 100.0
                }
            }
        }
    }
}

impl std::fmt::Display for BibleInstallStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BibleInstallStatus::NotInstalled => write!(f, ""),
            BibleInstallStatus::Partial {
                installed_books,
                total_books,
                installed_verses,
                total_verses,
            } => write!(
                f,
                "[{installed_books}/{total_books} - {installed_verses}/{total_verses}]"
            ),
            BibleInstallStatus::Complete { .. } => write!(f, "[Installed]"),
        }
    }
}

impl Setup {
    fn emit<E: event::Event>(&self, args: E::Args) {
        let type_id = TypeId::of::<E>();
        let Some(callback) = self.callbacks.get(&type_id) else {
            return;
        };
        let Some(cb) = callback.downcast_ref::<Callback<E::Args>>() else {
            return;
        };
        cb(args);
    }

    /// returns manifest Value and path
    pub fn load_manifest(&self) -> Result<(Value, PathBuf)> {
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
            self.emit::<event::Message>("Downloading manifest...".into());
            let text = self
                .client
                .get("https://v1.fetch.bible/manifest.json")
                .send()?
                .text()?;
            serde_json::from_str(&text)?
        } else {
            self.emit::<event::Message>("Using cached manifest".into());
            let bytes = std::fs::read(&manifest_path)?;
            serde_json::from_slice(&bytes)?
        };

        // Merge extra bible manifests provided via builder into the main manifest `v`.
        // Each extra manifest is expected to have the same top-level structure (e.g. "bibles", "languages", etc.).
        // We try to fetch them from their manifest_url and merge; failures are logged as messages but do not abort.
        for extra in &self.extra_bibles {
            let url = &extra.desc_url;
            self.emit::<event::Message>(format!(
                "Attempting to fetch extra manifest for {} from {url}",
                extra.id
            ));
            match self.client.get(url).send() {
                Ok(resp) if resp.status().is_success() => match resp.text() {
                    Ok(text) => match serde_json::from_str::<Value>(&text) {
                        Ok(extra_val) => {
                            let new_bible = v.get_mut("bibles").unwrap().as_object_mut().unwrap();
                            if !new_bible.contains_key(&extra.id) {
                                if new_bible.insert(extra.id.to_string(), extra_val).is_none() {
                                    self.emit::<event::Message>(format!(
                                        "Merged extra manifest for {}",
                                        extra.id
                                    ));
                                } else {
                                    self.emit::<event::Message>(format!(
                                        "Merged extra manifest for {} failed!",
                                        extra.id
                                    ));
                                }
                            }
                        }
                        Err(e) => {
                            self.emit::<event::Message>(format!(
                                "Failed to parse extra manifest for {}: {e}",
                                extra.id
                            ));
                        }
                    },
                    Err(e) => {
                        self.emit::<event::Message>(format!(
                            "Failed to read extra manifest response for {}: {e}",
                            extra.id
                        ));
                    }
                },
                _ => {
                    self.emit::<event::Message>(format!(
                        "Failed to fetch extra manifest for {} (url: {url}), skipping",
                        extra.id
                    ));
                }
            }
        }

        // Persist merged manifest back to cache path (either newly downloaded or updated cached)
        let serialized = serde_json::to_string_pretty(&v)?;
        std::fs::write(&manifest_path, serialized.as_bytes())?;

        // Emit readiness event with merged manifest
        self.emit::<event::ManifestReady>((v.clone(), manifest_path.clone()));

        Ok((v, manifest_path))
    }

    /// helpers for UI: list languages (code, english, local)
    pub fn list_languages(&self) -> Result<Vec<(String, String, String)>> {
        let (manifest, _) = self.load_manifest()?;
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

    /// helpers for UI: list bibles (id, local, english, language, status)
    pub fn list_bibles(
        &self,
        sink: &impl DbSink,
    ) -> Result<Vec<(String, String, String, String, BibleInstallStatus)>> {
        let (manifest, _) = self.load_manifest()?;
        let bibles_obj = manifest
            .get("bibles")
            .and_then(|v| v.as_object())
            .ok_or(Error::MissingField("manifest", "bibles"))?;

        let mut bible_entries: Vec<(String, String, String, String, BibleInstallStatus)> =
            Vec::new();

        for (bible_id, bible_value) in bibles_obj.iter() {
            let local = bible_value
                .get("name")
                .and_then(|n| n.get("local"))
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_string();
            let english = bible_value
                .get("name")
                .and_then(|n| n.get("english"))
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_string();
            let language = bible_value
                .get("language")
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_string();

            // Calcular estado de instalación
            let status = self.get_bible_status(sink, bible_id)?;

            bible_entries.push((bible_id.clone(), local, english, language, status));
        }

        bible_entries.sort_by(|a, b| a.2.cmp(&b.2));
        Ok(bible_entries)
    }

    fn get_bible_status(&self, sink: &impl DbSink, bible_id: &str) -> Result<BibleInstallStatus> {
        let bible_manifest_path = self
            .cache_path
            .join("bibles")
            .join(bible_id)
            .join("manifest.json");

        if !bible_manifest_path.exists() {
            return Ok(BibleInstallStatus::NotInstalled);
        }

        let text = std::fs::read_to_string(&bible_manifest_path)?;
        let Ok(bv) = serde_json::from_str::<models::BibleVariant>(&text) else {
            return Ok(BibleInstallStatus::NotInstalled);
        };

        let expected_book_ids: Vec<String> = bv.book_names.keys().cloned().collect();
        if expected_book_ids.is_empty() {
            return Ok(BibleInstallStatus::NotInstalled);
        }

        let books_dir = bible_manifest_path.parent().unwrap().join("books");
        let mut expected_books_with_verses: Vec<(String, usize)> = Vec::new();
        let mut total_expected_verses = 0usize;

        for book_id in &expected_book_ids {
            let book_path = books_dir.join(format!("{}.json", book_id));

            if !book_path.exists() {
                // Libro no descargado = no instalada
                return Ok(BibleInstallStatus::NotInstalled);
            }

            let data = std::fs::read_to_string(&book_path)?;
            let Ok(book_obj) = serde_json::from_str::<models::Book>(&data) else {
                return Ok(BibleInstallStatus::NotInstalled);
            };

            let verse_count = book_obj
                .contents
                .iter()
                .map(|chapter| chapter.iter().filter(|v| !v.is_empty()).count())
                .sum::<usize>();

            expected_books_with_verses.push((book_id.clone(), verse_count));
            total_expected_verses += verse_count;
        }

        let install_status = sink.get_bible_install_stats(bible_id, &expected_books_with_verses)?;

        Ok(install_status)
    }

    /// Save selection
    pub fn save_selection(&self, sel: &Selection) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.cache_path)?;
        let path = self.cache_path.join("selection.json");
        let bytes = serde_json::to_vec_pretty(sel)?;
        std::fs::write(&path, &bytes)?;
        self.emit::<event::SelectionSaved>(path.clone());
        Ok(path)
    }

    /// cache crossrefs (returns paths)
    pub fn cache_crossrefs(&self, manifest: &Value) -> Result<Vec<PathBuf>> {
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
                {
                    Ok(resp) if resp.status().is_success() => {
                        let text = resp.text()?;
                        std::fs::write(&target, text.as_bytes())?;
                        self.emit::<event::CrossRefCached>((book_id.clone(), target.clone()));
                        saved.push(target);
                    }
                    _ => {
                        self.emit::<event::Message>(format!(
                            "Skipping crossref {book_id} (network failed, no cache)"
                        ));
                    }
                }
            } else {
                self.emit::<event::CrossRefCached>((book_id.clone(), target.clone()));
                saved.push(target);
            }
            self.emit::<event::Progress>(("crossrefs".to_string(), current, total));
        }
        Ok(saved)
    }

    /// Cache bible manifests (including extra manifests provided via extra_bibles)
    /// returns Vec<(bible_id, manifest_path)>
    pub fn cache_bible_manifests(
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
                    self.emit::<event::Message>(format!(
                    "Bible id {bible_id} not present in global manifest; treating as external or using provided sources"
                ));
                }
            }

            // First prefer explicit extra_bibles configured via builder
            if let Some(extra) = extra_map.get(bible_id) {
                // attempt to fetch from provided manifest URL
                self.emit::<event::Message>(format!(
                    "Downloading extra bible manifest for {bible_id} from {}",
                    extra.manifest_url
                ));
                match self.client.get(&extra.manifest_url).send() {
                    Ok(resp) if resp.status().is_success() => {
                        let text = resp.text()?;
                        std::fs::write(&manifest_path, text.as_bytes())?;
                        results.push((bible_id.clone(), manifest_path.clone()));
                        self.emit::<event::BibleManifestCached>((
                            bible_id.clone(),
                            manifest_path.clone(),
                        ));
                        continue;
                    }
                    _ => {
                        self.emit::<event::Message>(format!(
                        "Failed to fetch extra manifest for {bible_id}, will try default endpoint or cached file"
                    ));
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
                self.emit::<event::Message>(format!(
                "Downloading bible manifest for {bible_id} from manifest URL declared in global manifest: {url}"
            ));
                match self.client.get(&url).send() {
                    Ok(resp) if resp.status().is_success() => {
                        let text = resp.text()?;
                        std::fs::write(&manifest_path, text.as_bytes())?;
                        results.push((bible_id.clone(), manifest_path.clone()));
                        self.emit::<event::BibleManifestCached>((
                            bible_id.clone(),
                            manifest_path.clone(),
                        ));
                        continue;
                    }
                    _ => {
                        self.emit::<event::Message>(format!(
                        "Failed to fetch manifest for {bible_id} from declared URL; will try default endpoint or cached file"
                    ));
                        // fallthrough
                    }
                }
            }

            // default behaviour: attempt to fetch from fetch.bible endpoint
            self.emit::<event::Message>(format!(
                "Fetching manifest for {bible_id} from fetch.bible"
            ));
            match self
                .client
                .get(format!(
                    "https://v1.fetch.bible/bibles/{bible_id}/extra.json"
                ))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let text = resp.text()?;
                    std::fs::write(&manifest_path, text.as_bytes())?;
                    results.push((bible_id.clone(), manifest_path.clone()));
                    self.emit::<event::BibleManifestCached>((
                        bible_id.clone(),
                        manifest_path.clone(),
                    ));
                }
                _ => {
                    if manifest_path.exists() {
                        self.emit::<event::Message>(format!(
                            "Using cached manifest for {bible_id}"
                        ));
                        results.push((bible_id.clone(), manifest_path.clone()));
                        self.emit::<event::BibleManifestCached>((
                            bible_id.clone(),
                            manifest_path.clone(),
                        ));
                    } else {
                        self.emit::<event::Message>(format!(
                            "Failed to fetch manifest for {bible_id} and no cache present"
                        ));
                    }
                }
            }
        }
        Ok(results)
    }

    /// Cache books for bibles; uses manifest to find book ids, and falls back to extra book_url_template when manifest doesn't provide file content.
    /// returns tuples (bible_id, book_id, path)
    pub fn cache_bible_books(
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

            let total = (books.len() + 1) as u64;
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
                        match self.client.get(&url).send() {
                            Ok(resp) if resp.status().is_success() => {
                                let text = resp.text()?;
                                std::fs::write(&target, text.as_bytes())?;
                                out.push((bible_id.clone(), book_id.clone(), target.clone()));
                                self.emit::<event::BibleBookCached>((
                                    bible_id.clone(),
                                    book_id.clone(),
                                    target.clone(),
                                ));
                            }
                            _ => {
                                self.emit::<event::Message>(format!(
                                    "Skipped book {book_id} of {bible_id} (fetch failed at template)"
                                ));
                            }
                        }
                    } else {
                        // fallback to fetch.bible default endpoint
                        let url =
                            format!("https://v1.fetch.bible/bibles/{bible_id}/txt/{book_id}.json");
                        match self.client.get(&url).send() {
                            Ok(resp) if resp.status().is_success() => {
                                let text = resp.text()?;
                                std::fs::write(&target, text.as_bytes())?;
                                out.push((bible_id.clone(), book_id.clone(), target.clone()));
                                self.emit::<event::BibleBookCached>((
                                    bible_id.clone(),
                                    book_id.clone(),
                                    target.clone(),
                                ));
                            }
                            _ => {
                                self.emit::<event::Message>(format!(
                                    "Skipped book {book_id} of {bible_id} (fetch failed)"
                                ));
                            }
                        }
                    }
                } else {
                    out.push((bible_id.clone(), book_id.clone(), target.clone()));
                    self.emit::<event::BibleBookCached>((
                        bible_id.clone(),
                        book_id.clone(),
                        target.clone(),
                    ));
                }
                self.emit::<event::Progress>((bible_id.clone(), current, total));
            }
        }
        Ok(out)
    }

    pub fn install_cross(&self, sink: &impl DbSink) -> Result<()> {
        let (manifest, _manifest_path) = self.load_manifest()?;

        let expected_books = manifest
            .get("book_names_english")
            .and_then(|v| v.as_object())
            .map(|o| o.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        if sink.has_cross_references(&expected_books).unwrap_or(false) {
            return Ok(());
        }

        self.emit::<event::Message>("Starting crossref caching".into());
        let cross_paths = self.cache_crossrefs(&manifest)?;

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
                sink.insert_cross_reference(&book_id, &parsed)?;
            } else {
                self.emit::<event::Message>(format!("Failed to parse crossref {path:?}"));
            }
        }

        Ok(())
    }

    pub fn install_bibles(&self, sink: &impl DbSink, bible_ids: &[String]) -> Result<()> {
        // Load manifest
        let (manifest, _manifest_path) = self.load_manifest()?;

        // Cache bible manifests
        self.emit::<event::Message>("Starting bible manifests caching".into());
        let manifests = self.cache_bible_manifests(&manifest, bible_ids)?;

        // Cache bible books
        self.emit::<event::Message>("Starting bible books caching".into());
        let book_files = self.cache_bible_books(&manifests)?;

        let mut bibles_to_install = Vec::new();

        for (bible_id, manifest_path) in &manifests {
            let text = std::fs::read_to_string(manifest_path)?;
            let Ok(bv) = serde_json::from_str::<models::BibleVariant>(&text) else {
                self.emit::<event::Message>(format!(
                    "Failed to parse manifest for {bible_id}, will install"
                ));
                bibles_to_install.push(bible_id.clone());
                continue;
            };

            let mut expected_books_with_verses: Vec<(String, usize)> = Vec::new();

            for book_id in bv.book_names.keys() {
                let book_path = book_files
                    .iter()
                    .find(|(bid, bkid, _)| bid == bible_id && bkid == book_id)
                    .map(|(_, _, path)| path);

                let Some(path) = book_path else {
                    self.emit::<event::Message>(format!(
                        "Book {book_id} not found in cache for {bible_id}, will install"
                    ));
                    bibles_to_install.push(bible_id.clone());
                    break;
                };

                let data = std::fs::read_to_string(path)?;
                let Ok(book_obj) = serde_json::from_str::<models::Book>(&data) else {
                    self.emit::<event::Message>(format!(
                        "Failed to parse book {book_id} for {bible_id}, will install"
                    ));
                    bibles_to_install.push(bible_id.clone());
                    break;
                };

                let verse_count = book_obj
                    .contents
                    .iter()
                    .map(|chapter| chapter.iter().filter(|verse| !verse.is_empty()).count())
                    .sum::<usize>();

                expected_books_with_verses.push((book_id.clone(), verse_count));
            }

            if !bibles_to_install.contains(bible_id) {
                if expected_books_with_verses.is_empty() {
                    self.emit::<event::Message>(format!("No books found for {bible_id}, skipping"));
                    continue;
                }

                let is_complete = sink
                    .is_bible_installed(bible_id, &expected_books_with_verses)
                    .unwrap_or(false);

                if !is_complete {
                    self.emit::<event::Message>(format!(
                        "Bible {bible_id} incomplete or not installed, will install"
                    ));
                    bibles_to_install.push(bible_id.clone());
                }
            }
        }

        if bibles_to_install.is_empty() {
            return Ok(());
        }

        self.emit::<event::Message>(format!(
            "Installing {} bible(s)...",
            bibles_to_install.len()
        ));

        for (bible_id, manifest_path) in &manifests {
            // Solo instalar si está en la lista
            if !bibles_to_install.contains(bible_id) {
                continue;
            }

            let mut verses_indexed = Vec::new();

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
                    .or_else(|| bible_id.split('_').next())
                    .unwrap_or_default()
                    .to_string();
                (local, english, language)
            } else {
                (String::new(), String::new(), String::new())
            };

            sink.insert_bible(bible_id, &name_local, &name_english, &language_id)?;

            let text = std::fs::read_to_string(manifest_path)?;
            if let Ok(bv) = serde_json::from_str::<models::BibleVariant>(&text) {
                let total = (bv.chapter_headings.len() + 1) as u64;
                let mut current = 0u64;

                for (book, chs) in bv.chapter_headings.iter() {
                    current += 1;
                    for (i, header) in chs.iter().enumerate() {
                        if !header.is_empty() {
                            sink.insert_header(bible_id, book, i, header)?;
                        }
                    }
                    self.emit::<event::Progress>((bible_id.clone(), current, total));
                }
                self.emit::<event::Progress>((bible_id.clone(), current, total));
            }

            let books: Vec<_> = book_files
                .iter()
                .filter(|(bid, _, _)| bid == bible_id)
                .collect();

            let total = books.len() as u64;
            let mut current = 0u64;

            let bible_name = if name_local.is_empty() {
                &name_english
            } else {
                &name_local
            };

            for (_, book_id, path) in books {
                current += 1;

                let data = std::fs::read_to_string(path)?;
                let Ok(book_obj) = serde_json::from_str::<models::Book>(&data) else {
                    self.emit::<event::Message>(format!(
                        "Failed to parse book file {}",
                        path.display()
                    ));
                    continue;
                };

                sink.insert_book_meta(
                    bible_id,
                    book_id,
                    &book_obj.name.normal,
                    &book_obj.name.long,
                    &book_obj.name.abbrev,
                )?;

                let total_chapters = book_obj.contents.len() as u64;
                let mut current_chapter = 0u64;

                for (chapter_idx, chapter) in book_obj.contents.iter().enumerate() {
                    current_chapter += 1;

                    for (verse_idx, verse_contents) in chapter.iter().enumerate() {
                        for content in verse_contents {
                            match content {
                                models::Content::Raw(s) => {
                                    verses_indexed.push(IndexedVerse {
                                        bible_id: bible_id.clone(),
                                        bible_name: bible_name.clone(),
                                        book_id: book_id.clone(),
                                        book_name: book_obj.name.long.clone(),
                                        chapter: chapter_idx as _,
                                        verse: verse_idx as _,
                                        text: s.clone(),
                                    });
                                    sink.insert_verse(book_id, chapter_idx, verse_idx, s)?;
                                }
                                models::Content::Note { contents, .. } => {
                                    sink.insert_note(book_id, chapter_idx, verse_idx, contents)?;
                                }
                                _ => {}
                            }
                        }
                    }

                    self.emit::<event::Progress>((
                        format!("{bible_id}_{book_id}"),
                        current_chapter,
                        total_chapters,
                    ));
                }

                self.emit::<event::Progress>((bible_id.clone(), current, total));
            }

            self.emit::<event::Message>(format!("Indexing {bible_id} verses ({})...", verses_indexed.len()));
            sink.index_bible_verses(verses_indexed)?;

            self.emit::<event::Progress>((bible_id.clone(), current, total));
        }

        Ok(())
    }

    pub fn install_langs(&self, sink: &impl DbSink, languages: &[String]) -> Result<()> {
        if sink.has_languages(languages).unwrap_or(false) {
            return Ok(());
        }

        // Load manifest
        let (manifest, _manifest_path) = self.load_manifest()?;

        // Insert languages
        self.emit::<event::Message>("Inserting languages via DbSink".into());
        for lang in languages {
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
                sink.insert_language(lang, direction, local, english)?;
            } else {
                sink.insert_language(lang, "ltr", "", "")?;
            }
        }

        Ok(())
    }

    /// Full run that pipes parsed data to a DbSink so the orchestrator itself can build the DB.
    /// This will:
    ///  - load manifest
    ///  - cache crossrefs, bible manifests, books
    ///  - parse and call DbSink hooks to insert crossrefs, languages, bibles, headers, books, verses
    pub fn run_with_sink(&self, selection: Selection, sink: &impl DbSink) -> Result<()> {
        // Save selection
        self.save_selection(&selection)?;

        self.install_cross(sink)?;
        self.install_langs(sink, &selection.languages)?;
        self.install_bibles(sink, &selection.bibles)?;

        // finalize sink (commit or vacuum etc)
        sink.finalize()?;

        self.emit::<event::Completed>(());
        Ok(())
    }
}
