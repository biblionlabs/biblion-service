// Example CLI using setup_core and implementing DbSink using the repo's `db` crate.
// This file replaces the previous main.rs and demonstrates how to:
//  - show language/bible lists to user
//  - add an extra bible with a books URL template
//  - build a Selection and pass a DbSink to run_with_sink

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use db::Crud;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use inquire::list_option::ListOption;
use inquire::validator::Validation;
use inquire::{Confirm, MultiSelect};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use reqwest::Client;

use setup_core::{CrossReference, DbSink, Event, Reference, Selection, SetupBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=======================================");
    println!("     Welcome to Bible Setup CLI (with DbSink)");
    println!("=======================================\n");

    let mp = Arc::new(MultiProgress::new());
    let bars: Arc<Mutex<HashMap<String, ProgressBar>>> = Arc::new(Mutex::new(HashMap::new()));

    let event_cb = {
        let mp = mp.clone();
        let bars = bars.clone();
        move |ev: Event| {
            match ev {
                Event::Message(msg) => {
                    // Mensaje informativo rápido (se muestra y se limpia)
                    let pb = mp.add(ProgressBar::new_spinner());
                    let style = ProgressStyle::with_template("{spinner} {msg}")
                        .unwrap()
                        .tick_chars("⠋⠙⠹⠸⠼⠴");
                    pb.set_style(style);
                    pb.set_message(msg);
                    // dejamos tick por un instante y luego limpiamos
                    pb.enable_steady_tick(Duration::from_millis(80));
                    pb.finish_and_clear();
                }

                Event::Progress {
                    step,
                    current,
                    total,
                } => {
                    // Barra por "step" con longitud conocida
                    let key = format!("progress:{}", step);
                    let mut map = bars.lock().unwrap();
                    let pb = map.entry(key.clone()).or_insert_with(|| {
                    let p = mp.add(ProgressBar::new(total));
                    p.set_style(
                        ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} {msg}")
                            .unwrap()
                            .progress_chars("=> "),
                    );
                    p.set_message(step.clone());
                    p
                });
                    pb.set_length(total);
                    pb.set_position(current);
                    if current >= total {
                        pb.finish_with_message(format!("{step} complete"));
                        map.remove(&key);
                    }
                }

                Event::ManifestReady { path, .. } => {
                    let pb = mp.add(ProgressBar::new_spinner());
                    let style = ProgressStyle::with_template("{spinner} {msg}")
                        .unwrap()
                        .tick_chars("⠋⠙⠹⠸⠼⠴");
                    pb.set_style(style);
                    pb.set_message(format!("Manifest ready: {}", path.display()));
                    pb.finish_with_message("Manifest cached");
                }

                Event::CrossRefCached { book_id, path } => {
                    let pb = mp.add(ProgressBar::new_spinner());
                    pb.set_style(
                        ProgressStyle::with_template("{spinner} {msg}")
                            .unwrap()
                            .tick_chars("⠋⠙⠹"),
                    );
                    pb.set_message(format!(
                        "CrossRef cached: {} -> {}",
                        book_id,
                        path.display()
                    ));
                    pb.finish_and_clear();
                }

                Event::BibleManifestCached { bible_id, path } => {
                    let pb = mp.add(ProgressBar::new_spinner());
                    pb.set_style(
                        ProgressStyle::with_template("{spinner} {msg}")
                            .unwrap()
                            .tick_chars("⠋⠙⠹"),
                    );
                    pb.set_message(format!(
                        "Bible manifest: {} -> {}",
                        bible_id,
                        path.display()
                    ));
                    pb.finish_and_clear();
                }

                Event::BibleBookCached {
                    bible_id,
                    book_id,
                    path: _,
                } => {
                    // Barra acumulativa por Biblia (cuenta libros descargados)
                    let key = format!("books:{}", bible_id);
                    let mut map = bars.lock().unwrap();
                    let pb = map.entry(key.clone()).or_insert_with(|| {
                        let p = mp.add(ProgressBar::new(0)); // longitud desconocida al principio
                        p.set_style(
                            ProgressStyle::with_template("{spinner} {msg}")
                                .unwrap()
                                .tick_chars("⠋⠙⠹"),
                        );
                        p.set_message(format!("Downloading books for {bible_id}"));
                        p.enable_steady_tick(Duration::from_millis(80));
                        p
                    });
                    // incrementamos contador (si queremos longitud conocida, el event Progress la manejará)
                    pb.inc(1);
                    pb.set_message(format!("{bible_id}: {book_id}"));
                }

                Event::SelectionSaved { path } => {
                    let pb = mp.add(ProgressBar::new_spinner());
                    pb.set_style(
                        ProgressStyle::with_template("{spinner} {msg}")
                            .unwrap()
                            .tick_chars("⠋⠙"),
                    );
                    pb.set_message(format!("Selection saved: {}", path.display()));
                    pb.finish_and_clear();
                }

                Event::Completed => {
                    // final; cerramos todo con mensaje
                    let finish_pb = mp.add(ProgressBar::new_spinner());
                    finish_pb.set_style(
                        ProgressStyle::with_template("{spinner} {msg}")
                            .unwrap()
                            .tick_chars("⠁⠂⠄"),
                    );
                    finish_pb.set_message("Setup completed successfully");
                    finish_pb.finish_with_message("Done");
                }

                Event::Error(e) => {
                    // errores van a stderr y también mostramos con indicatif
                    eprintln!("Setup error: {}", e);
                    let pb = mp.add(ProgressBar::new_spinner());
                    pb.set_style(
                        ProgressStyle::with_template("{spinner} {msg}")
                            .unwrap()
                            .tick_chars("!!"),
                    );
                    pb.set_message(format!("Error: {}", e));
                    pb.finish_and_clear();
                }
            }
        }
    };

    // Build Setup orchestrator
    let (_client, setup) = SetupBuilder::new()
        .cache_path(".cache")
        // Add Reina Valera 1960 Bible
        .add_bible_from_url("spa_rv1960", "https://raw.githubusercontent.com/biblionlabs/extra_data_source/refs/heads/main/bibles/spa_rv1960/manifest.json", "https://raw.githubusercontent.com/biblionlabs/extra_data_source/refs/heads/main/bibles/spa_rv1960/desc.json", Some("https://raw.githubusercontent.com/biblionlabs/extra_data_source/refs/heads/main/bibles/{bible_id}/books/{book}.json"))
        .on_event(event_cb)
        .build();

    // Example: fetch lists for UI prompts
    let languages = setup.list_languages().await?;
    let language_options = languages
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

    // list bibles
    let bibles = setup.list_bibles().await?;
    let bible_options = bibles
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
        .map(|opt| languages[opt.index].0.clone())
        .collect();

    let mut selected_bible_ids: Vec<String> = selected_bible_options
        .iter()
        .map(|opt| bibles[opt.index].0.clone())
        .collect();

    let mut originals_added: Vec<String> = Vec::new();

    if include_originals {
        for (code, eng, loc) in &languages {
            let low_eng = eng.to_lowercase();
            let low_loc = loc.to_lowercase();
            if (code == "grc"
                || code == "hbo"
                || code == "heb"
                || low_eng.contains("greek")
                || low_eng.contains("hebrew")
                || low_loc.contains("greek")
                || low_loc.contains("hebrew"))
                && !selected_lang_codes.contains(code)
            {
                selected_lang_codes.push(code.clone());
                originals_added.push(code.clone());
            }
        }
        selected_bible_ids.extend_from_slice(&[
            "grc_bre".into(),
            "grc_sr".into(),
            "grc_sbl".into(),
            "grc_tr".into(),
            "grc_rp".into(),
            "cop_shc".into(),
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
        if let Some((_, eng, loc)) = languages.iter().find(|(k, _, _)| k == code) {
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
        if let Some((_, local, english, lang)) = bibles.iter().find(|(k, _, _, _)| k == id) {
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
    let selection = Selection {
        languages: selected_lang_codes.clone(),
        bibles: selected_bible_ids.clone(),
        originals_added: originals_added.clone(),
        include_originals,
    };

    // Create DB sink and run the orchestrator
    let sink = RepoDbSink::from("bible.db".to_string());
    setup.run_with_sink(selection, sink).await.map_err(|e| {
        println!("Setup failed: {e}");
        e
    })?;

    println!("\nSetup completed. Cached files are in .cache and DB created at bible.db");

    Ok(())
}
