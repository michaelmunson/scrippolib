//! Resolving what the user asked for and staging its files: a registry name, a Git
//! repository, or a plain directory.
use anyhow::{Context, Result, bail};
use scriptopotamus::ast::Stmt;
use scriptopotamus::macros::info::{FileInfo, PackageInfo, Version};
use scriptopotamus::paths::{self, NewKind};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

/// What an `install`/`update` argument refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A bare package or script name, looked up in the configured registry.
    Registry(String),
    /// A Git URL, Git repository, or plain directory.
    Source(String),
}

/// A bare name (`shell`) is a registry lookup. Anything with a path separator, a scheme,
/// or a leading dot is a source, so `./shell` names a local directory called `shell`.
pub fn classify(arg: &str) -> Target {
    if paths::is_package_name(arg) {
        Target::Registry(arg.to_string())
    } else {
        Target::Source(arg.to_string())
    }
}

/// Which revision of a Git source to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Revision {
    /// The highest stable `MAJOR.MINOR.PATCH` tag.
    LatestTag,
    Tag(String),
    /// The default branch (registries are not tagged).
    Head,
}

pub struct Staged {
    /// The tag everything was staged from; `None` when versions come from each entry's `@info`.
    pub tag: Option<String>,
    pub commit: String,
    /// Relative paths of every staged file.
    pub files: Vec<PathBuf>,
}

/// One installable `.scrippo` file.
#[derive(Debug)]
pub struct Entry {
    pub relative: PathBuf,
    pub kind: NewKind,
    pub info: PackageInfo,
}

pub fn version_tag(tag: &str) -> Option<Version> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag))
}

pub fn version_string(version: &Version) -> String {
    version
        .0
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

fn is_plain_directory(path: &Path) -> bool {
    path.is_dir() && !path.join(".git").exists()
}

/// Directories are stored by absolute path so a later `update` works from anywhere.
pub fn normalize_repository(repository: &str) -> String {
    let path = Path::new(repository);
    if path.exists() {
        if let Ok(canonical) = path.canonicalize() {
            return canonical.to_string_lossy().into_owned();
        }
    }
    repository.to_string()
}

pub fn repository_name(repository: &str) -> Result<&str> {
    let name = repository
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("");
    let name = name.strip_suffix(".git").unwrap_or(name);
    if name.is_empty()
        || !name.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        bail!("repository must have a simple basename");
    }
    Ok(name)
}

/// Materialize `repository` at `revision` into `source`, using `scratch` for Git data.
pub fn stage(
    repository: &str,
    revision: &Revision,
    scratch: &Path,
    source: &Path,
) -> Result<Staged> {
    let path = Path::new(repository);
    if is_plain_directory(path) {
        let files = snapshot_directory(path, source)?;
        let tag = match revision {
            Revision::Tag(tag) => Some(tag.clone()),
            _ => None,
        };
        return Ok(Staged {
            tag,
            commit: "local".into(),
            files,
        });
    }

    let git_dir = scratch.join("git");
    let mut clone = Command::new("git");
    clone.env("GIT_TERMINAL_PROMPT", "0").args([
        "-c",
        "core.hooksPath=/dev/null",
        "clone",
        "--quiet",
        "--bare",
    ]);
    if *revision == Revision::Head && !path.exists() {
        clone.args(["--depth", "1"]);
    }
    checked(clone.arg("--").arg(repository).arg(&git_dir))?;

    let (tag, commit) = match revision {
        Revision::Head => (None, rev_parse(&git_dir, "HEAD")?),
        Revision::Tag(tag) => (
            Some(tag.clone()),
            rev_parse(&git_dir, &format!("refs/tags/{tag}^{{commit}}"))?,
        ),
        Revision::LatestTag => {
            let output = git(&git_dir, &["tag", "--list"])?;
            let tag = String::from_utf8(output.stdout)?
                .lines()
                .filter_map(|tag| version_tag(tag).map(|version| (version, tag.to_string())))
                .max()
                .map(|(_, tag)| tag)
                .context("repository has no stable version tags (expected 1.2.3 or v1.2.3)")?;
            let commit = rev_parse(&git_dir, &format!("refs/tags/{tag}^{{commit}}"))?;
            (Some(tag), commit)
        }
    };
    let files = snapshot(&git_dir, &commit, source)?;
    Ok(Staged { tag, commit, files })
}

fn rev_parse(git_dir: &Path, revision: &str) -> Result<String> {
    Ok(
        String::from_utf8(git(git_dir, &["rev-parse", "--verify", revision])?.stdout)?
            .trim()
            .to_string(),
    )
}

/// Normalize the optional directory inside the source to a relative path
/// (empty means the source root).
pub fn source_root(directory: Option<&Path>) -> Result<PathBuf> {
    let mut root = PathBuf::new();
    for part in directory.into_iter().flat_map(Path::components) {
        match part {
            Component::CurDir => {}
            Component::Normal(part) if part != ".git" => root.push(part),
            _ => bail!(
                "directory must be a relative path inside the source: {}",
                directory.unwrap().display()
            ),
        }
    }
    Ok(root)
}

fn candidate_files<'a>(files: &'a [PathBuf], root: &Path) -> Vec<&'a PathBuf> {
    let mut sorted: Vec<_> = files
        .iter()
        .filter(|path| {
            path.starts_with(root)
                && path.extension().is_some_and(|ext| ext == "scrippo")
                && path.components().all(|part| {
                    !matches!(part, Component::Normal(s) if s.to_string_lossy().starts_with('.'))
                })
        })
        .collect();
    sorted.sort();
    sorted
}

