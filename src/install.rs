//! Install and update: stage a source, validate and compile it, then publish it atomically.
use crate::source::{self, Entry, Revision, Target};
use crate::store::{self, Group, Manifest, ManifestEntry};
use crate::{kind_dir, kind_name};
use anyhow::{Context, Result, bail};
use scriptopotamus::ast::{Program, Stmt};
use scriptopotamus::paths::NewKind;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct Request {
    pub target: Target,
    pub tag: Option<String>,
    pub global: bool,
    /// The configured registry, used for [`Target::Registry`].
    pub registry: String,
}

/// What one install (or update) of a package or script group did.
#[derive(Debug)]
pub struct Installed {
    pub kind: NewKind,
    pub tag: String,
    pub commit: String,
    /// `(name, version)` of each entry.
    pub entries: Vec<(String, String)>,
    pub files: Vec<PathBuf>,
    /// The previously installed tag, for updates.
    pub replaced: Option<String>,
}

#[derive(Debug)]
pub enum Outcome {
    Installed(Installed),
    UpToDate { names: Vec<String>, version: String },
}

/// Where an install comes from and which of its entries to take.
struct Origin {
    repository: String,
    revision: Revision,
    registry: Option<String>,
    names: Option<Vec<String>>,
    id: Option<String>,
}

pub fn install(request: &Request) -> Result<Vec<Installed>> {
    if let Some(tag) = &request.tag {
        source::version_tag(tag).context("--tag must be a stable version such as 1.2.3 or v1.2.3")?;
    }
    let origin = match &request.target {
        Target::Registry(name) => {
            if request.tag.is_some() {
                bail!("--tag applies to Git repositories, not registry names");
            }
            Origin {
                repository: source::normalize_repository(&request.registry),
                revision: Revision::Head,
                registry: Some(request.registry.clone()),
                names: Some(vec![name.clone()]),
                id: Some(name.clone()),
            }
        }
        Target::Source(source) => Origin {
            repository: source::normalize_repository(source),
            revision: request
                .tag
                .clone()
                .map_or(Revision::LatestTag, Revision::Tag),
            registry: None,
            names: None,
            id: None,
        },
    };
    let outcomes = execute(&origin, request.global, &[])?;
    Ok(outcomes
        .into_iter()
        .filter_map(|outcome| match outcome {
            Outcome::Installed(installed) => Some(installed),
            Outcome::UpToDate { .. } => None,
        })
        .collect())
}

pub struct GroupUpdate {
    pub label: String,
    pub result: Result<Outcome>,
}

/// Update `name` (or everything installed in the scope when `None`) to the newest version
/// of the source it was installed from.
pub fn update(name: Option<&str>, global: bool) -> Result<Vec<GroupUpdate>> {
    let groups = match name {
        Some(name) => {
            let found = store::find(global, name)?;
            if found.is_empty() {
                let scope = if global { "globally" } else { "here" };
                bail!("not installed {scope}: {name}");
            }
            found
        }
        None => store::groups(global)?,
    };
    Ok(groups
        .into_iter()
        .map(|group| {
            let label = group
                .manifest
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let origin = Origin {
                repository: group
                    .manifest
                    .registry
                    .as_deref()
                    .map(source::normalize_repository)
                    .unwrap_or_else(|| group.manifest.repository.clone()),
                revision: if group.manifest.registry.is_some() {
                    Revision::Head
                } else {
                    Revision::LatestTag
                },
                registry: group.manifest.registry.clone(),
                names: Some(
                    group
                        .manifest
                        .entries
                        .iter()
                        .map(|entry| entry.name.clone())
                        .collect(),
                ),
                id: Some(group.id.clone()),
            };
            let result = execute(&origin, global, std::slice::from_ref(&group))
                .map(|mut outcomes| outcomes.remove(0));
            GroupUpdate { label, result }
        })
        .collect())
}

/// Entries of one kind, validated against a single version.
struct KindPlan<'a> {
    kind: NewKind,
    entries: Vec<&'a Entry>,
    tag: String,
    destination: PathBuf,
    replacing: Option<&'a Group>,
}

