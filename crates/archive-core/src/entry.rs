#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    pub name: String,
    pub size: u64,
    pub compressed_size: Option<u64>,
    pub is_directory: bool,
}
