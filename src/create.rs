//! `scrippolib new`: create a package or script file and open it in the configured editor.
use anyhow::{Context, Result, bail};
use scriptopotamus::config::Config;
use scriptopotamus::paths::{self, NewKind};
use std::path::PathBuf;
use std::process::Command;

/// Create `<name>.scrippo` for `kind`: a package in `.scrippo/lib` (or `~/.scrippo/lib`),
/// a script in the current directory (or `~/.scrippo/scripts`).
pub fn create(kind: NewKind, global: bool, name: &str) -> Result<PathBuf> {
    let stem = paths::validate_name(name)?;
    let path = crate::store::destination(kind, global)?.join(format!("{stem}.scrippo"));

    if path.exists() {
        bail!("already exists: {}", path.display());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let body = match kind {
        NewKind::Package => format!(
            "@info\n  --name {stem}\n  --version 0.1.0\n  --description \"A {stem} package\"\n  --package\n\n# {stem} package\n"
        ),
        NewKind::Script => format!(
            "#! /usr/bin/env scrippo\n@info\n  --name {stem}\n  --version 0.1.0\n  --description \"A {stem} script\"\n  --script\n\n.()\n  -> \"root command\"\n"
        ),
    };
    std::fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

/// Open `path` with the editor from `scrippo --configure editor=...`, when one is set.
pub fn open_in_editor(path: &std::path::Path) -> Result<()> {
    let editor = Config::load()?.editor;
    if editor.is_empty() {
        return Ok(());
    }
    let status = Command::new("bash")
        .arg("-c")
        .arg(format!("{editor} \"$@\""))
        .arg("scrippolib-editor")
        .arg(path)
        .status()
        .with_context(|| format!("created {}, but failed to launch editor", path.display()))?;
    if !status.success() {
        bail!("created {}, but editor exited with {status}", path.display());
    }
    Ok(())
}