fn execute(origin: &Origin, global: bool, replacing: &[Group]) -> Result<Vec<Outcome>> {
    let repo_id = match &origin.id {
        Some(id) => id.clone(),
        None => source::repository_name(&origin.repository)?.to_string(),
    };
    let temp = tempfile::Builder::new()
        .prefix("scrippolib-")
        .tempdir()
        .context("failed to create a temporary directory")?;
    let source = temp.path().join("source");
    fs::create_dir_all(&source)?;
    let staged = source::stage(&origin.repository, &origin.revision, temp.path(), &source)?;
    let selected = source::select_entries(
        &source,
        &staged.files,
        Path::new(""),
        origin.names.as_deref(),
    )?;

    if let Some(names) = &origin.names {
        for name in names {
            if !selected.iter().any(|entry| &entry.info.name == name) {
                match &origin.registry {
                    Some(registry) => bail!("'{name}' was not found in registry {registry}"),
                    None => bail!("'{name}' is no longer provided by {}", origin.repository),
                }
            }
        }
    }
    if selected.is_empty() {
        bail!(
            "no installable packages or scripts found (expected .scrippo files declaring @info with --package or --script)"
        );
    }

    // One plan per kind; everything is validated before anything is published.
    let mut plans = Vec::new();
    for kind in [NewKind::Package, NewKind::Script] {
        let group = replacing.iter().find(|group| group.kind == kind);
        // An update only touches the kinds the group was installed as.
        if !replacing.is_empty() && group.is_none() {
            continue;
        }
        let entries: Vec<&Entry> = selected.iter().filter(|e| e.kind == kind).collect();
        if entries.is_empty() {
            continue;
        }
        let tag = match &staged.tag {
            Some(tag) => tag.clone(),
            None => directory_tag(&entries)?,
        };
        let version = source::version_tag(&tag).unwrap();
        let mut names = BTreeSet::new();
        for entry in &entries {
            if entry.info.version != version {
                bail!(
                    "{}: @info version must match the selected tag",
                    entry.relative.display()
                );
            }
            if !names.insert(&entry.info.name) {
                bail!("duplicate {} entry name: {}", kind_dir(kind), entry.info.name);
            }
        }
        let destination = match group {
            Some(group) => group.destination.clone(),
            None => store::destination(kind, global)?,
        };
        plans.push(KindPlan {
            kind,
            entries,
            tag,
            destination,
            replacing: group,
        });
    }
    for group in replacing {
        if !plans.iter().any(|plan| plan.kind == group.kind) {
            bail!(
                "{} no longer provides a {}",
                origin.repository,
                kind_name(group.kind)
            );
        }
    }

    let mut outcomes = Vec::new();
    let mut work = Vec::new();
    for plan in plans {
        if let Some(group) = plan.replacing {
            let old = source::version_tag(&group.manifest.tag);
            let new = source::version_tag(&plan.tag);
            let newer = match (old, new) {
                (Some(old), Some(new)) => new > old,
                _ => plan.tag != group.manifest.tag,
            };
            if !newer {
                outcomes.push(Outcome::UpToDate {
                    names: plan.entries.iter().map(|e| e.info.name.clone()).collect(),
                    version: group.manifest.tag.clone(),
                });
                continue;
            }
        }
        work.push(plan);
    }

    let mut locks = Vec::new();
    // Prepare: copy the staged tree beside its destination, run pre-install hooks, compile.
    let mut prepared = Vec::new();
    for plan in &work {
        fs::create_dir_all(&plan.destination)?;
        let destination = plan.destination.canonicalize()?;
        let scratch = tempfile::Builder::new()
            .prefix(".scrippo-install-")
            .tempdir_in(&destination)?;
        let content = scratch.path().join("content");
        let content_source = content.join("source");
        source::copy_tree(&source, &content_source)?;
        run_install_hooks(&content_source, &plan.entries, "pre-install")?;

        let mut artifacts = Vec::new();
        let mut metadata = Vec::new();
        for entry in &plan.entries {
            let file = content_source.join(&entry.relative);
            let artifact = match plan.kind {
                NewKind::Package => {
                    let bash = scriptopotamus::compile_package(&file, &content_source)?;
                    let output = content
                        .join("compiled")
                        .join(format!("{}.bash", entry.info.name));
                    fs::create_dir_all(output.parent().unwrap())?;
                    fs::write(&output, bash)?;
                    source::checked(Command::new("bash").arg("-n").arg(&output))?;
                    output.strip_prefix(&content)?.to_path_buf()
                }
                NewKind::Script => PathBuf::from("source").join(&entry.relative),
            };
            artifacts.push((entry.info.name.clone(), artifact));
            metadata.push(ManifestEntry {
                name: entry.info.name.clone(),
                version: source::version_string(&entry.info.version),
                description: entry.info.description.clone(),
                author: entry.info.metadata.author.clone(),
                license: entry.info.metadata.license.clone(),
                repository: entry.info.metadata.repository.clone(),
                kind: kind_name(plan.kind).to_string(),
                autoload: entry.info.metadata.autoload,
                source: entry.relative.to_string_lossy().into_owned(),
            });
        }
        let manifest = Manifest {
            repository: origin.repository.clone(),
            tag: plan.tag.clone(),
            commit: staged.commit.clone(),
            kind: kind_dir(plan.kind).to_string(),
            entries: metadata,
            registry: origin.registry.clone(),
        };
        store::write_manifest_at(&content, &manifest)?;
        prepared.push(Prepared {
            publication: None,
            scratch,
            destination,
            artifacts,
        });
    }

    // Publish. Cooperating installers serialize on a lock directory per destination, and each
    // public link is created with no-clobber semantics. A failure rolls back everything this
    // run created and restores whatever it replaced.
    for (plan, prepared) in work.iter().zip(prepared.iter_mut()) {
        let lock = prepared.destination.join(".scrippo-install.lock");
        fs::create_dir(&lock).context("another install is publishing here (or .scrippo-install.lock remains from an interrupted process)")?;
        locks.push(LockDir(lock));

        let repo_dir = store::group_dir(&prepared.destination, plan.kind, &repo_id);
        let mut publication = Publication {
            files: Vec::new(),
            repository: None,
            backups: Vec::new(),
            committed: false,
        };
        if let Some(group) = plan.replacing {
            let backup = prepared.scratch.path().join("previous");
            fs::create_dir_all(&backup)?;
            let old_dir = group.dir();
            let moved = backup.join("group");
            fs::rename(&old_dir, &moved)
                .with_context(|| format!("failed to move aside {}", old_dir.display()))?;
            publication.backups.push((old_dir, moved));
            for entry in &group.manifest.entries {
                let public = group.public_file(&entry.name);
                if fs::symlink_metadata(&public).is_ok() {
                    let moved = backup.join(format!("link-{}", entry.name));
                    fs::rename(&public, &moved)?;
                    publication.backups.push((public, moved));
                }
            }
        }
        prepared.publication = Some(publication);
        let publication = prepared.publication.as_mut().unwrap();

        for entry in &plan.entries {
            for ext in ["scrippo", "bash"] {
                let path = prepared.destination.join(format!("{}.{ext}", entry.info.name));
                if fs::symlink_metadata(&path).is_ok() {
                    bail!(
                        "destination already exists: {} (use `scrippolib update {}` to upgrade)",
                        path.display(),
                        entry.info.name
                    );
                }
            }
        }
        fs::create_dir_all(repo_dir.parent().unwrap())?;
        fs::create_dir(&repo_dir).context("already installed at this destination")?;
        publication.repository = Some(repo_dir.clone());
        fs::rename(prepared.scratch.path().join("content"), repo_dir.join("content"))?;
        for (name, relative) in &prepared.artifacts {
            let stored = repo_dir.join("content").join(relative);
            let public = prepared.destination.join(store::public_file_name(plan.kind, name));
            match plan.kind {
                NewKind::Package => fs::hard_link(&stored, &public)?,
                NewKind::Script => script_link(stored.strip_prefix(&prepared.destination)?, &public)?,
            }
            publication.files.push(public);
        }
    }

    for (plan, prepared) in work.iter().zip(&prepared) {
        let repo_dir = store::group_dir(&prepared.destination, plan.kind, &repo_id);
        run_install_hooks(&repo_dir.join("content/source"), &plan.entries, "post-install")?;
    }

    for (plan, mut prepared) in work.iter().zip(prepared) {
        let publication = prepared.publication.as_mut().unwrap();
        publication.committed = true;
        outcomes.push(Outcome::Installed(Installed {
            kind: plan.kind,
            tag: plan.tag.clone(),
            commit: staged.commit.clone(),
            entries: plan
                .entries
                .iter()
                .map(|e| (e.info.name.clone(), source::version_string(&e.info.version)))
                .collect(),
            files: publication.files.clone(),
            replaced: plan.replacing.map(|group| group.manifest.tag.clone()),
        }));
    }
    drop(locks);
    Ok(outcomes)
}

