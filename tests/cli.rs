use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    project: PathBuf,
    home: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let project = root.join("project");
        let home = root.join("home");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&home).unwrap();
        Self { _temp: temp, root, project, home }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_scrippolib-core"))
            .current_dir(&self.project)
            .env("SCRIPPO_HOME", &self.home)
            .env("SCRIPPOLIB_CONFIG", self.root.join("scrippolib.config"))
            .args(args)
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(output.status.success(), "{args:?}: {}", text(&output));
        text(&output)
    }

    fn fails(&self, args: &[&str], message: &str) {
        let output = self.run(args);
        assert!(!output.status.success(), "unexpected success: {}", text(&output));
        assert!(text(&output).contains(message), "expected {message:?}: {}", text(&output));
    }

    fn init(&self) {
        self.ok(&["init"]);
    }

    fn lib(&self) -> PathBuf {
        self.project.join(".scrippo/lib")
    }

    /// A plain-directory registry; configure scrippolib to use it.
    fn registry(&self) -> Registry {
        let dir = self.root.join("registry");
        fs::create_dir_all(&dir).unwrap();
        self.ok(&["configure", &format!("registry={}", dir.display())]);
        Registry(dir)
    }
}

struct Registry(PathBuf);

impl Registry {
    fn entry(&self, kind: &str, name: &str, version: &str, body: &str) {
        let dir = self.0.join(format!("{kind}s/{name}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(format!("{name}.scrippo")),
            format!("@info\n  --name {name} --version {version} --description \"The {name} {kind}\" --{kind}\n{body}"),
        )
        .unwrap();
    }
}

fn text(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=t@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output));
}

/// A Git repository with a `greet` package and a `hello` script at `version`, tagged `v<version>`.
struct Repo(PathBuf);

impl Repo {
    fn new(f: &Fixture) -> Self {
        let dir = f.root.join("helpers");
        fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        let repo = Self(dir);
        repo.release("1.0.0");
        repo
    }

