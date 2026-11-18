use std::path::PathBuf;

use anyhow::Result;
use archive_core::{create_zip, extract_entry, list_entries};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "archive-cli", version, about = "Modularer Archivmanager")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    List { archive: PathBuf },
    Extract {
        archive: PathBuf,
        #[arg(long)]
        entry: String,
        #[arg(long, default_value = ".")]
        output: PathBuf,
    },
    Create {
        archive: PathBuf,
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
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
        Command::Extract { archive, entry, output } => {
            extract_entry(&archive, &entry, &output)?;
            println!("Extrahiert: {entry}");
        }
        Command::Create { archive, inputs } => {
            create_zip(&archive, &inputs)?;
            println!("Erstellt: {}", archive.display());
        }
    }
    Ok(())
}
