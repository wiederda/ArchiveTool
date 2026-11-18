use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("Datei konnte nicht gelesen werden: {0}")]
    Io(#[from] std::io::Error),
    #[error("ZIP-Fehler: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("7z-Fehler: {0}")]
    SevenZip(String),
    #[error("Nicht unterstütztes Archivformat: {0}")]
    UnsupportedFormat(String),
    #[error("Eintrag nicht gefunden: {0}")]
    EntryNotFound(String),
    #[error("Unsicherer Archivpfad: {0}")]
    UnsafePath(String),
    #[error("Ungültiger Zielpfad: {0}")]
    InvalidPath(PathBuf),
}

pub type Result<T> = std::result::Result<T, ArchiveError>;
