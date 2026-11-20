mod archive;
mod entry;
mod error;
mod format;

pub use archive::{
    EntryFailure, OperationReport, UpdateReport, add_to_zip, create_7z, create_zip, extract_all,
    extract_entry, extract_selection, list_entries, remove_entries_from_zip, rename_entry_in_zip,
    replace_entry_in_zip, test_archive,
};
pub use entry::ArchiveEntry;
pub use error::{ArchiveError, Result};
pub use format::{ArchiveFormat, TarCompression};
