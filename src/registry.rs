//! Browsing a registry: a Git repository (or directory) of `.scrippo` files whose `@info`
//! declares them installable. Layout is free-form; the reference registry uses
//! `packages/<name>/<name>.scrippo`.
use crate::source::{self, Entry, Revision};
use anyhow::Result;
use std::path::Path;

pub struct Fetched {
    // Keeps the staged files alive.
    _temp: tempfile::TempDir,
    pub entries: Vec<Entry>,
    pub commit: String,
}

/// Fetch the registry's default branch and list what it offers.
pub fn fetch(registry: &str) -> Result<Fetched> {
    let temp = tempfile::Builder::new().prefix("scrippolib-registry-").tempdir()?;
    let root = temp.path().join("source");
    std::fs::create_dir_all(&root)?;
    let repository = source::normalize_repository(registry);
    let staged = source::stage(&repository, &Revision::Head, temp.path(), &root)?;
    let mut entries = source::scan_entries(&root, &staged.files);
    entries.sort_by(|a, b| a.info.name.cmp(&b.info.name));
    Ok(Fetched {
        _temp: temp,
        entries,
        commit: staged.commit,
    })
}

impl Fetched {
    pub fn find(&self, name: &str) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|entry| entry.info.name == name)
            .collect()
    }
}

pub fn relative_dir(entry: &Entry) -> &Path {
    entry.relative.parent().unwrap_or(Path::new(""))
}
