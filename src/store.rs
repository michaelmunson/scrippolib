//! What is installed where: install locations, manifests, listing and removal.
//!
//! An install of one source (a repository, directory, or registry entry) is a *group*,
//! stored at `<destination>/.scrippo-installed/<packages|scripts>/<id>/content/`
//! with an `install.json` manifest. The public files (`<name>.bash` for packages,
//! `<name>.scrippo` for scripts) live in `<destination>` and point into the group.
use crate::{kind_dir, kind_name};
use anyhow::{Context, Result, bail};
use scriptopotamus::paths::{self, NewKind};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const INSTALLED_DIR: &str = ".scrippo-installed";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub repository: String,
    pub tag: String,
    pub commit: String,
    /// `packages` or `scripts`.
    pub kind: String,
    pub entries: Vec<ManifestEntry>,
    /// Set for registry installs: the registry the entry came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub autoload: bool,
    #[serde(default)]
    pub source: String,
}

/// One installed source.
#[derive(Debug, Clone)]
pub struct Group {
    pub kind: NewKind,
    /// Directory holding the public files.
    pub destination: PathBuf,
    pub id: String,
    pub manifest: Manifest,
}

impl Group {
    pub fn dir(&self) -> PathBuf {
        group_dir(&self.destination, self.kind, &self.id)
    }

    pub fn public_file(&self, name: &str) -> PathBuf {
        self.destination.join(public_file_name(self.kind, name))
    }

    pub fn has(&self, name: &str) -> bool {
        self.manifest.entries.iter().any(|entry| entry.name == name)
    }
}

pub fn group_dir(destination: &Path, kind: NewKind, id: &str) -> PathBuf {
    destination
        .join(INSTALLED_DIR)
        .join(kind_dir(kind))
        .join(id)
}

pub fn public_file_name(kind: NewKind, name: &str) -> String {
    match kind {
        NewKind::Package => format!("{name}.bash"),
        NewKind::Script => format!("{name}.scrippo"),
    }
}

/// Where `kind` is installed. Local packages need an initialized project.
pub fn destination(kind: NewKind, global: bool) -> Result<PathBuf> {
    if global {
        return match kind {
            NewKind::Package => paths::global_lib_dir(),
            NewKind::Script => paths::global_scripts_dir(),
        };
    }
    let cwd = std::env::current_dir().context("failed to get current directory")?;
    Ok(match kind {
        NewKind::Package => paths::require_project_scrippo_dir(&cwd)
            .map_err(|_| anyhow::anyhow!("run `scrippolib init` to initialize your environment"))?
            .join("lib"),
        NewKind::Script => cwd,
    })
}

/// Like [`destination`], but `None` when it does not exist yet (nothing installed there).
fn existing_destination(kind: NewKind, global: bool) -> Result<Option<PathBuf>> {
    if !global && kind == NewKind::Package {
        let cwd = std::env::current_dir().context("failed to get current directory")?;
        return Ok(paths::find_project_scrippo_dir(&cwd).map(|dir| dir.join("lib")));
    }
    Ok(Some(destination(kind, global)?))
}

pub fn read_manifest(group_dir: &Path) -> Result<Manifest> {
    let file = group_dir.join("content/install.json");
    let bytes = fs::read(&file).with_context(|| format!("failed to read {}", file.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("invalid manifest {}", file.display()))
}

pub fn write_manifest_at(content: &Path, manifest: &Manifest) -> Result<()> {
    let file = content.join("install.json");
    fs::write(&file, serde_json::to_vec_pretty(manifest)?)
        .with_context(|| format!("failed to write {}", file.display()))
}

pub fn write_manifest(group_dir: &Path, manifest: &Manifest) -> Result<()> {
    let file = group_dir.join("content/install.json");
    fs::write(&file, serde_json::to_vec_pretty(manifest)?)
        .with_context(|| format!("failed to write {}", file.display()))
}

/// Every installed group in the local or global scope, packages first, sorted by id.
pub fn groups(global: bool) -> Result<Vec<Group>> {
    let mut groups = Vec::new();
    for kind in [NewKind::Package, NewKind::Script] {
        let Some(destination) = existing_destination(kind, global)? else {
            continue;
        };
        let root = destination.join(INSTALLED_DIR).join(kind_dir(kind));
        let Ok(items) = fs::read_dir(&root) else {
            continue;
        };
        let mut ids: Vec<_> = items
            .filter_map(|item| item.ok())
            .filter(|item| item.path().is_dir())
            .map(|item| item.file_name().to_string_lossy().into_owned())
            .collect();
        ids.sort();
        for id in ids {
            match read_manifest(&root.join(&id)) {
                Ok(manifest) => groups.push(Group {
                    kind,
                    destination: destination.clone(),
                    id,
                    manifest,
                }),
                Err(err) => eprintln!("warning: skipping {}: {err:#}", root.join(&id).display()),
            }
        }
    }
    Ok(groups)
}

/// Groups in scope that contain an entry called `name`.
pub fn find(global: bool, name: &str) -> Result<Vec<Group>> {
    Ok(groups(global)?
        .into_iter()
        .filter(|group| group.has(name))
        .collect())
}

pub struct Listed {
    pub name: String,
    pub version: String,
    pub kind: NewKind,
    pub origin: String,
}

pub fn list(global: bool) -> Result<Vec<Listed>> {
    let mut listed: Vec<Listed> = groups(global)?
        .into_iter()
        .flat_map(|group| {
            let origin = match &group.manifest.registry {
                Some(_) => "registry".to_string(),
                None => group.manifest.repository.clone(),
            };
            let kind = group.kind;
            group.manifest.entries.into_iter().map(move |entry| Listed {
                name: entry.name,
                version: entry.version,
                kind,
                origin: origin.clone(),
            })
        })
        .collect();
    listed.sort_by(|a, b| (a.name.as_str(), kind_name(a.kind)).cmp(&(b.name.as_str(), kind_name(b.kind))));
    Ok(listed)
}

/// Remove an installed entry, returning the paths that were deleted.
pub fn uninstall(global: bool, name: &str) -> Result<Vec<PathBuf>> {
    paths::validate_name(name)?;
    let found = find(global, name)?;
    if found.is_empty() {
        let scope = if global { "globally" } else { "here" };
        bail!("not installed {scope}: {name}");
    }
    let mut removed = Vec::new();
    for mut group in found {
        let public = group.public_file(name);
        if fs::symlink_metadata(&public).is_ok() {
            fs::remove_file(&public)
                .with_context(|| format!("failed to remove {}", public.display()))?;
            removed.push(public);
        }
        group.manifest.entries.retain(|entry| entry.name != name);
        let dir = group.dir();
        if group.manifest.entries.is_empty() {
            fs::remove_dir_all(&dir)
                .with_context(|| format!("failed to remove {}", dir.display()))?;
        } else {
            let _ = fs::remove_file(dir.join("content/compiled").join(format!("{name}.bash")));
            write_manifest(&dir, &group.manifest)?;
        }
    }
    Ok(removed)
}
