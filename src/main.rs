use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use scriptopotamus::paths;
use scrippolib::config::{self, Config};
use scrippolib::install::{self, Outcome, Request};
use scrippolib::source::{self, Target};
use scrippolib::{kind_name, registry, store};

#[derive(Parser, Debug)]
#[command(name = "scrippolib", version, about = "Package manager for scrippo packages and scripts")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Initialize a new scrippolib project (creates .scrippo here)
    Init,
    /// Install a package or script from the registry, a Git repository, or a directory
    Install {
        /// A registry name, a Git URL/repository, or a directory (use ./name for a directory
        /// whose name is also a valid registry name)
        name: String,
        /// Install into ~/.scrippo (or SCRIPPO_HOME) instead of the current project
        #[arg(short, long)]
        global: bool,
        /// Exact stable version tag to install from a Git repository (default: highest)
        #[arg(long, value_name = "VERSION")]
        tag: Option<String>,
    },
    /// Update a package or script, or everything installed when no name is given
    Update {
        name: Option<String>,
        #[arg(short, long)]
        global: bool,
    },
    /// Remove an installed package or script
    Uninstall {
        name: String,
        #[arg(short, long)]
        global: bool,
    },
    /// List installed packages and scripts
    List {
        #[arg(short, long)]
        global: bool,
    },
    /// Show information about a package or script
    Info {
        name: String,
        #[arg(short, long)]
        global: bool,
    },
    /// Print settings, or set them (registry=<url>)
    Configure {
        #[arg(value_name = "KEY=VALUE")]
        options: Vec<String>,
    },
}

fn main() {
    if let Err(err) = run(Cli::parse()) {
        eprintln!("scrippolib: {err:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Init => {
            let path = paths::initialize_environment(&std::env::current_dir()?)?;
            println!("initialized {}", path.display());
            Ok(())
        }
        Command::Install { name, global, tag } => run_install(&name, global, tag),
        Command::Update { name, global } => run_update(name.as_deref(), global),
        Command::Uninstall { name, global } => {
            for path in store::uninstall(global, &name)? {
                println!("removed {}", path.display());
            }
            Ok(())
        }
        Command::List { global } => run_list(global),
        Command::Info { name, global } => run_info(&name, global),
        Command::Configure { options } => config::apply_configure(&options),
    }
}

fn run_install(name: &str, global: bool, tag: Option<String>) -> Result<()> {
    let target = source::classify(name);
    let request = Request {
        target: target.clone(),
        tag,
        global,
        registry: Config::load()?.registry,
    };
    let result = install::install(&request);
    if let (Err(err), Target::Registry(_)) = (&result, &target) {
        if std::path::Path::new(name).is_dir() {
            bail!("{err:#}\nhint: to install the directory ./{name}, pass a path such as ./{name}");
        }
    }
    for installed in result? {
        print_installed(&installed, "installed");
    }
    Ok(())
}

fn print_installed(installed: &install::Installed, verb: &str) {
    let names = installed
        .entries
        .iter()
        .map(|(name, version)| format!("{name}@{version}"))
        .collect::<Vec<_>>()
        .join(", ");
    let from = installed
        .replaced
        .as_ref()
        .map(|old| format!(" (was {old})"))
        .unwrap_or_default();
    println!(
        "{verb} {} {names}{from} [{} {}]",
        kind_name(installed.kind),
        installed.tag,
        installed.commit
    );
    for file in &installed.files {
        println!("  {}", file.display());
    }
}

fn run_update(name: Option<&str>, global: bool) -> Result<()> {
    let updates = install::update(name, global)?;
    if updates.is_empty() {
        println!("nothing installed");
        return Ok(());
    }
    let mut failed = 0;
    for update in updates {
        match update.result {
            Ok(Outcome::Installed(installed)) => print_installed(&installed, "updated"),
            Ok(Outcome::UpToDate { names, version }) => {
                println!("{} is up to date ({version})", names.join(", "))
            }
            Err(err) => {
                failed += 1;
                eprintln!("scrippolib: {}: {err:#}", update.label);
            }
        }
    }
    if failed > 0 {
        bail!("{failed} update(s) failed");
    }
    Ok(())
}

fn run_list(global: bool) -> Result<()> {
    let listed = store::list(global)?;
    let width = listed.iter().map(|item| item.name.len()).max().unwrap_or(0);
    for item in listed {
        println!(
            "{:<width$}  {:<8}  {:<7}  {}",
            item.name,
            item.version,
            kind_name(item.kind),
            item.origin
        );
    }
    Ok(())
}

fn run_info(name: &str, global: bool) -> Result<()> {
    let installed = store::find(global, name)?;
    if !installed.is_empty() {
        for group in installed {
            let Some(entry) = group.manifest.entries.iter().find(|e| e.name == name) else {
                continue;
            };
            println!("name:        {}", entry.name);
            println!("version:     {}", entry.version);
            println!("kind:        {}", entry.kind);
            println!("description: {}", entry.description);
            field("author", entry.author.as_deref());
            field("license", entry.license.as_deref());
            field("repository", entry.repository.as_deref());
            match &group.manifest.registry {
                Some(registry) => println!("source:      registry {registry}"),
                None => println!("source:      {}", group.manifest.repository),
            }
            println!("tag:         {} ({})", group.manifest.tag, group.manifest.commit);
            println!("installed:   {}", group.public_file(name).display());
        }
        return Ok(());
    }
    let config = Config::load()?;
    let fetched = registry::fetch(&config.registry)?;
    let found = fetched.find(name);
    if found.is_empty() {
        bail!("'{name}' is not installed and was not found in registry {}", config.registry);
    }
    for entry in found {
        let metadata = &entry.info.metadata;
        println!("name:        {}", entry.info.name);
        println!("version:     {}", source::version_string(&entry.info.version));
        println!("kind:        {}", kind_name(entry.kind));
        println!("description: {}", entry.info.description);
        field("author", metadata.author.as_deref());
        field("license", metadata.license.as_deref());
        field("repository", metadata.repository.as_deref());
        println!("source:      registry {}", config.registry);
        println!("installed:   no (scrippolib install {name})");
    }
    Ok(())
}

fn field(label: &str, value: Option<&str>) {
    if let Some(value) = value {
        println!("{:<12} {value}", format!("{label}:"));
    }
}