    fn release(&self, version: &str) {
        fs::write(self.0.join("greet.scrippo"), format!("@info\n  --name greet --version {version} --package\n@import \"./support.scrippo\"\n.greet()\n  message\n")).unwrap();
        fs::write(self.0.join("hello.scrippo"), format!("@info\n  --name hello --version {version} --script\n@import \"./support.scrippo\"\n.main()\n  message\n")).unwrap();
        fs::write(self.0.join("support.scrippo"), format!("message() -> \"{version}\"\n")).unwrap();
        git(&self.0, &["add", "."]);
        git(&self.0, &["commit", "-qm", version]);
        git(&self.0, &["tag", "-a", "-m", version, &format!("v{version}")]);
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

#[test]
fn init_creates_the_project_environment_and_local_packages_require_it() {
    let f = Fixture::new();
    let registry = f.registry();
    registry.entry("package", "util", "1.0.0", "util() -> \"u\"\n");
    f.fails(&["install", "util"], "run `scrippolib init`");
    assert!(f.ok(&["init"]).contains("initialized"));
    assert!(f.project.join(".scrippo").is_dir());
    f.ok(&["install", "util"]);
    assert!(f.lib().join("util.bash").is_file());
}

#[test]
fn configure_prints_and_sets_the_registry() {
    let f = Fixture::new();
    assert!(f.ok(&["configure"]).contains("registry=https://github.com/michaelmunson/scrippolib-registry"));
    f.ok(&["configure", "registry=https://example.com/reg"]);
    assert_eq!(f.ok(&["configure"]), "registry=https://example.com/reg\n");
    f.fails(&["configure", "color=blue"], "unknown config key 'color'");
    f.fails(&["configure", "registry"], "expected key=value");
    f.fails(&["configure", "registry="], "registry must be");
}

#[test]
fn registry_names_install_update_list_info_and_uninstall() {
    let f = Fixture::new();
    f.init();
    let registry = f.registry();
    registry.entry("package", "util", "1.0.0", "util() -> \"one\"\n");
    registry.entry("script", "deploy", "0.2.0", ".main()\n  -> \"deploy\"\n");

    assert!(f.ok(&["info", "util"]).contains("installed:   no"));

    f.ok(&["install", "util"]);
    f.ok(&["install", "deploy"]);
    let list = f.ok(&["list"]);
    assert!(list.contains("util") && list.contains("package"), "{list}");
    assert!(list.contains("deploy") && list.contains("script"), "{list}");
    assert!(f.project.join("deploy.scrippo").is_symlink());
    let info = f.ok(&["info", "util"]);
    assert!(info.contains("version:     1.0.0") && info.contains("source:      registry"), "{info}");
    f.fails(&["install", "util"], "use `scrippolib update util`");

    assert!(f.ok(&["update", "util"]).contains("up to date"));
    registry.entry("package", "util", "1.1.0", "util() -> \"two\"\n");
    let out = f.ok(&["update"]);
    assert!(out.contains("updated package util@1.1.0 (was 1.0.0)"), "{out}");
    assert!(out.contains("deploy is up to date"), "{out}");
    assert!(fs::read_to_string(f.lib().join("util.bash")).unwrap().contains("two"));
    assert!(f.ok(&["list"]).contains("1.1.0"));

    f.ok(&["uninstall", "util"]);
    f.ok(&["uninstall", "deploy"]);
    assert!(!f.lib().join("util.bash").exists());
    assert!(!f.project.join("deploy.scrippo").exists());
    assert_eq!(f.ok(&["list"]), "");
    f.fails(&["uninstall", "util"], "not installed here: util");
    f.fails(&["install", "missing"], "'missing' was not found in registry");
}

#[test]
fn global_scope_follows_scrippo_home_and_is_separate_from_local() {
    let f = Fixture::new();
    let registry = f.registry();
    registry.entry("package", "util", "1.0.0", "util() -> \"u\"\n");
    registry.entry("script", "deploy", "1.0.0", ".main()\n  -> \"d\"\n");
    f.ok(&["install", "-g", "util"]);
    f.ok(&["install", "--global", "deploy"]);
    assert!(f.home.join("lib/util.bash").is_file());
    assert!(f.home.join("scripts/deploy.scrippo").is_symlink());
    assert!(f.ok(&["list", "-g"]).contains("util"));
    assert_eq!(f.ok(&["list"]), "");
    f.fails(&["uninstall", "util"], "not installed here");
    f.ok(&["uninstall", "-g", "util"]);
    assert!(!f.home.join("lib/util.bash").exists());
    assert!(f.ok(&["info", "-g", "deploy"]).contains("installed:"));
}

#[test]
fn git_repositories_install_the_highest_tag_and_update_to_newer_ones() {
    let f = Fixture::new();
    f.init();
    let repo = Repo::new(&f);
    repo.release("1.9.0");
    repo.release("1.10.0");
    let out = f.ok(&["install", repo.path()]);
    assert!(out.contains("greet@1.10.0") && out.contains("hello@1.10.0"), "{out}");
    assert!(fs::read_to_string(f.lib().join("greet.bash")).unwrap().contains("1.10.0"));

    assert!(f.ok(&["update"]).matches("up to date").count() >= 2);
    repo.release("2.0.0");
    let out = f.ok(&["update", "hello"]);
    assert!(out.contains("updated script hello@2.0.0 (was v1.10.0)"), "{out}");
    // `greet` is a separate group (package) and was left alone until updated.
    assert!(fs::read_to_string(f.lib().join("greet.bash")).unwrap().contains("1.10.0"));
    f.ok(&["update", "greet"]);
    assert!(fs::read_to_string(f.lib().join("greet.bash")).unwrap().contains("2.0.0"));
    assert!(f.ok(&["list"]).contains("2.0.0"));
}

#[test]
fn explicit_tag_installs_that_version_only() {
    let f = Fixture::new();
    f.init();
    let repo = Repo::new(&f);
    repo.release("1.1.0");
    f.ok(&["install", "--tag", "v1.0.0", repo.path()]);
    assert!(fs::read_to_string(f.lib().join("greet.bash")).unwrap().contains("1.0.0"));
    f.fails(&["install", "--tag", "main", repo.path()], "--tag must be a stable version");
    f.fails(&["install", "--tag", "1.0.0", "util"], "--tag applies to Git repositories");
}

#[test]
fn directories_install_without_git_and_names_win_over_bare_directories() {
    let f = Fixture::new();
    f.init();
    let registry = f.registry();
    registry.entry("package", "other", "1.0.0", "other() -> 1\n");
    let dir = f.project.join("tools");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("tools.scrippo"), "@info\n  --name tools --version 3.1.4 --package\ntools() -> \"t\"\n").unwrap();
    // A bare `tools` is a registry name, not the directory.
    let output = f.run(&["install", "tools"]);
    assert!(!output.status.success());
    assert!(text(&output).contains("pass a path such as ./tools"), "{}", text(&output));
    f.ok(&["install", "./tools"]);
    assert!(f.lib().join("tools.bash").is_file());
    assert!(f.ok(&["list"]).contains("3.1.4"));
    // The directory path is remembered as an absolute path for updates.
    fs::write(dir.join("tools.scrippo"), "@info\n  --name tools --version 3.2.0 --package\ntools() -> \"t2\"\n").unwrap();
    assert!(f.ok(&["update", "tools"]).contains("tools@3.2.0"));
}

#[test]
fn failed_installs_and_updates_leave_the_previous_state_intact() {
    let f = Fixture::new();
    f.init();
    let repo = Repo::new(&f);
    f.ok(&["install", repo.path()]);
    // Version 2.0.0 ships a broken package: the update must fail and keep 1.0.0.
    fs::write(repo.0.join("greet.scrippo"), "@info\n  --name greet --version 2.0.0 --package\n.greet(\n").unwrap();
    git(&repo.0, &["commit", "-qam", "break"]);
    git(&repo.0, &["tag", "-a", "-m", "2", "v2.0.0"]);
    let output = f.run(&["update", "greet"]);
    assert!(!output.status.success(), "{}", text(&output));
    assert!(fs::read_to_string(f.lib().join("greet.bash")).unwrap().contains("1.0.0"));
    assert!(f.ok(&["list"]).contains("greet"));
    assert!(!fs::read_dir(&f.lib()).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".scrippo-install-")));
    // Re-running update (all) reports the failure but still processes other groups.
    let output = f.run(&["update"]);
    assert!(!output.status.success());
    assert!(text(&output).contains("update(s) failed"));
}

