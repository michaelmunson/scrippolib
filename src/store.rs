//! What is installed where: install locations, manifests, listing and removal.
//!
//! An install of one source (a repository, directory, or registry entry) is a *group*,
//! stored at `<root>/.scrippolib/<packages|scripts>/<id>/content/` with an `install.json`
//! manifest, where `<root>` is the project's `.scrippo` directory (or the global one). The
//! public files (`<name>.bash` for packages, `<name>.scrippo` for scripts) live in the
//! destination and point into the group.
use crate::{kind_dir, kind_name};
use anyhow::{Context, Result, bail};
use scriptopotamus::paths::{self, NewKind};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const INSTALLED_DIR: &str = ".scrippolib";

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
    /// Where the public files live, relative to the store root. Empty in older manifests,
    /// which means the default for `kind`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub destination: String,
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
    /// The `.scrippo` directory holding `.scrippolib`.
    pub root: PathBuf,
    /// Directory holding the public files.
    pub destination: PathBuf,
    pub id: String,
    pub manifest: Manifest,
}

impl Group {
    pub fn dir(&self) -> PathBuf {
        group_dir(&self.root, self.kind, &self.id)
    }

    pub fn public_file(&self, name: &str) -> PathBuf {
        self.destination.join(public_file_name(self.kind, name))
    }

    pub fn has(&self, name: &str) -> bool {
        self.manifest.entries.iter().any(|entry| entry.name == name)
    }
}

pub fn group_dir(root: &Path, kind: NewKind, id: &str) -> PathBuf {
    root.join(INSTALLED_DIR)
        .join(kind_dir(kind))
        .join(id)
}

pub fn public_file_name(kind: NewKind, name: &str) -> String {
    match kind {
        NewKind::Package => format!("{name}.bash"),
        NewKind::Script => format!("{name}.scrippo"),
    }
}

/// Where `kind` is installed: the store root and the directory for the public files.
pub struct Location {
    pub root: PathBuf,
    pub destination: PathBuf,
}

/// Where `kind` is installed. Local installs need an initialized project.
pub fn location(kind: NewKind, global: bool) -> Result<Location> {
    if global {
        return Ok(Location {
            root: paths::scrippo_home()?,
            destination: match kind {
                NewKind::Package => paths::global_lib_dir()?,
                NewKind::Script => paths::global_scripts_dir()?,
            },
        });
    }
    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let root = paths::require_project_scrippo_dir(&cwd)
        .map_err(|_| anyhow::anyhow!("run `scrippolib init` to initialize your environment"))?;
    let destination = match kind {
        NewKind::Package => root.join("lib"),
        NewKind::Script => cwd,
    };
    Ok(Location { root, destination })
}

pub fn destination(kind: NewKind, global: bool) -> Result<PathBuf> {
    Ok(location(kind, global)?.destination)
}

/// Like [`location`], but `None` when there is no store root yet (nothing installed).
fn existing_root(global: bool) -> Result<Option<PathBuf>> {
    if global {
        return Ok(Some(paths::scrippo_home()?));
    }
    let cwd = std::env::current_dir().context("failed to get current directory")?;
    Ok(paths::find_project_scrippo_dir(&cwd))
}

/// `to` relative to the directory `from`. Both must be absolute and symlink-free.
pub fn relative_path(from: &Path, to: &Path) -> PathBuf {
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..from.len() {
        out.push("..");
    }
    for part in &to[common..] {
        out.push(part);
    }
    out
}

fn default_destination(kind: NewKind) -> &'static str {
    match kind {
        NewKind::Package => "lib",
        NewKind::Script => "scripts",
    }
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
    let Some(root) = existing_root(global)? else {
        return Ok(groups);
    };
    let cwd = std::env::current_dir().ok().and_then(|cwd| cwd.canonicalize().ok());
    for kind in [NewKind::Package, NewKind::Script] {
        let store = root.join(INSTALLED_DIR).join(kind_dir(kind));
        let Ok(items) = fs::read_dir(&store) else {
            continue;
        };
        let mut ids: Vec<_> = items
            .filter_map(|item| item.ok())
            .filter(|item| item.path().is_dir())
            .map(|item| item.file_name().to_string_lossy().into_owned())
            .collect();
        ids.sort();
        for id in ids {
            match read_manifest(&store.join(&id)) {
                Ok(manifest) => {
                    let relative = match manifest.destination.as_str() {
                        "" => default_destination(kind),
                        other => other,
                    };
                    let destination = root.join(relative);
                    // Local scripts live in the directory they were installed from.
                    if !global && kind == NewKind::Script {
                        let here = destination.canonicalize().ok();
                        if here.is_none() || here != cwd {
                            continue;
                        }
                    }
                    groups.push(Group {
                        kind,
                        root: root.clone(),
                        destination,
                        id,
                        manifest,
                    });
                }
                Err(err) => eprintln!("warning: skipping {}: {err:#}", store.join(&id).display()),
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