fn directory_tag(entries: &[&Entry]) -> Result<String> {
    let version = &entries[0].info.version;
    if entries.iter().any(|entry| &entry.info.version != version) {
        bail!("directory entries must share one @info version when --tag is omitted");
    }
    Ok(source::version_string(version))
}

struct Prepared {
    // Dropped first, so a failed publish rolls back before the scratch directory goes.
    publication: Option<Publication>,
    scratch: tempfile::TempDir,
    destination: PathBuf,
    artifacts: Vec<(String, PathBuf)>,
}

/// Run each entry's `@hook <phase>` blocks. A hook runs as its own script made from the
/// entry's `@import` lines and the hook body, with the source tree as the working directory.
fn run_install_hooks(source: &Path, entries: &[&Entry], phase: &str) -> Result<()> {
    for entry in entries {
        let file = source.join(&entry.relative);
        let program = scriptopotamus::parse::parse_file(&file)?;
        let imports: Vec<Stmt> = program
            .body
            .iter()
            .filter(|stmt| matches!(stmt, Stmt::MacroCall { name, .. } if name == "import"))
            .cloned()
            .collect();
        for stmt in &program.body {
            let Stmt::Hook { name, body, .. } = stmt else {
                continue;
            };
            if name != phase {
                continue;
            }
            let mut script = imports.clone();
            script.extend(body.iter().cloned());
            let bash = scriptopotamus::compile_program(&file, Program { body: script })?;
            run_hook_script(&file, &entry.relative, source, phase, &bash)?;
        }
    }
    Ok(())
}

