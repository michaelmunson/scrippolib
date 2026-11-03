//! scrippolib settings, stored as `key=value` lines in `$SCRIPPO_HOME/scrippolib.config`
//! (override the path with `SCRIPPOLIB_CONFIG`).
use anyhow::{Context, Result, bail};
use scriptopotamus::paths;
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_REGISTRY: &str = "https://github.com/michaelmunson/scrippolib-registry";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub registry: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            registry: DEFAULT_REGISTRY.to_string(),
        }
    }
}

pub fn config_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SCRIPPOLIB_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    Ok(paths::scrippo_home()?.join("scrippolib.config"))
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        let mut config = Self::default();
        if !path.exists() {
            return Ok(config);
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .with_context(|| format!("{}:{}: expected key=value", path.display(), index + 1))?;
            config.set(key.trim(), value.trim())?;
        }
        Ok(config)
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&path, self.display())
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(path)
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "registry" => {
                if value.is_empty() || value.contains(['\n', '\r']) {
                    bail!("registry must be a single-line URL or directory");
                }
                if value.starts_with('-') {
                    bail!("registry must not start with '-'");
                }
                self.registry = value.to_string();
            }
            _ => bail!("unknown config key '{key}' (expected registry)"),
        }
        Ok(())
    }

    pub fn display(&self) -> String {
        format!("registry={}\n", self.registry)
    }
}

/// Apply `key=value` pairs and save. With no pairs, print the current settings.
pub fn apply_configure(pairs: &[String]) -> Result<()> {
    let mut config = Config::load()?;
    if pairs.is_empty() {
        print!("{}", config.display());
        return Ok(());
    }
    for pair in pairs {
        let (key, value) = pair
            .split_once('=')
            .with_context(|| format!("expected key=value, got '{pair}'"))?;
        config.set(key.trim(), value.trim())?;
    }
    let path = config.save()?;
    println!("updated {}", path.display());
    print!("{}", config.display());
    Ok(())
}
