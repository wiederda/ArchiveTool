use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    SevenZip,
}

impl ArchiveFormat {
    pub fn detect(path: &Path) -> crate::Result<Self> {
        use std::io::Read;
        let mut file = std::fs::File::open(path)?;
        let mut signature = [0u8; 6];
        let count = file.read(&mut signature)?;

        if count >= 4
            && (&signature[..4] == b"PK\x03\x04"
                || &signature[..4] == b"PK\x05\x06"
                || &signature[..4] == b"PK\x07\x08")
        {
            return Ok(Self::Zip);
        }

        if count == 6 && &signature == b"7z\xBC\xAF\x27\x1C" {
            return Ok(Self::SevenZip);
        }

        Err(crate::ArchiveError::UnsupportedFormat(
            path.extension()
                .and_then(|s| s.to_str())
                .unwrap_or("unbekannt")
                .to_string(),
        ))
    }
}