fn run_hook_script(
    file: &Path,
    relative: &Path,
    source: &Path,
    phase: &str,
    bash: &str,
) -> Result<()> {
    let tmp = tempfile::Builder::new()
        .prefix(&format!("scrippo-{phase}-"))
        .suffix(".bash")
        .tempfile()
        .context("failed to create install hook temp file")?;
    fs::write(tmp.path(), bash.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o755))?;
    }
    let status = Command::new("bash")
        .env("_SCRIPPO_FILE", file)
        .current_dir(source)
        .arg(tmp.path())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to run @hook {phase} in {}", relative.display()))?;
    if !status.success() {
        bail!(
            "@hook {phase} in {} failed{}",
            relative.display(),
            status
                .code()
                .map(|code| format!(" (exit {code})"))
                .unwrap_or_default()
        );
    }
    Ok(())
}

#[cfg(unix)]
fn script_link(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}

#[cfg(windows)]
fn script_link(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(source, destination)
}

struct LockDir(PathBuf);
impl Drop for LockDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

struct Publication {
    files: Vec<PathBuf>,
    /// Set once this run has created the directory, so rollback never removes someone else's.
    repository: Option<PathBuf>,
    /// `(original, moved-aside)` paths of what an update replaced.
    backups: Vec<(PathBuf, PathBuf)>,
    committed: bool,
}

impl Drop for Publication {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for file in &self.files {
            let _ = fs::remove_file(file);
        }
        if let Some(repository) = &self.repository {
            let _ = fs::remove_dir_all(repository);
        }
        for (original, moved) in self.backups.iter().rev() {
            let _ = fs::rename(moved, original);
        }
    }
}
