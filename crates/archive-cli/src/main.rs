use std::path::PathBuf;

use anyhow::{Context, Result};
use archive_core::{
    ArchiveFormat, OperationReport, add_to_zip, create_7z, create_zip, extract_all,
    extract_selection, list_entries, remove_entries_from_zip, rename_entry_in_zip, test_archive,
};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "archive-cli", version, about = "Modularer Archivmanager")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    List {
        archive: PathBuf,
    },
    Extract {
        archive: PathBuf,
        /// Einzelne Einträge; ohne Angabe wird das gesamte Archiv entpackt.
        #[arg(long = "entry")]
        entries: Vec<String>,
        #[arg(long, default_value = ".")]
        output: PathBuf,
    },
    Create {
        archive: PathBuf,
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
    },
    Add {
        archive: PathBuf,
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
    },
    Rename {
        archive: PathBuf,
        /// Vorhandener Eintrag oder Verzeichnis.
        entry: String,
        /// Neuer Name innerhalb des Archivs.
        name: String,
    },
    Delete {
        archive: PathBuf,
        #[arg(required = true)]
        entries: Vec<String>,
    },
    Test {
        archive: PathBuf,
    },
    Info {
        archive: PathBuf,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::List { archive } => {
            for entry in list_entries(&archive)? {
                let kind = if entry.is_directory { "<DIR>" } else { "     " };
                println!("{kind} {:>12}  {}", entry.size, entry.name);
            }
        }
        Command::Extract {
            archive,
            entries,
            output,
        } => {
            let report = if entries.is_empty() {
                extract_all(&archive, &output)?
            } else {
                extract_selection(&archive, &entries, &output)?
            };
            finish_report(report, "Extrahiert")?;
        }
        Command::Create { archive, inputs } => {
            match archive.extension().and_then(|extension| extension.to_str()) {
                Some(extension) if extension.eq_ignore_ascii_case("7z") => {
                    create_7z(&archive, &inputs)?
                }
                Some(extension) if extension.eq_ignore_ascii_case("zip") => {
                    create_zip(&archive, &inputs)?
                }
                _ => anyhow::bail!("Zieldatei muss die Endung .zip oder .7z haben"),
            }
            println!("Erstellt: {}", archive.display());
        }
        Command::Add { archive, inputs } => {
            let report = add_to_zip(&archive, &inputs)?;
            println!(
                "Aktualisiert: {added} hinzugefügt, {replaced} ersetzt",
                added = report.added,
                replaced = report.replaced
            );
        }
        Command::Rename {
            archive,
            entry,
            name,
        } => {
            let report = rename_entry_in_zip(&archive, &entry, &name)?;
            println!("Umbenannt: {} Einträge", report.renamed);
        }
        Command::Delete { archive, entries } => {
            let report = remove_entries_from_zip(&archive, &entries)?;
            println!("Entfernt: {} Einträge", report.removed);
        }
        Command::Test { archive } => {
            let report = test_archive(&archive)?;
            finish_report(report, "Geprüft")?;
        }
        Command::Info { archive } => {
            let format = ArchiveFormat::detect(&archive)?;
            let entries = list_entries(&archive)?;
            let total: u64 = entries.iter().map(|entry| entry.size).sum();
            let compressed: u64 = entries
                .iter()
                .filter_map(|entry| entry.compressed_size)
                .sum();
            let bytes = std::fs::metadata(&archive)
                .with_context(|| format!("{} konnte nicht gelesen werden", archive.display()))?
                .len();
            println!("Archiv:       {}", archive.display());
            println!("Format:       {format}");
            println!("Dateigröße:   {bytes} B");
            println!("Einträge:     {}", entries.len());
            println!("Gesamtgröße:  {total} B");
            println!("Komprimiert:  {compressed} B");
        }
    }
    Ok(())
}

fn finish_report(report: OperationReport, verb: &str) -> Result<()> {
    if report.is_success() {
        println!("{verb}: {} Einträge", report.processed);
        return Ok(());
    }

    for failure in &report.failures {
        eprintln!("Fehler bei {}: {}", failure.name, failure.message);
    }
    anyhow::bail!(
        "{verb}: {} von {} Einträgen fehlgeschlagen",
        report.failures.len(),
        report.processed + report.failures.len()
    )
}
