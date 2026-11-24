//! Initial read-only ecosystem manager and installation diagnostics.
use anyhow::{bail, Result};
use serde::Serialize;
use std::{env, fs, io::{self, Read}, os::unix::fs::PermissionsExt, path::{Path, PathBuf}};

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
struct Distribution {
    id: Option<String>,
    id_like: Vec<String>,
    version: Option<String>,
}

#[derive(Debug, Serialize)]
struct Executable { name: &'static str, path: Option<PathBuf> }

#[derive(Debug, Serialize)]
struct Report {
    schema: u32,
    distribution: Distribution,
    distribution_error: Option<String>,
    architecture: &'static str,
    session_type: Option<String>,
    desktop: Option<String>,
    package_managers: Vec<Executable>,
    products: Vec<Executable>,
    compositors: Vec<Executable>,
    optional_tools: Vec<Executable>,
}

fn token(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}
fn unquote(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.starts_with('"') { value.strip_prefix('"')?.strip_suffix('"') }
    else if value.starts_with('\'') { value.strip_prefix('\'')?.strip_suffix('\'') }
    else { Some(value) }
}
fn parse_os_release(text: &str) -> Distribution {
    let mut result = Distribution::default();
    for line in text.lines() {
        let Some((key, raw)) = line.split_once('=') else { continue; };
        let Some(value) = unquote(raw) else { continue; };
        match key.trim() {
            "ID" if token(value) => result.id = Some(value.to_owned()),
            "ID_LIKE" => result.id_like = value.split_whitespace().filter(|part| token(part)).map(str::to_owned).collect(),
            "VERSION_ID" if token(value) => result.version = Some(value.to_owned()),
            _ => {}
        }
    }
    result
}

fn read_distribution() -> io::Result<Distribution> {
    let file = match fs::File::open("/etc/os-release") {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::File::open("/usr/lib/os-release")?,
        Err(error) => return Err(error),
    };
    let mut text = String::new();
    file.take(64 * 1024 + 1).read_to_string(&mut text)?;
    if text.len() > 64 * 1024 { return Err(io::Error::new(io::ErrorKind::InvalidData, "os-release exceeds 64 KiB")); }
    Ok(parse_os_release(&text))
}

fn find(name: &'static str, dirs: &[PathBuf]) -> Executable {
    let path = dirs.iter().filter(|dir| dir.is_absolute()).map(|dir| dir.join(name)).find(|path| {
        fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    });
    Executable { name, path }
}
fn inventory(names: &[&'static str], dirs: &[PathBuf]) -> Vec<Executable> {
    names.iter().map(|name| find(name, dirs)).collect()
}
fn report() -> Report {
    let (distribution, distribution_error) = match read_distribution() {
        Ok(value) => (value, None),
        Err(error) => (Distribution::default(), Some(error.to_string())),
    };
    let path_dirs: Vec<_> = env::var_os("PATH").map(|value| env::split_paths(&value).collect()).unwrap_or_default();
    let mut product_dirs = Vec::new();
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() { product_dirs.push(parent.to_path_buf()); }
    }
    if let Some(home) = env::var_os("HOME").map(PathBuf::from).filter(|path| path.is_absolute()) {
        product_dirs.push(home.join(".local/bin"));
    }
    product_dirs.extend(path_dirs.iter().cloned());
    Report {
        schema: 1, distribution, distribution_error, architecture: env::consts::ARCH,
        session_type: env::var("XDG_SESSION_TYPE").ok(), desktop: env::var("XDG_CURRENT_DESKTOP").ok(),
        package_managers: inventory(&["pacman", "apt", "dnf", "zypper", "nix"], &path_dirs),
        products: inventory(&["atelier-compositor", "atelier-shell", "atelier-session", "atelier-settings", "atelier-services", "atelier-notifications", "atelier-manager", "atelier", "atelier-workspace"], &product_dirs),
        compositors: inventory(&["mango", "Hyprland", "niri", "quickshell", "qs"], &product_dirs),
        optional_tools: inventory(&["playerctl", "pactl", "nmcli", "bluetoothctl", "timeout", "git", "cargo"], &path_dirs),
    }
}
fn show_group(title: &str, entries: &[Executable]) {
    println!("\n{title}");
    for entry in entries {
        match &entry.path {
            Some(path) => println!("  {}: found ({})", entry.name, path.display()),
            None => println!("  {}: not found", entry.name),
        }
    }
}
fn main() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.as_slice() {
        [mode] if mode == "doctor" => {
            let report = report();
            println!("Atelier ecosystem diagnostics — read-only");
            println!("Distribution: {} {}", report.distribution.id.as_deref().unwrap_or("unknown"), report.distribution.version.as_deref().unwrap_or(""));
            if let Some(error) = &report.distribution_error { println!("Distribution detection unavailable: {error}"); }
            println!("Architecture: {}", report.architecture);
            println!("Session: {} / {}", report.session_type.as_deref().unwrap_or("unknown"), report.desktop.as_deref().unwrap_or("unknown"));
            show_group("Package managers", &report.package_managers);
            show_group("Atelier products and compatibility helpers", &report.products);
            show_group("External desktop components", &report.compositors);
            show_group("Optional providers and tools", &report.optional_tools);
            println!("\nPresence is not a compatibility or runtime readiness guarantee. No commands were executed.");
        }
        [mode, flag] if mode == "doctor" && flag == "--json" => println!("{}", serde_json::to_string_pretty(&report())?),
        [flag] if flag == "--help" || flag == "-h" => println!("atelier-manager doctor [--json]\nRead-only inventory; no setup or package changes."),
        _ => bail!("usage: atelier-manager doctor [--json]"),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn os_release_is_parsed_without_shell_expansion() {
        let distro = parse_os_release("ID=cachyos\nID_LIKE=\"arch linux\"\nVERSION_ID='2026.11'\nPRETTY_NAME=$(touch file)\n");
        assert_eq!(distro.id.as_deref(), Some("cachyos"));
        assert_eq!(distro.id_like, vec!["arch", "linux"]);
        assert_eq!(distro.version.as_deref(), Some("2026.11"));
        assert_eq!(parse_os_release("ID=$(anything)").id, None);
    }
    #[test]
    fn only_executable_regular_files_in_absolute_directories_are_found() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("tool");
        fs::write(&binary, "").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(find("tool", &[dir.path().to_owned()]).path.is_none());
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(find("tool", &[dir.path().to_owned()]).path, Some(binary));
        assert!(find("tool", &[PathBuf::from(".")]).path.is_none());
        assert!(find("missing", &[dir.path().to_owned()]).path.is_none());
    }
}
