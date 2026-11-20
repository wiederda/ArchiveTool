use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use flate2::read::GzDecoder;
use lz4_flex::frame::FrameDecoder;
use sevenz_rust2::{Archive, ArchiveReader, Password};
use tar::Archive as TarArchive;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

use bzip2::read::BzDecoder;
use lzma_rust2::XzReader;

use crate::{ArchiveEntry, ArchiveError, ArchiveFormat, Result, TarCompression};

/// Ergebnis einer Operation, die mehrere Einträge verarbeitet (Extraktion, Test).
#[derive(Debug, Default)]
pub struct OperationReport {
    /// Anzahl erfolgreich verarbeiteter Einträge.
    pub processed: usize,
    /// Fehler pro Eintrag, damit einzelne Probleme nachvollziehbar bleiben.
    pub failures: Vec<EntryFailure>,
}

impl OperationReport {
    pub fn is_success(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Fehler bei einem einzelnen Archiveintrag.
#[derive(Debug, Clone)]
pub struct EntryFailure {
    pub name: String,
    pub message: String,
}

/// Ergebnis einer Archivänderung.
#[derive(Debug, Default, Clone, Copy)]
pub struct UpdateReport {
    pub added: usize,
    pub replaced: usize,
    pub removed: usize,
    pub renamed: usize,
}

pub fn list_entries(path: &Path) -> Result<Vec<ArchiveEntry>> {
    match ArchiveFormat::detect(path)? {
        ArchiveFormat::Zip => {
            let file = File::open(path)?;
            let mut archive = ZipArchive::new(file)?;
            let mut entries = Vec::with_capacity(archive.len());
            for index in 0..archive.len() {
                let item = archive.by_index(index)?;
                entries.push(ArchiveEntry {
                    name: item.name().to_string(),
                    size: item.size(),
                    compressed_size: Some(item.compressed_size()),
                    is_directory: item.is_dir(),
                });
            }
            Ok(entries)
        }
        ArchiveFormat::SevenZip => {
            let archive = Archive::open(path).map_err(|e| ArchiveError::SevenZip(e.to_string()))?;
            Ok(archive
                .files
                .iter()
                .map(|item| ArchiveEntry {
                    name: item.name.clone(),
                    size: item.size,
                    compressed_size: Some(item.compressed_size),
                    is_directory: item.is_directory,
                })
                .collect())
        }
        format => list_tar_entries(path, tar_compression(format)),
    }
}

fn tar_compression(format: ArchiveFormat) -> TarCompression {
    format
        .tar_compression()
        .expect("ZIP und 7z werden bereits vorher behandelt")
}

fn list_tar_entries(path: &Path, compression: TarCompression) -> Result<Vec<ArchiveEntry>> {
    let mut archive = open_tar(path, compression)?;
    let mut entries = Vec::new();
    for item in archive.entries()? {
        let item = item?;
        let is_directory = item.header().entry_type().is_dir();
        let name = tar_entry_name(&item)?;
        entries.push(ArchiveEntry {
            name: if is_directory {
                format!("{}/", name.trim_end_matches('/'))
            } else {
                name
            },
            size: item.size(),
            compressed_size: None,
            is_directory,
        });
    }
    Ok(entries)
}

fn open_tar(path: &Path, compression: TarCompression) -> Result<TarArchive<Box<dyn Read>>> {
    let file = File::open(path)?;
    let reader: Box<dyn Read> = match compression {
        TarCompression::None => Box::new(file),
        TarCompression::Gzip => Box::new(GzDecoder::new(file)),
        TarCompression::Bzip2 => Box::new(BzDecoder::new(file)),
        TarCompression::Xz => Box::new(XzReader::new(file, true)),
        TarCompression::Zstd => Box::new(zstd::stream::read::Decoder::new(file)?),
        TarCompression::Lz4 => Box::new(FrameDecoder::new(file)),
    };
    Ok(TarArchive::new(reader))
}

fn tar_entry_name<R: Read>(entry: &tar::Entry<'_, R>) -> Result<String> {
    let path = entry.path()?;
    let name = path.to_string_lossy().replace('\\', "/");
    Ok(name.strip_prefix("./").unwrap_or(&name).to_string())
}

pub fn extract_entry(archive_path: &Path, entry_name: &str, destination: &Path) -> Result<()> {
    let relative = safe_relative_path(entry_name)?;
    let target = destination.join(relative);

    match ArchiveFormat::detect(archive_path)? {
        ArchiveFormat::Zip => {
            let file = File::open(archive_path)?;
            let mut archive = ZipArchive::new(file)?;
            let mut item = archive
                .by_name(entry_name)
                .map_err(|_| ArchiveError::EntryNotFound(entry_name.to_string()))?;

            if item.is_dir() {
                fs::create_dir_all(&target)?;
                return Ok(());
            }

            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = File::create(&target)?;
            std::io::copy(&mut item, &mut output)?;
            output.flush()?;
            Ok(())
        }
        ArchiveFormat::SevenZip => {
            let metadata =
                Archive::open(archive_path).map_err(|e| ArchiveError::SevenZip(e.to_string()))?;
            let item = metadata
                .files
                .iter()
                .find(|item| item.name == entry_name)
                .ok_or_else(|| ArchiveError::EntryNotFound(entry_name.to_string()))?;

            if item.is_directory {
                fs::create_dir_all(&target)?;
                return Ok(());
            }

            let mut reader = ArchiveReader::open(archive_path, Password::empty())
                .map_err(|e| ArchiveError::SevenZip(e.to_string()))?;
            let data = reader
                .read_file(entry_name)
                .map_err(|e| ArchiveError::SevenZip(e.to_string()))?;
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target, data)?;
            Ok(())
        }
        format => extract_tar_entry(
            archive_path,
            entry_name,
            destination,
            tar_compression(format),
        ),
    }
}

fn extract_tar_entry(
    archive_path: &Path,
    entry_name: &str,
    destination: &Path,
    compression: TarCompression,
) -> Result<()> {
    let wanted = normalize_name(entry_name);
    let wanted = wanted.trim_end_matches('/');
    safe_relative_path(entry_name)?;

    let mut archive = open_tar(archive_path, compression)?;
    for item in archive.entries()? {
        let mut item = item?;
        if tar_entry_name(&item)?.trim_end_matches('/') != wanted {
            continue;
        }
        let is_directory = item.header().entry_type().is_dir();
        return extract_tar_item(&mut item, wanted, is_directory, destination);
    }
    Err(ArchiveError::EntryNotFound(entry_name.to_string()))
}

/// Extrahiert alle Einträge des Archivs.
pub fn extract_all(archive_path: &Path, destination: &Path) -> Result<OperationReport> {
    extract_where(archive_path, destination, |_| true)
}

/// Extrahiert genau die gewählten Einträge. Verzeichnisse werden rekursiv
/// entpackt. Auswahlpfade werden mit `/` normalisiert.
pub fn extract_selection(
    archive_path: &Path,
    entries: &[String],
    destination: &Path,
) -> Result<OperationReport> {
    if entries.is_empty() {
        return Ok(OperationReport::default());
    }
    let selection = Selection::new(entries);
    extract_where(archive_path, destination, |name| selection.matches(name))
}

fn extract_where(
    archive_path: &Path,
    destination: &Path,
    wanted: impl Fn(&str) -> bool,
) -> Result<OperationReport> {
    match ArchiveFormat::detect(archive_path)? {
        ArchiveFormat::Zip => extract_zip_where(archive_path, destination, wanted),
        ArchiveFormat::SevenZip => extract_7z_where(archive_path, destination, wanted),
        ArchiveFormat::Tar => {
            extract_tar_where(archive_path, destination, wanted, TarCompression::None)
        }
        ArchiveFormat::TarGz => {
            extract_tar_where(archive_path, destination, wanted, TarCompression::Gzip)
        }
        ArchiveFormat::TarBz2 => {
            extract_tar_where(archive_path, destination, wanted, TarCompression::Bzip2)
        }
        ArchiveFormat::TarXz => {
            extract_tar_where(archive_path, destination, wanted, TarCompression::Xz)
        }
        ArchiveFormat::TarZstd => {
            extract_tar_where(archive_path, destination, wanted, TarCompression::Zstd)
        }
        ArchiveFormat::TarLz4 => {
            extract_tar_where(archive_path, destination, wanted, TarCompression::Lz4)
        }
    }
}

fn extract_tar_where(
    archive_path: &Path,
    destination: &Path,
    wanted: impl Fn(&str) -> bool,
    compression: TarCompression,
) -> Result<OperationReport> {
    let mut archive = open_tar(archive_path, compression)?;
    let mut report = OperationReport::default();

    for item in archive.entries()? {
        let mut item = item?;
        let name = tar_entry_name(&item)?;
        let is_directory = item.header().entry_type().is_dir();
        let listed = if is_directory {
            format!("{}/", name.trim_end_matches('/'))
        } else {
            name.clone()
        };
        if !wanted(&listed) && !wanted(&name) {
            continue;
        }
        match extract_tar_item(&mut item, &name, is_directory, destination) {
            Ok(()) => report.processed += 1,
            Err(error) => report.failures.push(EntryFailure {
                name: listed,
                message: error.to_string(),
            }),
        }
    }

    Ok(report)
}

fn extract_tar_item<R: Read>(
    item: &mut tar::Entry<'_, R>,
    name: &str,
    is_directory: bool,
    destination: &Path,
) -> Result<()> {
    let target = destination.join(safe_relative_path(name)?);
    if is_directory {
        fs::create_dir_all(&target)?;
        return Ok(());
    }
    if !item.header().entry_type().is_file() {
        return Err(ArchiveError::UnsupportedOperation(format!(
            "{name}: Es werden nur Dateien und Ordner unterstützt"
        )));
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = File::create(&target)?;
    std::io::copy(item, &mut output)?;
    output.flush()?;
    Ok(())
}

fn extract_zip_where(
    archive_path: &Path,
    destination: &Path,
    wanted: impl Fn(&str) -> bool,
) -> Result<OperationReport> {
    let file = File::open(archive_path)?;
    let mut archive = ZipArchive::new(file)?;
    let mut report = OperationReport::default();

    for index in 0..archive.len() {
        let mut item = archive.by_index(index)?;
        let name = item.name().to_string();
        if !wanted(&name) {
            continue;
        }
        let is_directory = item.is_dir();
        match extract_zip_item(&mut item, &name, is_directory, destination) {
            Ok(()) => report.processed += 1,
            Err(error) => report.failures.push(EntryFailure {
                name,
                message: error.to_string(),
            }),
        }
    }

    Ok(report)
}

fn extract_zip_item<R: Read>(
    item: &mut zip::read::ZipFile<'_, R>,
    name: &str,
    is_directory: bool,
    destination: &Path,
) -> Result<()> {
    let target = destination.join(safe_relative_path(name)?);
    if is_directory {
        fs::create_dir_all(&target)?;
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = File::create(&target)?;
    std::io::copy(item, &mut output)?;
    output.flush()?;
    Ok(())
}

fn extract_7z_where(
    archive_path: &Path,
    destination: &Path,
    wanted: impl Fn(&str) -> bool,
) -> Result<OperationReport> {
    let metadata =
        Archive::open(archive_path).map_err(|e| ArchiveError::SevenZip(e.to_string()))?;
    let mut report = OperationReport::default();

    // Verzeichnisse zuerst anlegen, unabhängig von der Reihenfolge im Archiv.
    for item in &metadata.files {
        if !item.is_directory || !wanted(&item.name) {
            continue;
        }
        let outcome = safe_relative_path(&item.name)
            .and_then(|relative| Ok(fs::create_dir_all(destination.join(relative))?));
        match outcome {
            Ok(()) => report.processed += 1,
            Err(error) => report.failures.push(EntryFailure {
                name: item.name.clone(),
                message: error.to_string(),
            }),
        }
    }

    let mut reader = ArchiveReader::open(archive_path, Password::empty())
        .map_err(|e| ArchiveError::SevenZip(e.to_string()))?;
    reader
        .for_each_entries(|entry, data| {
            if !wanted(&entry.name) || entry.is_directory {
                return Ok(true);
            }
            match extract_7z_data(entry, data, destination) {
                Ok(()) => report.processed += 1,
                Err(error) => report.failures.push(EntryFailure {
                    name: entry.name.clone(),
                    message: error.to_string(),
                }),
            }
            Ok(true)
        })
        .map_err(|e| ArchiveError::SevenZip(e.to_string()))?;

    Ok(report)
}

fn extract_7z_data(
    entry: &sevenz_rust2::ArchiveEntry,
    data: &mut dyn Read,
    destination: &Path,
) -> Result<()> {
    let target = destination.join(safe_relative_path(&entry.name)?);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = File::create(&target)?;
    std::io::copy(data, &mut output)?;
    output.flush()?;
    Ok(())
}

/// Prüft die Integrität des Archivs und meldet Fehler pro Eintrag.
pub fn test_archive(archive_path: &Path) -> Result<OperationReport> {
    match ArchiveFormat::detect(archive_path)? {
        ArchiveFormat::Zip => {
            let file = File::open(archive_path)?;
            let mut archive = ZipArchive::new(file)?;
            let mut report = OperationReport::default();
            for index in 0..archive.len() {
                let mut item = archive.by_index(index)?;
                let name = item.name().to_string();
                let mut sink = std::io::sink();
                match std::io::copy(&mut item, &mut sink) {
                    Ok(_) => report.processed += 1,
                    Err(error) => report.failures.push(EntryFailure {
                        name,
                        message: error.to_string(),
                    }),
                }
            }
            Ok(report)
        }
        ArchiveFormat::SevenZip => {
            let mut reader = ArchiveReader::open(archive_path, Password::empty())
                .map_err(|e| ArchiveError::SevenZip(e.to_string()))?;
            let mut report = OperationReport::default();
            reader
                .for_each_entries(|entry, data| {
                    let mut sink = std::io::sink();
                    match std::io::copy(data, &mut sink) {
                        Ok(_) => report.processed += 1,
                        Err(error) => report.failures.push(EntryFailure {
                            name: entry.name.clone(),
                            message: error.to_string(),
                        }),
                    }
                    Ok(true)
                })
                .map_err(|e| ArchiveError::SevenZip(e.to_string()))?;
            Ok(report)
        }
        format => test_tar_archive(archive_path, tar_compression(format)),
    }
}

fn test_tar_archive(archive_path: &Path, compression: TarCompression) -> Result<OperationReport> {
    let mut archive = open_tar(archive_path, compression)?;
    let mut report = OperationReport::default();
    for item in archive.entries()? {
        let mut item = match item {
            Ok(item) => item,
            Err(error) => {
                report.failures.push(EntryFailure {
                    name: "unbekannt".to_string(),
                    message: error.to_string(),
                });
                break;
            }
        };
        let name = tar_entry_name(&item)?;
        let mut sink = std::io::sink();
        match std::io::copy(&mut item, &mut sink) {
            Ok(_) => report.processed += 1,
            Err(error) => {
                report.failures.push(EntryFailure {
                    name,
                    message: error.to_string(),
                });
                break;
            }
        }
    }
    Ok(report)
}

pub fn create_zip(output: &Path, inputs: &[PathBuf]) -> Result<()> {
    let planned = plan_inputs(inputs)?;
    let file = File::create(output)?;
    let mut writer = ZipWriter::new(file);
    write_planned(&mut writer, &planned, file_options())?;
    writer.finish()?;
    Ok(())
}

/// Erstellt ein 7z-Archiv in einer temporären Datei, prüft es vollständig und
/// ersetzt erst danach ein eventuell vorhandenes Zielarchiv.
pub fn create_7z(output: &Path, inputs: &[PathBuf]) -> Result<()> {
    let planned = plan_inputs(inputs)?;
    let temp = temp_path_for(output);
    let result = (|| -> Result<()> {
        let mut writer = sevenz_rust2::ArchiveWriter::create(&temp)
            .map_err(|error| ArchiveError::SevenZip(error.to_string()))?;
        for entry in &planned {
            let metadata = sevenz_rust2::ArchiveEntry::from_path(&entry.path, entry.name.clone());
            if entry.is_directory {
                writer
                    .push_archive_entry::<&[u8]>(metadata, None)
                    .map_err(|error| ArchiveError::SevenZip(error.to_string()))?;
            } else {
                let source = File::open(&entry.path)?;
                writer
                    .push_archive_entry(metadata, Some(source))
                    .map_err(|error| ArchiveError::SevenZip(error.to_string()))?;
            }
        }
        writer
            .finish()
            .map_err(|error| ArchiveError::SevenZip(error.to_string()))?;

        let written = list_entries(&temp)?.len();
        if written != planned.len() {
            return Err(ArchiveError::Verification(format!(
                "erwartet {} Einträge, geschrieben {written}",
                planned.len()
            )));
        }
        let report = test_archive(&temp)?;
        if !report.is_success() || report.processed != planned.len() {
            return Err(ArchiveError::Verification(format!(
                "7z-Prüfung fehlgeschlagen: {} von {} Einträgen geprüft",
                report.processed,
                planned.len()
            )));
        }
        Ok(())
    })();

    match result {
        Ok(()) if output.exists() => replace_archive(&temp, output),
        Ok(()) => fs::rename(&temp, output).map_err(ArchiveError::from),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

/// Fügt Dateien und Verzeichnisse zu einem bestehenden ZIP hinzu. Gleichnamige
/// Einträge werden ersetzt. Gearbeitet wird auf einer temporären Datei, die
/// erst nach erfolgreicher Prüfung das Original ersetzt.
pub fn add_to_zip(archive_path: &Path, inputs: &[PathBuf]) -> Result<UpdateReport> {
    ensure_zip(archive_path, "Hinzufügen")?;
    let planned = plan_inputs(inputs)?;
    if planned.is_empty() {
        return Ok(UpdateReport::default());
    }

    let mut replace_exact = HashSet::new();
    let mut replace_prefixes = Vec::new();
    for entry in &planned {
        if entry.is_directory {
            replace_prefixes.push(entry.name.clone());
        } else {
            replace_exact.insert(entry.name.clone());
        }
    }

    let temp = temp_path_for(archive_path);
    let result = (|| -> Result<UpdateReport> {
        let source_file = File::open(archive_path)?;
        let mut source = ZipArchive::new(source_file)?;
        let temp_file = File::create(&temp)?;
        let mut writer = ZipWriter::new(temp_file);
        let options = file_options();

        let replaced = copy_zip_entries(&mut source, &mut writer, options, |name| {
            !(replace_exact.contains(name)
                || replace_prefixes
                    .iter()
                    .any(|prefix| name.starts_with(prefix.as_str())))
        })?;
        let mut report = UpdateReport {
            replaced,
            ..Default::default()
        };
        write_planned(&mut writer, &planned, options)?;
        report.added = planned.len();
        writer.finish()?;

        let written = verify_zip(&temp)?;
        let expected = source.len() - report.replaced + planned.len();
        if written != expected {
            return Err(ArchiveError::Verification(format!(
                "erwartet {expected} Einträge, geschrieben {written}"
            )));
        }
        Ok(report)
    })();

    match result {
        Ok(report) => {
            replace_archive(&temp, archive_path)?;
            Ok(report)
        }
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

/// Entfernt Einträge aus einem ZIP. Verzeichnisse werden rekursiv entfernt.
pub fn remove_entries_from_zip(archive_path: &Path, entries: &[String]) -> Result<UpdateReport> {
    ensure_zip(archive_path, "Löschen")?;
    if entries.is_empty() {
        return Ok(UpdateReport::default());
    }
    let selection = Selection::new(entries);

    let temp = temp_path_for(archive_path);
    let result = (|| -> Result<UpdateReport> {
        let source_file = File::open(archive_path)?;
        let mut source = ZipArchive::new(source_file)?;
        let temp_file = File::create(&temp)?;
        let mut writer = ZipWriter::new(temp_file);
        let options = file_options();

        let removed = copy_zip_entries(&mut source, &mut writer, options, |name| {
            !selection.matches(name)
        })?;
        let report = UpdateReport {
            removed,
            ..Default::default()
        };
        writer.finish()?;

        let written = verify_zip(&temp)?;
        let expected = source.len() - report.removed;
        if written != expected {
            return Err(ArchiveError::Verification(format!(
                "erwartet {expected} Einträge, geschrieben {written}"
            )));
        }
        Ok(report)
    })();

    match result {
        Ok(report) => {
            replace_archive(&temp, archive_path)?;
            Ok(report)
        }
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

/// Ersetzt den Inhalt genau eines Dateieintrags durch eine lokale Datei. Der
/// Eintragsname im Archiv bleibt unverändert. Gearbeitet wird auf einer
/// temporären Datei, die erst nach erfolgreicher Prüfung das Original ersetzt.
pub fn replace_entry_in_zip(
    archive_path: &Path,
    entry_name: &str,
    source: &Path,
) -> Result<UpdateReport> {
    ensure_zip(archive_path, "Bearbeiten")?;
    let target = normalize_name(entry_name).trim_end_matches('/').to_string();
    safe_relative_path(&target)?;
    if !source.is_file() {
        return Err(ArchiveError::InvalidPath(source.to_path_buf()));
    }

    let temp = temp_path_for(archive_path);
    let result = (|| -> Result<UpdateReport> {
        let source_file = File::open(archive_path)?;
        let mut archive = ZipArchive::new(source_file)?;

        let mut found = false;
        for index in 0..archive.len() {
            let item = archive.by_index(index)?;
            if normalize_name(item.name()).trim_end_matches('/') == target {
                if item.is_dir() {
                    return Err(ArchiveError::UnsupportedOperation(format!(
                        "{entry_name} ist ein Verzeichnis und kann nicht direkt bearbeitet werden"
                    )));
                }
                found = true;
                break;
            }
        }
        if !found {
            return Err(ArchiveError::EntryNotFound(entry_name.to_string()));
        }

        let temp_file = File::create(&temp)?;
        let mut writer = ZipWriter::new(temp_file);
        let options = file_options();
        let mut replaced = 0usize;

        for index in 0..archive.len() {
            let mut item = archive.by_index(index)?;
            let name = item.name().to_string();
            if normalize_name(&name).trim_end_matches('/') == target {
                writer.start_file(name, options)?;
                let mut input = File::open(source)?;
                std::io::copy(&mut input, &mut writer)?;
                replaced += 1;
            } else if item.is_dir() {
                writer.add_directory(name, options)?;
            } else {
                writer.start_file(name, options)?;
                std::io::copy(&mut item, &mut writer)?;
            }
        }
        writer.finish()?;

        let written = verify_zip(&temp)?;
        if written != archive.len() || replaced != 1 {
            return Err(ArchiveError::Verification(format!(
                "erwartet {} Einträge mit genau einer Ersetzung, geschrieben {written}",
                archive.len()
            )));
        }
        Ok(UpdateReport {
            replaced,
            ..Default::default()
        })
    })();

    match result {
        Ok(report) => {
            replace_archive(&temp, archive_path)?;
            Ok(report)
        }
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

/// Benennt einen Eintrag in einem ZIP um. Bei Verzeichnissen wird der gesamte
/// Teilbaum mit umbenannt. Gearbeitet wird auf einer temporären Datei, die erst
/// nach erfolgreicher Prüfung das Original ersetzt.
pub fn rename_entry_in_zip(
    archive_path: &Path,
    entry_name: &str,
    new_name: &str,
) -> Result<UpdateReport> {
    ensure_zip(archive_path, "Umbenennen")?;
    let target = normalize_name(new_name).trim_end_matches('/').to_string();
    safe_relative_path(&target)?;
    let plan = RenamePlan::new(entry_name);

    if plan.from == target {
        return Ok(UpdateReport::default());
    }
    if plan.is_inside(&target) {
        return Err(ArchiveError::UnsupportedOperation(
            "Ein Verzeichnis kann nicht in sich selbst verschoben werden".to_string(),
        ));
    }

    let temp = temp_path_for(archive_path);
    let result = (|| -> Result<UpdateReport> {
        let source_file = File::open(archive_path)?;
        let mut source = ZipArchive::new(source_file)?;

        // Namen zuerst erfassen, damit Kollisionen vor dem Schreiben erkannt werden.
        let mut existing = Vec::with_capacity(source.len());
        for index in 0..source.len() {
            existing.push(source.by_index(index)?.name().to_string());
        }

        let matched: Vec<String> = existing
            .iter()
            .filter(|name| plan.matches(name))
            .cloned()
            .collect();
        if matched.is_empty() {
            return Err(ArchiveError::EntryNotFound(entry_name.to_string()));
        }

        let renamed: Vec<String> = matched
            .iter()
            .map(|name| plan.rename(name, &target))
            .collect();
        for name in &existing {
            if plan.matches(name) {
                continue;
            }
            let normalized = normalize_name(name);
            let collision = renamed.iter().any(|candidate| {
                let candidate = candidate.trim_end_matches('/');
                normalized.trim_end_matches('/') == candidate
                    || normalized.starts_with(&format!("{candidate}/"))
            });
            if collision {
                return Err(ArchiveError::EntryExists(new_name.to_string()));
            }
        }

        let temp_file = File::create(&temp)?;
        let mut writer = ZipWriter::new(temp_file);
        let options = file_options();
        for index in 0..source.len() {
            let mut item = source.by_index(index)?;
            let name = item.name().to_string();
            let output_name = if plan.matches(&name) {
                plan.rename(&name, &target)
            } else {
                name
            };
            if item.is_dir() {
                writer.add_directory(output_name, options)?;
            } else {
                writer.start_file(output_name, options)?;
                std::io::copy(&mut item, &mut writer)?;
            }
        }
        writer.finish()?;

        let written = verify_zip(&temp)?;
        if written != existing.len() {
            return Err(ArchiveError::Verification(format!(
                "erwartet {} Einträge, geschrieben {written}",
                existing.len()
            )));
        }
        Ok(UpdateReport {
            renamed: renamed.len(),
            ..Default::default()
        })
    })();

    match result {
        Ok(report) => {
            replace_archive(&temp, archive_path)?;
            Ok(report)
        }
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

/// Beschreibt das Umbenennen eines Eintrags oder eines ganzen Verzeichnisses.
struct RenamePlan {
    from: String,
    prefix: String,
}

impl RenamePlan {
    fn new(from: &str) -> Self {
        let from = normalize_name(from).trim_end_matches('/').to_string();
        Self {
            prefix: format!("{from}/"),
            from,
        }
    }

    fn matches(&self, name: &str) -> bool {
        let name = normalize_name(name);
        name == self.from || name.starts_with(&self.prefix)
    }

    fn is_inside(&self, target: &str) -> bool {
        target.starts_with(&self.prefix)
    }

    fn rename(&self, name: &str, target: &str) -> String {
        let normalized = normalize_name(name);
        let mut result = if normalized == self.from {
            target.to_string()
        } else {
            format!(
                "{target}/{}",
                normalized[self.prefix.len()..].trim_start_matches('/')
            )
        };
        if name.ends_with('/') {
            result = format!("{}/", result.trim_end_matches('/'));
        }
        result
    }
}

struct PlannedEntry {
    path: PathBuf,
    name: String,
    is_directory: bool,
}

fn plan_inputs(inputs: &[PathBuf]) -> Result<Vec<PlannedEntry>> {
    let mut planned = Vec::new();
    for input in inputs {
        if input.is_dir() {
            plan_directory(input, &mut planned)?;
        } else if input.is_file() {
            let name = input
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| ArchiveError::InvalidPath(input.clone()))?;
            planned.push(PlannedEntry {
                path: input.clone(),
                name: name.to_string(),
                is_directory: false,
            });
        }
    }
    Ok(planned)
}

fn plan_directory(directory: &Path, planned: &mut Vec<PlannedEntry>) -> Result<()> {
    let base = directory.parent().unwrap_or(directory);
    let mut items = walk_files(directory)?;
    // Das Verzeichnis selbst aufnehmen, damit auch leere Ordner erhalten bleiben.
    items.insert(0, directory.to_path_buf());
    for item in items {
        let relative = item
            .strip_prefix(base)
            .map_err(|_| ArchiveError::InvalidPath(item.clone()))?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let name = relative.to_string_lossy().replace('\\', "/");
        if item.is_dir() {
            planned.push(PlannedEntry {
                path: item,
                name: format!("{name}/"),
                is_directory: true,
            });
        } else {
            planned.push(PlannedEntry {
                path: item,
                name,
                is_directory: false,
            });
        }
    }
    Ok(())
}

fn write_planned(
    writer: &mut ZipWriter<File>,
    planned: &[PlannedEntry],
    options: SimpleFileOptions,
) -> Result<()> {
    for entry in planned {
        if entry.is_directory {
            writer.add_directory(entry.name.as_str(), options)?;
        } else {
            add_file(writer, &entry.path, &entry.name, options)?;
        }
    }
    Ok(())
}

fn file_options() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)
}

fn copy_zip_entries(
    source: &mut ZipArchive<File>,
    writer: &mut ZipWriter<File>,
    options: SimpleFileOptions,
    mut keep: impl FnMut(&str) -> bool,
) -> Result<usize> {
    let mut skipped = 0;
    for index in 0..source.len() {
        let mut item = source.by_index(index)?;
        let name = item.name().to_string();
        if !keep(&name) {
            skipped += 1;
            continue;
        }
        if item.is_dir() {
            writer.add_directory(name, options)?;
        } else {
            writer.start_file(name, options)?;
            std::io::copy(&mut item, writer)?;
        }
    }
    Ok(skipped)
}

fn verify_zip(path: &Path) -> Result<usize> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    for index in 0..archive.len() {
        let mut item = archive.by_index(index)?;
        std::io::copy(&mut item, &mut std::io::sink())?;
    }
    Ok(archive.len())
}

fn ensure_zip(path: &Path, operation: &str) -> Result<()> {
    match ArchiveFormat::detect(path)? {
        ArchiveFormat::Zip => Ok(()),
        format => Err(ArchiveError::UnsupportedOperation(format!(
            "{operation} für {} wird noch nicht unterstützt",
            format
        ))),
    }
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".tmp{}", std::process::id()));
    path.with_file_name(name)
}

/// Ersetzt das Original erst, wenn die neue Datei vollständig geschrieben und
/// geprüft wurde. Das Original wird bis dahin als Sicherung behalten.
fn replace_archive(temp: &Path, target: &Path) -> Result<()> {
    let mut backup_name = target
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    backup_name.push(format!(".bak{}", std::process::id()));
    let backup = target.with_file_name(backup_name);

    fs::rename(target, &backup)?;
    match fs::rename(temp, target) {
        Ok(()) => {
            let _ = fs::remove_file(&backup);
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&backup, target);
            let _ = fs::remove_file(temp);
            Err(error.into())
        }
    }
}

fn walk_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(current)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path.clone());
                result.push(path);
            } else if path.is_file() {
                result.push(path);
            }
        }
    }
    result.sort();
    Ok(result)
}

fn add_file(
    writer: &mut ZipWriter<File>,
    path: &Path,
    archive_name: &str,
    options: SimpleFileOptions,
) -> Result<()> {
    writer.start_file(archive_name.replace('\\', "/"), options)?;
    let mut source = File::open(path)?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
    }
    Ok(())
}

