//! Explicit user-owned shell profiles. No imported profile is executed automatically.
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};
use crate::Shell;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    version: u32,
    #[serde(default)]
    profiles: BTreeMap<String, Shell>,
}
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 48 && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
fn validate_shell(shell: &Shell) -> Result<()> {
    let program = &shell.program;
    if program.trim().is_empty() || program.trim() != program || program.starts_with('-') || program.contains('\0')
        || (program.contains('/') && !Path::new(program).is_absolute()) {
        bail!("shell executable must be a bare name or absolute path");
    }
    if program.len() > 4096 || shell.args.len() > 64 || shell.args.iter().any(|arg| arg.len() > 4096 || arg.contains('\0')) {
        bail!("shell profile exceeds argument limits");
    }
    Ok(())
}
fn parse(text: &str) -> Result<BTreeMap<String, Shell>> {
    if text.len() > 256 * 1024 { bail!("session profile file exceeds 256 KiB"); }
    let file: ProfileFile = toml::from_str(text).context("parse session profile file")?;
    if file.version != 1 { bail!("unsupported session profile version {}", file.version); }
    if file.profiles.len() > 64 { bail!("too many session profiles"); }
    for (name, shell) in &file.profiles {
        if !valid_name(name) { bail!("invalid session profile name: {name}"); }
        validate_shell(shell).with_context(|| format!("invalid shell profile {name}"))?;
    }
    Ok(file.profiles)
}
fn file_path() -> Result<PathBuf> {
    let settings = atelier_config::config_path()?;
    Ok(settings.with_file_name("session-profiles.toml"))
}
pub fn load() -> Result<BTreeMap<String, Shell>> {
    use std::io::Read;
    let path = file_path()?;
    match fs::metadata(&path) {
        Ok(metadata) if !metadata.is_file() => bail!("session profile path is not a regular file"),
        Ok(_) => {},
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error).with_context(|| format!("inspect {}", path.display())),
    }
    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    if !file.metadata()?.is_file() { bail!("session profile path is not a regular file"); }
    let mut text = String::new();
    file.take(256 * 1024 + 1).read_to_string(&mut text).with_context(|| format!("read {}", path.display()))?;
    parse(&text).with_context(|| format!("validate {}", path.display()))
}
pub fn selected(name: &str) -> Result<Shell> {
    if !valid_name(name) { bail!("invalid session profile name"); }
    load()?.remove(name).with_context(|| format!("unknown session profile {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_arguments_and_sorted_names() {
        let profiles = parse(r#"
version = 1
[profiles.classic]
program = "quickshell"
args = ["-c", "my profile", "$HOME"]
[profiles.native]
program = "/usr/bin/atelier-shell"
"#).unwrap();
        assert_eq!(profiles["classic"].args, vec!["-c", "my profile", "$HOME"]);
        assert!(profiles["native"].args.is_empty());
    }
    #[test]
    fn rejects_unknown_version_relative_paths_and_unknown_fields() {
        for value in [
            "version=2",
            "version=1\n[profiles.test]\nprogram='./shell'",
            "version=1\n[profiles.test]\nprogram='shell'\nunknown=true",
            "version=1\n[profiles.\"bad name\"]\nprogram='shell'",
        ] { assert!(parse(value).is_err(), "accepted {value}"); }
    }
}