/// Find every installable `.scrippo` file under `root`. Files without `@info`, or without a
/// `--package`/`--script` kind, are support files and are skipped. With `names`, only files
/// with a matching stem are read, so unrelated broken files cannot block the install.
pub fn select_entries(
    source: &Path,
    files: &[PathBuf],
    root: &Path,
    names: Option<&[String]>,
) -> Result<Vec<Entry>> {
    if !root.as_os_str().is_empty() && !files.iter().any(|path| path.starts_with(root)) {
        bail!("directory not found in source: {}", root.display());
    }
    let mut selected = Vec::new();
    for relative in candidate_files(files, root) {
        if let Some(names) = names {
            let stem = relative.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if !names.iter().any(|name| name == stem) {
                continue;
            }
        }
        if let Some(entry) = read_entry(&source.join(relative), relative)? {
            selected.push(entry);
        }
    }
    Ok(selected)
}

/// Like [`select_entries`] over the whole source, but skips files that fail to parse
/// (used for browsing a registry).
pub fn scan_entries(source: &Path, files: &[PathBuf]) -> Vec<Entry> {
    candidate_files(files, Path::new(""))
        .into_iter()
        .filter_map(|relative| read_entry(&source.join(relative), relative).ok().flatten())
        .collect()
}

fn read_entry(file: &Path, relative: &Path) -> Result<Option<Entry>> {
    let program = scriptopotamus::parse::parse_file(file)?;
    let declarations: Vec<_> = program
        .body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::MacroCall { name, args, line } if name == "info" => Some((args, line)),
            _ => None,
        })
        .collect();
    if declarations.len() > 1 {
        bail!("{}: expected exactly one top-level @info", file.display());
    }
    let Some(&(args, line)) = declarations.first() else {
        return Ok(None);
    };
    let kind = match FileInfo::parse(file, *line, args)?.kind {
        Some(scriptopotamus::macros::info::FileKind::Package) => NewKind::Package,
        Some(scriptopotamus::macros::info::FileKind::Script) => NewKind::Script,
        None => return Ok(None),
    };
    let info = PackageInfo::parse(file, *line, args)?;
    if file.file_stem().and_then(|s| s.to_str()) != Some(&info.name) {
        bail!("{}: @info name must match the filename", file.display());
    }
    Ok(Some(Entry {
        relative: relative.to_path_buf(),
        kind,
        info,
    }))
}

/// Read blob objects directly, never checking out hooks, filters, or links.
fn snapshot(git_dir: &Path, commit: &str, destination: &Path) -> Result<Vec<PathBuf>> {
    let tree = git(git_dir, &["ls-tree", "-rz", "--full-tree", commit])?;
    let mut entries = Vec::new();
    for entry in tree
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let entry = std::str::from_utf8(entry).context("repository paths must be UTF-8")?;
        let (header, name) = entry.split_once('\t').context("invalid Git tree entry")?;
        let fields: Vec<_> = header.split_whitespace().collect();
        if fields.len() != 3 || !matches!(fields[0], "100644" | "100755") || fields[1] != "blob" {
            bail!("repository links and submodules are not supported");
        }
        let relative = Path::new(name);
        if !relative
            .components()
            .all(|part| matches!(part, Component::Normal(s) if s != ".git"))
        {
            bail!("invalid repository path: {name}");
        }
        let path = destination.join(relative);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(
            &path,
            git(git_dir, &["cat-file", "blob", fields[2]])?.stdout,
        )?;
        #[cfg(unix)]
        if fields[0] == "100755" {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
        entries.push(relative.to_path_buf());
    }
    Ok(entries)
}

fn snapshot_directory(root: &Path, destination: &Path) -> Result<Vec<PathBuf>> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", root.display()))?;
    let mut entries = Vec::new();
    fn walk(root: &Path, dir: &Path, destination: &Path, entries: &mut Vec<PathBuf>) -> Result<()> {
        for item in
            fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))?
        {
            let item = item?;
            let name = item.file_name();
            // Installer scratch/state may live inside the source (e.g. installing `.`).
            if name == ".git" || name.to_string_lossy().starts_with(".scrippo-install") {
                continue;
            }
            let path = item.path();
            let meta = fs::symlink_metadata(&path)?;
            if meta.file_type().is_symlink() {
                bail!("repository links and submodules are not supported");
            }
            let relative = path
                .strip_prefix(root)
                .with_context(|| format!("path escaped install root: {}", path.display()))?;
            if !relative
                .components()
                .all(|part| matches!(part, Component::Normal(s) if s != ".git"))
            {
                bail!("invalid repository path: {}", relative.display());
            }
            if meta.is_dir() {
                walk(root, &path, destination, entries)?;
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let target = destination.join(relative);
            fs::create_dir_all(target.parent().unwrap())?;
            fs::copy(&path, &target)
                .with_context(|| format!("failed to copy {}", path.display()))?;
            entries.push(relative.to_path_buf());
        }
        Ok(())
    }
    walk(&root, &root, destination, &mut entries)?;
    Ok(entries)
}

/// Copy a staged tree (regular files and directories only; permissions are preserved).
pub fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for item in fs::read_dir(from)? {
        let item = item?;
        let target = to.join(item.file_name());
        if item.file_type()?.is_dir() {
            copy_tree(&item.path(), &target)?;
        } else {
            fs::copy(item.path(), &target)?;
        }
    }
    Ok(())
}

pub fn git(directory: &Path, args: &[&str]) -> Result<Output> {
    checked(Command::new("git").arg("-C").arg(directory).args(args))
}

pub fn checked(command: &mut Command) -> Result<Output> {
    let output = command
        .output()
        .with_context(|| format!("failed to run {:?}", command.get_program()))?;
    if !output.status.success() {
        bail!(
            "{} failed: {}",
            command.get_program().to_string_lossy(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output)
}