#[test]
fn metadata_problems_and_empty_sources_publish_nothing() {
    let f = Fixture::new();
    f.init();
    let dir = f.root.join("bad");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("a.scrippo"), "# no info\n").unwrap();
    f.fails(&["install", dir.to_str().unwrap()], "no installable packages or scripts found");
    fs::write(dir.join("a.scrippo"), "@info\n  --name wrong --version 1.0.0 --package\n").unwrap();
    f.fails(&["install", dir.to_str().unwrap()], "@info name must match the filename");
    fs::write(dir.join("a.scrippo"), "@info\n  --name a --version 1.0.0 --package\n").unwrap();
    fs::write(dir.join("b.scrippo"), "@info\n  --name b --version 1.0.1 --package\n").unwrap();
    f.fails(&["install", dir.to_str().unwrap()], "share one @info version");
    assert!(!f.lib().exists() || fs::read_dir(f.lib()).unwrap().next().is_none());
    f.fails(&["install", "/nonexistent/repo"], "git failed");
}

#[test]
fn partial_uninstall_keeps_the_rest_of_a_multi_entry_source() {
    let f = Fixture::new();
    f.init();
    let dir = f.root.join("multi");
    fs::create_dir_all(&dir).unwrap();
    for name in ["one", "two"] {
        fs::write(dir.join(format!("{name}.scrippo")), format!("@info\n  --name {name} --version 1.0.0 --package\n{name}() -> \"{name}\"\n")).unwrap();
    }
    f.ok(&["install", dir.to_str().unwrap()]);
    f.ok(&["uninstall", "one"]);
    assert!(!f.lib().join("one.bash").exists());
    assert!(f.lib().join("two.bash").is_file());
    let list = f.ok(&["list"]);
    assert!(list.contains("two") && !list.contains("one"), "{list}");
    f.ok(&["uninstall", "two"]);
    assert_eq!(f.ok(&["list"]), "");
}

