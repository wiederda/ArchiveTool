use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use sevenz_rust2::{Archive, ArchiveReader, Password};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

use crate::{ArchiveEntry, ArchiveError, ArchiveFormat, Result};

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
            let archive = Archive::open(path)
                .map_err(|e| ArchiveError::SevenZip(e.to_string()))?;
            Ok(archive.files.iter().map(|item| ArchiveEntry {
                name: item.name.clone(),
                size: item.size,
                compressed_size: Some(item.compressed_size),
                is_directory: item.is_directory,
            }).collect())
        }
    }
}

pub fn extract_entry(archive_path: &Path, entry_name: &str, destination: &Path) -> Result<()> {
    let relative = safe_relative_path(entry_name)?;
    let target = destination.join(relative);

    match ArchiveFormat::detect(archive_path)? {
        ArchiveFormat::Zip => {
            let file = File::open(archive_path)?;
            let mut archive = ZipArchive::new(file)?;
            let mut item = archive.by_name(entry_name)
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
            let metadata = Archive::open(archive_path)
                .map_err(|e| ArchiveError::SevenZip(e.to_string()))?;
            let item = metadata.files.iter()
                .find(|item| item.name == entry_name)
                .ok_or_else(|| ArchiveError::EntryNotFound(entry_name.to_string()))?;

            if item.is_directory {
                fs::create_dir_all(&target)?;
                return Ok(());
            }

            let mut reader = ArchiveReader::open(archive_path, Password::empty())
                .map_err(|e| ArchiveError::SevenZip(e.to_string()))?;
            let data = reader.read_file(entry_name)
                .map_err(|e| ArchiveError::SevenZip(e.to_string()))?;
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target, data)?;
            Ok(())
        }
    }
}

pub fn create_zip(output: &Path, inputs: &[PathBuf]) -> Result<()> {
    let file = File::create(output)?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for input in inputs {
        if input.is_dir() {
            add_directory(&mut writer, input, options)?;
        } else if input.is_file() {
            let name = input.file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| ArchiveError::InvalidPath(input.clone()))?;
            add_file(&mut writer, input, name, options)?;
        }
    }

    writer.finish()?;
    Ok(())
}

fn add_directory(
    writer: &mut ZipWriter<File>,
    directory: &Path,
    options: SimpleFileOptions,
) -> Result<()> {
    let base = directory.parent().unwrap_or(directory);
    for item in walk_files(directory)? {
        let relative = item.strip_prefix(base)
            .map_err(|_| ArchiveError::InvalidPath(item.clone()))?;
        let name = relative.to_string_lossy().replace('\\', "/");
        if item.is_dir() {
            writer.add_directory(format!("{name}/"), options)?;
        } else {
            add_file(writer, &item, &name, options)?;
        }
    }
    Ok(())
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
        if read == 0 { break; }
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


#[cfg(test)]
mod tests {
    use super::safe_relative_path;

    #[test]
    fn rejects_parent_traversal() {
        assert!(safe_relative_path("../outside.txt").is_err());
        assert!(safe_relative_path("folder/../../outside.txt").is_err());
    }

    #[test]
    fn accepts_normal_relative_path() {
        assert_eq!(
            safe_relative_path("config/settings.ini").unwrap(),
            PathBuf::from("config/settings.ini")
        );
    }

    use std::path::PathBuf;
}
