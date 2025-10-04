use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use indicatif::{ProgressBar, ProgressStyle};
use inquire::list_option::ListOption;
use inquire::validator::Validation;
use inquire::{Confirm, MultiSelect};
use reqwest::Client;
use serde_json::Value;

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

    let selected_bible_ids: Vec<String> = selected_bible_options
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
        for id in &selected_bible_ids {
            if let Some(bv) = bibles_obj.get(id) {
                if let Some(orig) = bv.get("original_language").and_then(|v| v.as_str()) {
                    if !selected_lang_codes.contains(&orig.to_string()) {
                        selected_lang_codes.push(orig.to_string());
                        originals_added.push(orig.to_string());
                    }
                }
            }
        }
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

    println!("\nSetup completed. Database created at: {db_path}");
    println!("Saved selection to .cache/selection.json");
    Ok(())
}