#[test]
fn install_hooks_run_around_publication() {
    let f = Fixture::new();
    f.init();
    let dir = f.root.join("hooked");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("hooked.scrippo"),
        "@info\n  --name hooked --version 1.0.0 --package\n@hook pre-install\n  echo pre >> \"$LOG\"\n@hook post-install\n  echo post >> \"$LOG\"\nhooked() -> \"h\"\n",
    )
    .unwrap();
    let log = f.root.join("hooks.log");
    let output = Command::new(env!("CARGO_BIN_EXE_scrippolib-core"))
        .current_dir(&f.project)
        .env("SCRIPPO_HOME", &f.home)
        .env("LOG", &log)
        .args(["install", dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(fs::read_to_string(log).unwrap(), "pre\npost\n");
}

#[test]
fn help_and_version() {
    let f = Fixture::new();
    let help = f.ok(&["--help"]);
    for command in ["init", "new", "install", "update", "uninstall", "list", "info", "configure"] {
        assert!(help.contains(command), "{command}: {help}");
    }
    assert!(f.ok(&["--version"]).starts_with("scrippolib "));
    assert!(f.ok(&["install", "--help"]).contains("--global"));
}

// ---- Package compilation sandbox and repository safety ----

fn package_repo(f: &Fixture, body: &str) -> Repo {
    let repo = Repo::new(f);
    fs::write(repo.0.join("greet.scrippo"), format!("@info\n  --name greet --version 1.0.0 --package\n{body}")).unwrap();
    git(&repo.0, &["commit", "-qam", "custom"]);
    git(&repo.0, &["tag", "-f", "-a", "-m", "1", "v1.0.0"]);
    repo
}

#[test]
fn package_compilation_never_runs_raw_bash_or_command_substitutions() {
    let f = Fixture::new();
    f.init();
    let sentinel = f.root.join("executed");
    let repo = package_repo(&f, &format!("@import \"./raw.bash\"\nx=$(touch '{0}')\ngreet() -> hello\n", sentinel.display()));
    fs::write(repo.0.join("raw.bash"), format!("touch '{}'\n", sentinel.display())).unwrap();
    git(&repo.0, &["add", "."]);
    git(&repo.0, &["commit", "-qm", "raw"]);
    git(&repo.0, &["tag", "-f", "-a", "-m", "1", "v1.0.0"]);
    f.ok(&["install", repo.path()]);
    assert!(!sentinel.exists());
}

#[test]
fn repository_dependencies_and_the_standard_library_compile_inside_the_sandbox() {
    let f = Fixture::new();
    f.init();
    let repo = package_repo(&f, "@import nested\n@import scrippo/std/types\ngreet()\n  nested\n");
    fs::write(repo.0.join("nested.scrippo"), "@info\n  --name nested --version 1.0.0 --package\nnested() -> dependency\n").unwrap();
    git(&repo.0, &["add", "."]);
    git(&repo.0, &["commit", "-qm", "nested"]);
    git(&repo.0, &["tag", "-f", "-a", "-m", "1", "v1.0.0"]);
    f.ok(&["install", repo.path()]);
    let output = Command::new("bash")
        .args(["-c", "source \"$1\"; greet", "_"])
        .arg(f.lib().join("greet.bash"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "dependency\n");
}

#[test]
fn imports_outside_the_repository_and_host_packages_are_blocked() {
    for glob in [false, true] {
        let f = Fixture::new();
        f.init();
        let outside = f.root.join("outside.scrippo");
        fs::write(&outside, "secret() -> private\n").unwrap();
        let pattern = if glob { format!("{}/*.scrippo", f.root.display()) } else { outside.display().to_string() };
        let repo = package_repo(&f, &format!("@import \"{pattern}\"\n"));
        f.fails(&["install", repo.path()], "outside the package sandbox");
    }
    let f = Fixture::new();
    f.init();
    fs::create_dir_all(f.home.join("lib")).unwrap();
    fs::write(f.home.join("lib/private.scrippo"), "secret() -> private\n").unwrap();
    let repo = package_repo(&f, "@import private\n");
    f.fails(&["install", repo.path()], "package not found");
}

#[test]
fn home_imports_cannot_escape_the_sandbox() {
    for pattern in ["~/private.scrippo", "~/*.scrippo"] {
        let f = Fixture::new();
        f.init();
        fs::write(f.home.join("private.scrippo"), "secret() -> private\n").unwrap();
        let repo = package_repo(&f, &format!("@import {pattern}\n"));
        let output = Command::new(env!("CARGO_BIN_EXE_scrippolib-core"))
            .current_dir(&f.project)
            .env("SCRIPPO_HOME", &f.home)
            .env("HOME", &f.home)
            .args(["install", repo.path()])
            .output()
            .unwrap();
        assert!(!output.status.success() && text(&output).contains("outside the package sandbox"), "{}", text(&output));
        assert!(!f.lib().join("greet.bash").exists());
    }
}

#[cfg(unix)]
#[test]
fn repository_symlinks_and_submodules_are_rejected() {
    let f = Fixture::new();
    f.init();
    let repo = Repo::new(&f);
    std::os::unix::fs::symlink("/etc/passwd", repo.0.join("link")).unwrap();
    git(&repo.0, &["add", "."]);
    git(&repo.0, &["commit", "-qm", "link"]);
    git(&repo.0, &["tag", "v2.0.0"]);
    f.fails(&["install", repo.path()], "links and submodules");

    let f = Fixture::new();
    f.init();
    let repo = Repo::new(&f);
    let commit = String::from_utf8(Command::new("git").arg("-C").arg(&repo.0).args(["rev-parse", "HEAD"]).output().unwrap().stdout).unwrap();
    git(&repo.0, &["update-index", "--add", "--cacheinfo", &format!("160000,{},dep", commit.trim())]);
    git(&repo.0, &["commit", "-qm", "submodule"]);
    git(&repo.0, &["tag", "v2.0.0"]);
    f.fails(&["install", repo.path()], "links and submodules");
    // Symlinks in a plain directory are rejected too.
    let dir = f.root.join("plain");
    fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", dir.join("link")).unwrap();
    f.fails(&["install", dir.to_str().unwrap()], "links and submodules");
}

#[test]
fn simultaneous_installs_do_not_merge_or_overwrite_each_other() {
    let f = Fixture::new();
    f.init();
    let repo = Repo::new(&f);
    let start = || {
        Command::new(env!("CARGO_BIN_EXE_scrippolib-core"))
            .current_dir(&f.project)
            .env("SCRIPPO_HOME", &f.home)
            .args(["install", repo.path()])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    let (first, second) = (start(), start());
    let outputs = [first.wait_with_output().unwrap(), second.wait_with_output().unwrap()];
    assert_eq!(outputs.iter().filter(|o| o.status.success()).count(), 1);
    assert!(f.lib().join("greet.bash").is_file());
    assert!(!f.lib().join(".scrippo-install.lock").exists());
    assert!(!fs::read_dir(f.lib()).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".scrippo-install-")));
}

#[test]
fn existing_files_and_installations_are_never_overwritten_or_removed() {
    let f = Fixture::new();
    f.init();
    let repo = Repo::new(&f);
    fs::create_dir_all(f.lib()).unwrap();
    let source = f.lib().join("greet.scrippo");
    fs::write(&source, "original").unwrap();
    f.fails(&["install", repo.path()], "destination already exists");
    assert_eq!(fs::read_to_string(&source).unwrap(), "original");
    fs::remove_file(source).unwrap();
    f.ok(&["install", repo.path()]);
    let before = fs::read(f.lib().join("greet.bash")).unwrap();
    f.fails(&["install", repo.path()], "destination already exists");
    assert_eq!(fs::read(f.lib().join("greet.bash")).unwrap(), before);
    assert!(f.ok(&["list"]).contains("greet"));
}

#[test]
fn the_scrippo_wrapper_validates_arguments_then_delegates_to_core() {
    // The wrapper is cli/scrippolib.scrippo compiled by scrippo-core; skip when that is not built.
    let compiler = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scriptopotamus/target/debug/scrippo-core");
    if !compiler.exists() {
        return;
    }
    let f = Fixture::new();
    let wrapper = f.root.join("scrippolib");
    let compiled = Command::new(&compiler)
        .arg("-c")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("cli/scrippolib.scrippo"))
        .output()
        .unwrap();
    assert!(compiled.status.success(), "{}", text(&compiled));
    fs::write(&wrapper, &compiled.stdout).unwrap();
    let run = |args: &[&str]| {
        Command::new("bash")
            .arg(&wrapper)
            .args(args)
            .current_dir(&f.project)
            .env("SCRIPPOLIB_CORE", env!("CARGO_BIN_EXE_scrippolib-core"))
            .env("SCRIPPO_HOME", &f.home)
            .env("SCRIPPOLIB_CONFIG", f.root.join("scrippolib.config"))
            .output()
            .unwrap()
    };
    assert!(text(&run(&["--version"])).starts_with("scrippolib "));
    let help = text(&run(&["--help"]));
    for command in ["init", "new", "install", "update", "uninstall", "list", "info", "configure"] {
        assert!(help.contains(command), "{command}: {help}");
    }
    let out = run(&["install"]);
    assert!(!out.status.success() && text(&out).contains("<name> is required"), "{}", text(&out));
    let out = run(&["install", "x", "--bogus"]);
    assert!(!out.status.success() && text(&out).contains("unknown option"), "{}", text(&out));
    let out = run(&["bogus"]);
    assert!(!out.status.success() && text(&out).contains("unknown command"), "{}", text(&out));
    assert!(run(&["init"]).status.success());
    assert!(f.project.join(".scrippo").is_dir());
    let out = run(&["new", "package", "wrapped"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(f.lib().join("wrapped.scrippo").is_file());
    let out = run(&["new", "script", "-g", "wrapped_script"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(f.home.join("scripts/wrapped_script.scrippo").is_file());
    let out = run(&["new"]);
    assert!(!out.status.success() && text(&out).contains("package"), "{}", text(&out));
    assert!(run(&["configure", "registry=/tmp/reg"]).status.success());
    assert!(text(&run(&["configure"])).contains("registry=/tmp/reg"));
}

#[test]
fn new_creates_packages_and_scripts_locally_and_globally() {
    let f = Fixture::new();
    f.fails(&["new", "package", "mylib"], "run `scrippolib init`");
    f.init();
    f.ok(&["new", "package", "mylib"]);
    assert!(f.lib().join("mylib.scrippo").is_file());
    f.ok(&["new", "script", "hello"]);
    let script = fs::read_to_string(f.project.join("hello.scrippo")).unwrap();
    assert!(script.starts_with("#! /usr/bin/env scrippo\n.main()"), "{script}");
    f.ok(&["new", "script", "-g", "greet"]);
    assert!(f.home.join("scripts/greet.scrippo").is_file());
    f.ok(&["new", "package", "--global", "shared"]);
    assert!(f.home.join("lib/shared.scrippo").is_file());
    f.fails(&["new", "package", "mylib"], "already exists");
    f.fails(&["new", "script", "1bad"], "invalid name");
    f.fails(&["new", "thing", "x"], "thing");
}

#[cfg(unix)]
#[test]
fn new_opens_the_configured_editor_and_keeps_files_when_it_fails() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.init();
    let editor = f.root.join("fake editor");
    let log = f.root.join("editor args");
    let config = f.root.join("scrippo.config");
    fs::write(
        &editor,
        "#!/bin/bash\nprintf '%s\\n' \"$@\" > \"$EDITOR_LOG\"\nexit \"${EDITOR_STATUS:-0}\"\n",
    )
    .unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(&config, format!("editor='{}' --wait\n", editor.display())).unwrap();
    let run = |args: &[&str], status: &str| {
        Command::new(env!("CARGO_BIN_EXE_scrippolib-core"))
            .current_dir(&f.project)
            .env("SCRIPPO_HOME", &f.home)
            .env("SCRIPPO_CONFIG", &config)
            .env("EDITOR_LOG", &log)
            .env("EDITOR_STATUS", status)
            .args(args)
            .output()
            .unwrap()
    };
    let out = run(&["new", "script", "edited"], "0");
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        format!("--wait\n{}\n", f.project.join("edited.scrippo").display())
    );
    fs::remove_file(&log).unwrap();
    assert!(!run(&["new", "script", "edited"], "0").status.success());
    assert!(!log.exists(), "an existing file must not launch the editor");
    let failed = run(&["new", "script", "failing"], "7");
    assert!(!failed.status.success());
    assert!(text(&failed).contains("editor exited"), "{}", text(&failed));
    assert!(f.project.join("failing.scrippo").is_file());
}