fn safe_relative_path(name: &str) -> Result<PathBuf> {
    // Archive paths conventionally use '/', but malicious archives may contain
    // Windows separators or drive/UNC prefixes even when extracted on Linux.
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/')
        || normalized.starts_with("//")
        || normalized.as_bytes().get(1) == Some(&b':')
    {
        return Err(ArchiveError::UnsafePath(name.to_string()));
    }
    let path = Path::new(&normalized);
    if path.is_absolute() {
        return Err(ArchiveError::UnsafePath(name.to_string()));
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            _ => return Err(ArchiveError::UnsafePath(name.to_string())),
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(ArchiveError::UnsafePath(name.to_string()));
    }
    Ok(clean)
}

/// Auswahl von Archiveinträgen: exakte Namen sowie Verzeichnispräfixe.
struct Selection {
    exact: HashSet<String>,
    prefixes: Vec<String>,
}

impl Selection {
    fn new(names: &[String]) -> Self {
        let exact: HashSet<String> = names.iter().map(|name| normalize_name(name)).collect();
        let prefixes = exact
            .iter()
            .map(|name| format!("{}/", name.trim_end_matches('/')))
            .collect();
        Self { exact, prefixes }
    }

    fn matches(&self, name: &str) -> bool {
        let normalized = normalize_name(name);
        self.exact.contains(&normalized)
            || self
                .prefixes
                .iter()
                .any(|prefix| normalized.starts_with(prefix.as_str()))
    }
}

