//! Shared, versioned user preferences. No compositor, GUI or service dependency.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{env, fs, io::Write, path::{Path, PathBuf}, process::Command};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    #[default]
    Midnight,
    Daylight,
    Dusk,
}

impl Theme {
    pub fn name(self) -> &'static str {
        match self {
            Self::Midnight => "Midnight",
            Self::Daylight => "Daylight",
            Self::Dusk => "Dusk",
        }
    }

    pub fn is_dark(self) -> bool { self != Self::Daylight }

    /// Linear display clear colour for the initial nested scene.
    pub fn background(self) -> [f32; 4] {
        match self {
            Self::Midnight => [0.035, 0.05, 0.085, 1.0],
            Self::Daylight => [0.87, 0.90, 0.95, 1.0],
            Self::Dusk => [0.11, 0.065, 0.14, 1.0],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub theme: Theme,
    pub terminal: String,
    pub start_shell: bool,
    pub layout: Layout,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    #[default]
    Floating,
    Grid,
}

impl Default for Config {
    fn default() -> Self {
        Self { version: 1, theme: Theme::Midnight, terminal: "foot".into(), start_shell: true, layout: Layout::Floating }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 { bail!("unsupported Atelier config version {}", self.version); }
        if self.terminal.trim().is_empty() || self.terminal.contains('\0') {
            bail!("terminal must be a non-empty executable name or path");
        }
        if self.terminal.trim() != self.terminal { bail!("terminal cannot have leading or trailing whitespace"); }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let value = match fs::read_to_string(path) {
            Ok(text) => toml::from_str::<Self>(&text).with_context(|| format!("parse {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).context("config path has no parent")?;
        fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(path).with_context(|| format!("replace {}", path.display()))?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
}

pub fn config_path() -> Result<PathBuf> {
    let base = match env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
        Some(p) => p,
        None => PathBuf::from(env::var_os("HOME").context("HOME is not set")?).join(".config"),
    };
    Ok(base.join("atelier-de/settings.toml"))
}

/// Find sibling binaries in an installed prefix or cargo build directory; otherwise use PATH.
pub fn component(name: &str) -> Command {
    if let Ok(exe) = env::current_exe() {
        let sibling = exe.with_file_name(name);
        if sibling.is_file() { return Command::new(sibling); }
    }
    Command::new(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config/settings.toml");
        assert_eq!(Config::load(&path).unwrap(), Config::default());
        let config = Config { theme: Theme::Daylight, terminal: "/usr/bin/kitty".into(), start_shell: false, ..Config::default() };
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
    }

    #[test]
    fn invalid_value_cannot_replace_saved_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let original = Config::default();
        original.save(&path).unwrap();
        let invalid = Config { version: 99, ..original.clone() };
        assert!(invalid.save(&path).is_err());
        assert_eq!(Config::load(&path).unwrap(), original);
    }

    #[test]
    fn malformed_config_is_reported_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        fs::write(&path, "theme = [").unwrap();
        assert!(Config::load(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "theme = [");
    }
}
