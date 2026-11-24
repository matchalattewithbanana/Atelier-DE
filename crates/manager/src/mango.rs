//! Delegate to the versioned Mango Atelier installer without recreating its policy.
use anyhow::{bail, Context, Result};
use std::{fs, io::{self, IsTerminal, Write}, path::Path, process::Command};

const API: &str = "mango-atelier-setup-v1";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode { Setup, Plan, Rollback }

fn parse(args: &[String]) -> Result<(&Path, Mode)> {
    match args {
        [flag, path] if flag == "--from" && !path.is_empty() => Ok((Path::new(path), Mode::Setup)),
        [flag, path, mode] if flag == "--from" && !path.is_empty() => {
            let mode = match mode.as_str() {
                "--plan" => Mode::Plan,
                "--rollback" => Mode::Rollback,
                _ => bail!("only --plan or --rollback may follow the checkout path"),
            };
            Ok((Path::new(path), mode))
        }
        _ => bail!("usage: atelier-manager mango-setup --from CHECKOUT [--plan|--rollback]"),
    }
}

fn command(root: &Path) -> Command {
    let mut cmd = Command::new("/usr/bin/bash");
    cmd.args(["--noprofile", "--norc"]).arg(root.join("setup.sh"))
        .current_dir(root).env_remove("BASH_ENV").env_remove("ENV")
        .env_remove("ATELIER_ALLOW_ROOT_FOR_TESTS");
    cmd
}

fn check_api(root: &Path) -> Result<()> {
    let output = command(root).arg("--manager-api").output().context("query Mango Atelier setup API")?;
    if !output.status.success() || output.stdout != format!("{API}\n").as_bytes() {
        bail!("unsupported Mango Atelier setup contract; use a trusted checkout with the v1 manager handoff");
    }
    Ok(())
}

pub fn run(args: &[String]) -> Result<()> {
    let (root, mode) = parse(args)?;
    if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("Mango Atelier setup requires an interactive terminal");
    }
    let root = root.canonicalize().context("resolve local Mango Atelier checkout")?;
    if !root.is_dir() || !fs::symlink_metadata(root.join("setup.sh"))?.is_file() {
        bail!("expected a checkout containing a regular setup.sh file");
    }
    println!("Mango Atelier checkout: {}", root.display());
    println!("Mode: {mode:?}. Only run a checkout whose code you trust.");
    println!("The delegated wizard owns compositor/rice selection, package approvals and configuration recovery.");
    println!("The compatibility handshake executes local code; it is not a signature or sandbox.");
    print!("Run this trusted checkout? [y/N] "); io::stdout().flush()?;
    let mut answer = String::new(); io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        println!("Cancelled before launching Mango Atelier.");
        return Ok(());
    }
    check_api(&root)?;
    let mut child = command(&root);
    match mode {
        Mode::Setup => {},
        Mode::Plan => { child.arg("--plan"); },
        Mode::Rollback => { child.arg("--rollback"); },
    }
    let status = child.status().context("launch Mango Atelier setup")?;
    if !status.success() {
        bail!("Mango Atelier returned {status}; inspect its output and use its recovery instructions if needed");
    }
    println!("Mango Atelier wizard exited successfully. Review its output for the actual outcome; no native Atelier deployment was changed.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_interactive_contract_modes_are_accepted() {
        let args = |values: &[&str]| values.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(parse(&args(&["--from", "/tmp/checkout", "--plan"])).unwrap().1, Mode::Plan);
        assert_eq!(parse(&args(&["--from", "/tmp/checkout", "--rollback"])).unwrap().1, Mode::Rollback);
        assert!(parse(&args(&["--from", "/tmp/checkout", "--yes"])).is_err());
        assert!(parse(&args(&["--from", "/tmp/checkout", "--plan", "--rollback"])).is_err());
    }
    #[test]
    fn incompatible_handshake_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("setup.sh"), "printf '%s\\n' mango-atelier-setup-v0\n").unwrap();
        assert!(check_api(root.path()).is_err());
        fs::write(root.path().join("setup.sh"), "printf '%s\\n' mango-atelier-setup-v1\n").unwrap();
        assert!(check_api(root.path()).is_ok());
    }
}