fn normalize_name(name: &str) -> String {
    let normalized = name.replace('\\', "/");
    normalized
        .strip_prefix("./")
        .unwrap_or(&normalized)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("archive-core-{tag}-{}-{stamp}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_file(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn names(path: &Path) -> Vec<String> {
        list_entries(path)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect()
    }

    #[test]
    fn create_extract_update_and_test_zip() {
        let temp = TempDir::new("roundtrip");
        let source = temp.0.join("source");
        write_file(&source.join("readme.txt"), "hallo");
        write_file(&source.join("docs/notes.md"), "notizen");
        fs::create_dir_all(source.join("docs/leer")).unwrap();

        let archive = temp.0.join("test.zip");
        create_zip(&archive, &[source.join("readme.txt"), source.join("docs")]).unwrap();

        let listed = names(&archive);
        assert!(listed.contains(&"readme.txt".to_string()));
        assert!(listed.iter().any(|name| name == "docs/notes.md"));
        assert!(listed.iter().any(|name| name == "docs/leer/"));

        let out = temp.0.join("out");
        let report = extract_selection(&archive, &["docs".to_string()], &out).unwrap();
        assert!(report.is_success());
        assert_eq!(
            fs::read_to_string(out.join("docs/notes.md")).unwrap(),
            "notizen"
        );
        assert!(out.join("docs/leer").is_dir());
        assert!(!out.join("readme.txt").exists());

        let all = temp.0.join("all");
        let report = extract_all(&archive, &all).unwrap();
        assert!(report.is_success());
        assert_eq!(report.processed, listed.len());
        assert_eq!(fs::read_to_string(all.join("readme.txt")).unwrap(), "hallo");

        let extra = temp.0.join("extra/neu.txt");
        write_file(&extra, "neu");
        let report = add_to_zip(&archive, &[extra]).unwrap();
        assert_eq!(report.added, 1);
        assert!(names(&archive).contains(&"neu.txt".to_string()));

        let replacement = temp.0.join("replacement/readme.txt");
        write_file(&replacement, "geändert");
        let report = add_to_zip(&archive, &[replacement]).unwrap();
        assert_eq!(report.replaced, 1);
        let out2 = temp.0.join("out2");
        extract_selection(&archive, &["readme.txt".to_string()], &out2).unwrap();
        assert_eq!(
            fs::read_to_string(out2.join("readme.txt")).unwrap(),
            "geändert"
        );

        let report = rename_entry_in_zip(&archive, "docs", "unterlagen").unwrap();
        assert!(report.renamed >= 2);
        let renamed = names(&archive);
        assert!(renamed.iter().any(|name| name == "unterlagen/notes.md"));
        assert!(renamed.iter().any(|name| name == "unterlagen/leer/"));
        assert!(!renamed.iter().any(|name| name.starts_with("docs/")));

        let report = rename_entry_in_zip(&archive, "readme.txt", "liesmich.txt").unwrap();
        assert_eq!(report.renamed, 1);
        assert!(names(&archive).contains(&"liesmich.txt".to_string()));

        assert!(matches!(
            rename_entry_in_zip(&archive, "liesmich.txt", "neu.txt"),
            Err(ArchiveError::EntryExists(_))
        ));
        assert!(matches!(
            rename_entry_in_zip(&archive, "gibtsnicht", "x.txt"),
            Err(ArchiveError::EntryNotFound(_))
        ));
        assert!(rename_entry_in_zip(&archive, "unterlagen", "unterlagen/kopie").is_err());
        assert!(rename_entry_in_zip(&archive, "liesmich.txt", "../ausserhalb.txt").is_err());

        let report = remove_entries_from_zip(&archive, &["unterlagen".to_string()]).unwrap();
        assert!(report.removed >= 2);
        let remaining = names(&archive);
        assert!(!remaining.iter().any(|name| name.starts_with("unterlagen/")));

        let report = test_archive(&archive).unwrap();
        assert!(report.is_success(), "{:?}", report.failures);
        assert_eq!(report.processed, remaining.len());
    }

    fn write_tar(path: &Path, source: &Path, compression: TarCompression) {
        let mut builder = tar::Builder::new(Vec::new());
        builder.append_dir_all("quelle", source).unwrap();
        let data = builder.into_inner().unwrap();

        let packed = match compression {
            TarCompression::None => data,
            TarCompression::Gzip => {
                let mut encoder =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(&data).unwrap();
                encoder.finish().unwrap()
            }
            TarCompression::Bzip2 => {
                let mut encoder =
                    bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
                encoder.write_all(&data).unwrap();
                encoder.finish().unwrap()
            }
            TarCompression::Xz => {
                let mut encoder =
                    lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(6))
                        .unwrap();
                encoder.write_all(&data).unwrap();
                encoder.finish().unwrap()
            }
            TarCompression::Zstd => {
                let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
                encoder.write_all(&data).unwrap();
                encoder.finish().unwrap()
            }
            TarCompression::Lz4 => {
                let mut encoder = lz4_flex::frame::FrameEncoder::new(Vec::new());
                encoder.write_all(&data).unwrap();
                encoder.finish().unwrap()
            }
        };
        fs::write(path, packed).unwrap();
    }

    #[test]
    fn reads_all_tar_variants() {
        let temp = TempDir::new("tar");
        let source = temp.0.join("source");
        write_file(&source.join("readme.txt"), "hallo");
        write_file(&source.join("docs/notes.md"), "notizen");
        fs::create_dir_all(source.join("docs/leer")).unwrap();

        for (file_name, compression) in [
            ("test.tar", TarCompression::None),
            ("test.tar.gz", TarCompression::Gzip),
            ("test.tar.bz2", TarCompression::Bzip2),
            ("test.tbz2", TarCompression::Bzip2),
            ("test.tar.xz", TarCompression::Xz),
            ("test.txz", TarCompression::Xz),
            ("test.tar.zst", TarCompression::Zstd),
            ("test.tzst", TarCompression::Zstd),
            ("test.tar.lz4", TarCompression::Lz4),
            ("test.tlz4", TarCompression::Lz4),
        ] {
            let archive = temp.0.join(file_name);
            write_tar(&archive, &source, compression);

            let listed = names(&archive);
            assert!(
                listed.contains(&"quelle/readme.txt".to_string()),
                "{file_name}: {listed:?}"
            );
            assert!(listed.iter().any(|name| name == "quelle/docs/notes.md"));

            let selected = temp.0.join(format!("selected-{file_name}"));
            let report =
                extract_selection(&archive, &["quelle/docs".to_string()], &selected).unwrap();
            assert!(report.is_success(), "{:?}", report.failures);
            assert_eq!(
                fs::read_to_string(selected.join("quelle/docs/notes.md")).unwrap(),
                "notizen"
            );
            assert!(selected.join("quelle/docs/leer").is_dir());
            assert!(!selected.join("quelle/readme.txt").exists());

            let all = temp.0.join(format!("all-{file_name}"));
            let report = extract_all(&archive, &all).unwrap();
            assert!(report.is_success(), "{:?}", report.failures);
            assert_eq!(
                fs::read_to_string(all.join("quelle/readme.txt")).unwrap(),
                "hallo"
            );

            let single = temp.0.join(format!("single-{file_name}"));
            extract_entry(&archive, "quelle/readme.txt", &single).unwrap();
            assert_eq!(
                fs::read_to_string(single.join("quelle/readme.txt")).unwrap(),
                "hallo"
            );

            let report = test_archive(&archive).unwrap();
            assert!(report.is_success(), "{:?}", report.failures);
            assert_eq!(report.processed, listed.len());
        }
    }

    #[test]
    fn replaces_entry_content_without_changing_its_name() {
        let temp = TempDir::new("replace");
        let source = temp.0.join("source");
        write_file(&source.join("readme.txt"), "alt");
        write_file(&source.join("docs/notes.md"), "notizen");
        let archive = temp.0.join("edit.zip");
        create_zip(&archive, &[source.join("readme.txt"), source.join("docs")]).unwrap();

        let edited = temp.0.join("edited.txt");
        write_file(&edited, "neu");
        let report = replace_entry_in_zip(&archive, "docs/notes.md", &edited).unwrap();
        assert_eq!(report.replaced, 1);

        let listed = names(&archive);
        assert!(listed.contains(&"docs/notes.md".to_string()));
        assert!(listed.contains(&"readme.txt".to_string()));

        let out = temp.0.join("out");
        extract_all(&archive, &out).unwrap();
        assert_eq!(
            fs::read_to_string(out.join("docs/notes.md")).unwrap(),
            "neu"
        );
        assert_eq!(fs::read_to_string(out.join("readme.txt")).unwrap(), "alt");

        assert!(matches!(
            replace_entry_in_zip(&archive, "docs/notes.md", &temp.0.join("fehlt.txt")),
            Err(ArchiveError::InvalidPath(_))
        ));
        assert!(matches!(
            replace_entry_in_zip(&archive, "gibtsnicht.txt", &edited),
            Err(ArchiveError::EntryNotFound(_))
        ));
        assert!(matches!(
            replace_entry_in_zip(&archive, "docs", &edited),
            Err(ArchiveError::UnsupportedOperation(_))
        ));
        assert!(replace_entry_in_zip(&archive, "../ausserhalb.txt", &edited).is_err());

        let report = test_archive(&archive).unwrap();
        assert!(report.is_success(), "{:?}", report.failures);
    }

    #[test]
    fn refuses_updates_for_non_zip_archives() {
        let temp = TempDir::new("nonzip");
        let fake = temp.0.join("fake.7z");
        fs::write(&fake, b"not an archive").unwrap();
        assert!(add_to_zip(&fake, &[]).is_err());
        assert!(remove_entries_from_zip(&fake, &["x".to_string()]).is_err());
        assert!(rename_entry_in_zip(&fake, "x", "y").is_err());
        assert!(replace_entry_in_zip(&fake, "x", &fake).is_err());
    }

    #[test]
    fn rejects_parent_traversal() {
        assert!(safe_relative_path("../outside.txt").is_err());
        assert!(safe_relative_path("folder/../../outside.txt").is_err());
        assert!(safe_relative_path("C:/Windows/win.ini").is_err());
        assert!(safe_relative_path("/etc/passwd").is_err());
    }

    #[test]
    fn accepts_normal_relative_path() {
        assert_eq!(
            safe_relative_path("config/settings.ini").unwrap(),
            PathBuf::from("config/settings.ini")
        );
        assert_eq!(
            safe_relative_path("./config/settings.ini").unwrap(),
            PathBuf::from("config/settings.ini")
        );
    }

    use std::path::PathBuf;
}
