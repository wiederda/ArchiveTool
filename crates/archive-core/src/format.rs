use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    SevenZip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZstd,
    TarLz4,
}

/// Kompression einer TAR-Datei.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TarCompression {
    #[default]
    None,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
    Lz4,
}

impl ArchiveFormat {
    pub fn detect(path: &Path) -> crate::Result<Self> {
        use std::io::Read;
        let mut file = std::fs::File::open(path)?;
        // 265 Byte reichen für ZIP-/7z-Signaturen und die ustar-Kennung an
        // Offset 257. Leere Archive werden zusätzlich über die Endung erkannt.
        let mut signature = [0u8; 265];
        let count = file.read(&mut signature)?;

        if count >= 4
            && (&signature[..4] == b"PK\x03\x04"
                || &signature[..4] == b"PK\x05\x06"
                || &signature[..4] == b"PK\x07\x08")
        {
            return Ok(Self::Zip);
        }

        if count >= 6 && &signature[..6] == b"7z\xBC\xAF\x27\x1C" {
            return Ok(Self::SevenZip);
        }

        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        // Komprimierte TAR-Varianten werden an der Endung erkannt: Ihre
        // Signaturen (z. B. BZh, XZ oder Zstandard) unterscheiden nicht
        // zwischen einem TAR-Archiv und einer einzelnen komprimierten Datei.
        if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            return Ok(Self::TarGz);
        }
        if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") || name.ends_with(".tbz") {
            return Ok(Self::TarBz2);
        }
        if name.ends_with(".tar.xz") || name.ends_with(".txz") {
            return Ok(Self::TarXz);
        }
        if name.ends_with(".tar.zst") || name.ends_with(".tzst") {
            return Ok(Self::TarZstd);
        }
        if name.ends_with(".tar.lz4") || name.ends_with(".tlz4") {
            return Ok(Self::TarLz4);
        }

        if count >= 262 && &signature[257..262] == b"ustar" {
            return Ok(Self::Tar);
        }

        if name.ends_with(".tar") {
            return Ok(Self::Tar);
        }

        if count >= 2 && &signature[..2] == b"\x1F\x8B" {
            return Ok(Self::TarGz);
        }

        Err(crate::ArchiveError::UnsupportedFormat(
            path.extension()
                .and_then(|s| s.to_str())
                .unwrap_or("unbekannt")
                .to_string(),
        ))
    }

    /// Ob das Format beim Lesen und Entpacken unterstützt wird.
    pub fn supports_reading(&self) -> bool {
        matches!(
            self,
            Self::Zip
                | Self::SevenZip
                | Self::Tar
                | Self::TarGz
                | Self::TarBz2
                | Self::TarXz
                | Self::TarZstd
                | Self::TarLz4
        )
    }

    /// Kompression der TAR-Variante, falls das Format ein TAR-Archiv ist.
    pub fn tar_compression(&self) -> Option<TarCompression> {
        match self {
            Self::Tar => Some(TarCompression::None),
            Self::TarGz => Some(TarCompression::Gzip),
            Self::TarBz2 => Some(TarCompression::Bzip2),
            Self::TarXz => Some(TarCompression::Xz),
            Self::TarZstd => Some(TarCompression::Zstd),
            Self::TarLz4 => Some(TarCompression::Lz4),
            Self::Zip | Self::SevenZip => None,
        }
    }
}

impl std::fmt::Display for ArchiveFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Zip => formatter.write_str("ZIP"),
            Self::SevenZip => formatter.write_str("7Z"),
            Self::Tar => formatter.write_str("TAR"),
            Self::TarGz => formatter.write_str("TAR.GZ"),
            Self::TarBz2 => formatter.write_str("TAR.BZ2"),
            Self::TarXz => formatter.write_str("TAR.XZ"),
            Self::TarZstd => formatter.write_str("TAR.ZST"),
            Self::TarLz4 => formatter.write_str("TAR.LZ4"),
        }
    }
}
