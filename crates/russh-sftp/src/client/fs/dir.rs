use std::collections::VecDeque;

use super::Metadata;
use crate::protocol::FileType;

/// Entries returned by the [`ReadDir`] iterator.
#[derive(Debug)]
pub struct DirEntry {
    file: String,
    metadata: Metadata,
}

impl DirEntry {
    /// Returns the file name for the file that this entry points at.
    pub fn file_name(&self) -> String {
        self.file.to_owned()
    }

    /// Returns the file type for the file that this entry points at.
    pub fn file_type(&self) -> FileType {
        self.metadata.file_type()
    }

    /// Returns the metadata for the file that this entry points at.
    pub fn metadata(&self) -> Metadata {
        self.metadata.to_owned()
    }
}

/// Iterator over the entries in a remote directory.
pub struct ReadDir {
    pub(crate) entries: VecDeque<(String, Metadata)>,
}

impl Iterator for ReadDir {
    type Item = DirEntry;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.entries.pop_front() {
                None => return None,
                Some(entry) if entry.0 == "." || entry.0 == ".." => continue,
                Some(entry) => {
                    return Some(DirEntry {
                        file: entry.0,
                        metadata: entry.1,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_dir_skips_many_dot_entries_without_recursion() {
        let mut entries = VecDeque::new();
        for index in 0..50_000 {
            let name = if index % 2 == 0 { "." } else { ".." };
            entries.push_back((name.to_string(), Metadata::default()));
        }
        entries.push_back(("payload.bin".to_string(), Metadata::default()));
        let mut directory = ReadDir { entries };

        assert_eq!(directory.next().unwrap().file_name(), "payload.bin");
        assert!(directory.next().is_none());
    }
}
