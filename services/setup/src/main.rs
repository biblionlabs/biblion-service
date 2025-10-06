use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, SystemTime};

use indicatif::{ProgressBar, ProgressStyle};
use inquire::list_option::ListOption;
use inquire::validator::Validation;
use inquire::{Confirm, MultiSelect};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use reqwest::Client;
use serde_json::Value;

mod models;

use models::{BibleVariant, Book, CrossReference};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=======================================");
    println!("     Welcome to Bible Setup CLI");
    println!("=======================================\n");

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("bible-downloader/0.1 (rust)")
        .build()?;

    let cache_path = ".cache/manifest.json";
    let need_download = if Path::new(cache_path).exists() {
        match fs::metadata(cache_path)?.modified() {
            Ok(modified) => match SystemTime::now().duration_since(modified) {
                Ok(elapsed) => elapsed > Duration::from_secs(60 * 60 * 24 * 30),
                Err(_) => true,
            },
            Err(_) => true,
        }
    } else {
        true
    };

    let manifest: Value = if need_download {
        let pb = ProgressBar::new_spinner();
        let style = ProgressStyle::with_template("{spinner} {msg}")
            .unwrap()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");
        pb.set_style(style);
        pb.set_message("Downloading manifest...");
        pb.enable_steady_tick(Duration::from_millis(120));

        let content = client
            .get("https://v1.fetch.bible/manifest.json")
            .send()
            .await?
            .text()
            .await?;

        pb.finish_with_message("Manifest downloaded");

        fs::create_dir_all(".cache")?;
        fs::write(cache_path, content.as_bytes())?;

        serde_json::from_str(&content)?
    } else {
        let content = fs::read(cache_path)?;
        serde_json::from_slice(&content)?
    };

    let original_books = manifest
        .get("book_names_english")
        .ok_or("manifest missing book names")?
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<String>>();
    let languages_obj = manifest
        .get("languages")
        .and_then(|v| v.as_object())
        .ok_or("manifest missing languages")?;
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

    let language_options = language_entries
        .iter()
        .enumerate()
        .map(|(i, (_, eng, loc))| ListOption::new(i, format!("{eng} ({loc})")))
        .collect::<Vec<_>>();

    let selected_lang_options =
        MultiSelect::new("Select languages to install", language_options.clone())
            .with_validator(|a: &[ListOption<&ListOption<String>>]| {
                if a.is_empty() {
                    return Ok(Validation::Invalid(
                        "You must select at least one language.".into(),
                    ));
                }
                Ok(Validation::Valid)
            })
            .prompt()?;

    let bibles_obj = manifest
        .get("bibles")
        .and_then(|v| v.as_object())
        .ok_or("manifest missing bibles")?;
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

    let bible_options = bible_entries
        .iter()
        .enumerate()
        .map(|(i, (_, local, english, _))| {
            ListOption::new(
                i,
                format!(
                    "{} ({})",
                    if local.is_empty() { english } else { local },
                    english
                ),
            )
        })
        .collect::<Vec<_>>();

    let selected_bible_options =
        MultiSelect::new("Select Bible versions to install", bible_options.clone())
            .with_validator(|a: &[ListOption<&ListOption<String>>]| {
                if a.is_empty() {
                    return Ok(Validation::Invalid(
                        "You must select at least one Bible version.".into(),
                    ));
                }
                Ok(Validation::Valid)
            })
            .prompt()?;

    let include_originals = Confirm::new("Include original languages?")
        .with_default(true)
        .with_help_message("Original languages (Hebrew/Greek) usually require more space but are useful for study.")
        .prompt()?;

    let mut selected_lang_codes: Vec<String> = selected_lang_options
        .iter()
        .map(|opt| language_entries[opt.index].0.clone())
        .collect();

    let mut selected_bible_ids: Vec<String> = selected_bible_options
        .iter()
        .map(|opt| bible_entries[opt.index].0.clone())
        .collect();

    let mut originals_added: Vec<String> = Vec::new();

    if include_originals {
        for (code, eng, loc) in &language_entries {
            let low_eng = eng.to_lowercase();
            let low_loc = loc.to_lowercase();
            if code == "grc"
                || code == "hbo"
                || code == "heb"
                || low_eng.contains("greek")
                || low_eng.contains("hebrew")
                || low_loc.contains("greek")
                || low_loc.contains("hebrew")
            {
                if !selected_lang_codes.contains(code) {
                    selected_lang_codes.push(code.clone());
                    originals_added.push(code.clone());
                }
            }
        }
        selected_bible_ids.extend_from_slice(&[
            // Griego de la Septuaginta (LXX - AT)
            "grc_bre".into(), // Brenton's Septuagint Text (1870)
            // Griego del Nuevo Testamento
            "grc_sr".into(),  // Statistical Restoration Greek New Testament (2024)
            "grc_sbl".into(), // SBL Greek New Testament (2010)
            //
            // COMPLEMENTARIAS - Para análisis textual
            "grc_tr".into(), // Textus Receptus (1881)
            "grc_rp".into(), // Byzantine Textform (Robinson-Pierpont)
            //
            // VERSIONES ANTIGUAS IMPORTANTES
            "cop_shc".into(), // Coptic Sahidic New Testament (siglos II-IV)
        ]);
    }

    selected_lang_codes.sort();
    selected_lang_codes.dedup();
    originals_added.sort();
    originals_added.dedup();

    println!("\nSummary:");
    println!(
        "  Languages to install: {} ({} added as originals)",
        selected_lang_codes.len(),
        originals_added.len()
    );
    for code in &selected_lang_codes {
        if let Some((_, eng, loc)) = language_entries.iter().find(|(k, _, _)| k == code) {
            let mark = if originals_added.contains(code) {
                " (original)"
            } else {
                ""
            };
            println!("    - {eng} ({loc}){mark} ");
        } else {
            let mark = if originals_added.contains(code) {
                " (original)"
            } else {
                ""
            };
            println!("    - {mark}");
        }
    }
    println!(
        "\n  Bible versions to install: {}",
        selected_bible_ids.len()
    );
    for id in &selected_bible_ids {
        if let Some((_, local, english, lang)) = bible_entries.iter().find(|(k, _, _, _)| k == id) {
            println!(
                "    - {}{}",
                if local.is_empty() { english } else { local },
                if !lang.is_empty() {
                    format!(" ({lang})")
                } else {
                    String::default()
                }
            );
        } else {
            println!("    - {id}");
        }
    }

    let proceed = Confirm::new("Proceed with download and database creation?")
        .with_default(true)
        .prompt()?;

    if !proceed {
        println!("Setup cancelled.");
        return Ok(());
    }

    fs::create_dir_all(".cache")?;
    let selection = serde_json::json!({
        "languages": selected_lang_codes,
        "bibles": selected_bible_ids,
        "originals_added": originals_added,
        "include_originals": include_originals
    });
    fs::write(
        ".cache/selection.json",
        serde_json::to_vec_pretty(&selection)?,
    )?;

    let db_path = "bible.db";
    let current_handler = tokio::runtime::Handle::current();

    println!("Start setup Cross References");
    let cross_bible_dir = format!(".cache/cross");
    fs::create_dir_all(&cross_bible_dir)?;

    let pb = ProgressBar::new(original_books.len() as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} {msg}",
        )
        .unwrap()
        .progress_chars("=> "),
    );

    original_books.par_iter().for_each(|book_id| {
        current_handler.block_on(async {
            let cross_path = format!("{cross_bible_dir}/{book_id}.json");

            let data = if Path::new(&cross_path).exists() {
                fs::read_to_string(&cross_path).unwrap()
            } else {
                match client
                    .get(format!(
                        "https://v1.fetch.bible/crossref/large/{book_id}.json"
                    ))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        let text = resp.text().await.unwrap();
                        fs::write(&cross_path, &text).unwrap();
                        text
                    }
                    _ => {
                        pb.println(format!("  Skipped {book_id} (failed and no cache)"));
                        return;
                    }
                }
            };

            if let Err(e) = serde_json::from_str::<CrossReference>(&data) {
                pb.println(format!("  Error parsing {book_id}: {e}"));
            }

            pb.inc(1);
        });
    });
    pb.finish_with_message("Validate and cached Cross References");
    pb.reset();

    println!("Start setup Bible Sources");

    let c = AtomicU64::default();
    let selected_bible_ids = selected_bible_ids
        .par_iter()
        .flat_map(|bible_id| {
            current_handler.block_on(async {
                let bible_dir = format!(".cache/bibles/{bible_id}");
                let bible_manifest_path = format!(".cache/bibles/{bible_id}/manifest.json");
                fs::create_dir_all(&bible_dir).ok()?;

                println!("Start setup of {bible_id}");

                let bible_manifest: BibleVariant = match client
                    .get(&format!(
                        "https://v1.fetch.bible/bibles/{bible_id}/extra.json"
                    ))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        let text = resp.text().await.ok()?;
                        fs::create_dir_all(format!("{bible_dir}/books")).ok()?;
                        fs::write(&bible_manifest_path, &text).ok()?;
                        serde_json::from_str(&text).ok()?
                    }
                    _ => {
                        if Path::new(&bible_manifest_path).exists() {
                            println!("  Using cached manifest.json");
                            serde_json::from_str(&fs::read_to_string(&bible_manifest_path).ok()?)
                                .ok()?
                        } else {
                            println!("  Failed to fetch and no cache found for {bible_id}");
                            return None;
                        }
                    }
                };

                let books: Vec<String> = bible_manifest.book_names.keys().cloned().collect();
                c.fetch_add(books.len() as u64, std::sync::atomic::Ordering::SeqCst);

                Some((bible_id.clone(), books))
            })
        })
        .collect::<Vec<(String, Vec<_>)>>();

    selected_bible_ids.par_iter().for_each(|(bible_id, books)| {
        books.par_iter().for_each(|book| {
            current_handler.block_on(async {
                let bible_dir = format!(".cache/bibles/{bible_id}");
                pb.set_message(format!("Downloading {book}.json"));
                let book_url = format!("https://v1.fetch.bible/bibles/{bible_id}/txt/{book}.json");
                let book_path = format!("{bible_dir}/books/{book}.json");

                let data = if Path::new(&book_path).exists() {
                    fs::read_to_string(&book_path).unwrap()
                } else {
                    match client.get(&book_url).send().await {
                        Ok(resp) if resp.status().is_success() => {
                            let text = resp.text().await.unwrap();
                            fs::write(&book_path, &text).unwrap();
                            text
                        }
                        _ => {
                            pb.println(format!("  Skipped {book} (failed and no cache)"));
                            return;
                        }
                    }
                };

                match serde_json::from_str::<Book>(&data) {
                    Ok(parsed) => {
                        let chapters = parsed.contents.len();
                        pb.println(format!(
                            "  Parsed {}: {chapters} chapters loaded.",
                            parsed.name.long
                        ));
                    }
                    Err(e) => {
                        pb.println(format!("  Error parsing {book}: {e}"));
                    }
                }

                pb.inc(1);
            })
        });
    });
    pb.finish_with_message("Bible fully cached");

    println!("\nSetup completed. Database created at: {db_path}");
    println!("Saved selection to .cache/selection.json");
    Ok(())
}
