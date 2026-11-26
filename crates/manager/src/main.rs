//! Initial read-only ecosystem manager and installation diagnostics.
use anyhow::{bail, Result};
mod catalog;
mod customization;
mod customization_registry;
mod customization_discovery;
mod login_theme;
mod login_packs;
mod login_selection;
mod locker;
#[cfg(feature = "gui")]
mod cursor_preview;
mod wizard;
mod packages;
mod deploy;
mod selection;
mod session;
mod mango;
#[cfg(feature = "gui")]
mod rice;
#[cfg(feature = "gui")]
mod live_rice;
mod release;
mod trust;
mod discovery;
mod acquire;
#[cfg(feature = "gui")]
mod gui;
use serde::Serialize;
use std::{env, fs, io::{self, BufRead, IsTerminal, Read, Write}, os::unix::fs::PermissionsExt, path::{Path, PathBuf}};

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

fn guided_packages() -> Result<()> {
    let report = report();
    let result = wizard::run(&mut io::stdin().lock(), &mut io::stdout().lock(), &report)?;
    if let Some(preview) = result {
        println!("Preview accepted. Package changes require separate confirmation.");
        println!("This mode does not build or stage Atelier binaries, install rices or configure sessions.");
        preview.install_packages(&report)?;
    } else {
        println!("Setup cancelled. No changes made.");
    }
    Ok(())
}

fn guided_native(source: &Path) -> Result<()> {
    let result = wizard::run(&mut io::stdin().lock(), &mut io::stdout().lock(), &report())?;
    if let Some(preview) = result {
        println!("Preview accepted. Staging requires a separate confirmation.");
        println!("Only native binaries are staged; package, provider, rice and session preferences are not applied.");
        if let Some(id) = deploy::run(source, preview.requested_products())? {
            println!("Staging complete. Select this deployment or keep your current selection.");
            println!("If selection is declined, the new deployment remains staged.");
            selection::activate(Some(&id), false)?;
        }
    } else {
        println!("Setup cancelled. No changes made.");
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum InstallAction { Packages, Native, Mango, MangoPlan }

fn installer_choice(input: &mut impl BufRead, out: &mut impl Write) -> Result<Option<InstallAction>> {
    loop {
        writeln!(out, "\nAtelier ecosystem installer (experimental)")?;
        writeln!(out, "  1. Install system package dependencies (Arch-family; may upgrade the system)")?;
        writeln!(out, "  2. Stage and select trusted native Atelier build binaries")?;
        writeln!(out, "  3. Mango Atelier setup (MangoWM, Hyprland, Niri and rices)")?;
        writeln!(out, "  4. Preview Mango Atelier setup")?;
        writeln!(out, "  q. Exit")?;
        write!(out, "Choose an independent operation [q]: ")?;
        out.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 { return Ok(None); }
        match line.trim() {
            "1" => return Ok(Some(InstallAction::Packages)),
            "2" => return Ok(Some(InstallAction::Native)),
            "3" => return Ok(Some(InstallAction::Mango)),
            "4" => return Ok(Some(InstallAction::MangoPlan)),
            "" | "q" | "Q" => return Ok(None),
            _ => writeln!(out, "Choose 1, 2, 3, 4 or q.")?,
        }
    }
}

fn installer_path(prompt: &str) -> Result<Option<String>> {
    print!("{prompt} (blank cancels; no shell expansion): ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let path = line.trim_end_matches(['\r', '\n']);
    Ok((!path.trim().is_empty()).then(|| path.to_owned()))
}

fn installer_menu() -> Result<()> {
    if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("the ecosystem installer requires an interactive terminal");
    }
    loop {
        // Release terminal locks before invoking a backend or child process.
        let action = installer_choice(&mut io::stdin().lock(), &mut io::stdout().lock())?;
        match action {
            None => return Ok(()),
            Some(InstallAction::Packages) => guided_packages()?,
            Some(InstallAction::Native) => {
                if let Some(source) = installer_path("Trusted local build directory")? {
                    guided_native(Path::new(&source))?;
                }
            }
            Some(action @ (InstallAction::Mango | InstallAction::MangoPlan)) => {
                if let Some(source) = installer_path("Trusted Mango Atelier checkout directory")? {
                    let mut args = vec!["--from".to_owned(), source];
                    if action == InstallAction::MangoPlan { args.push("--plan".to_owned()); }
                    mango::run(&args)?;
                }
            }
        }
        println!("Operation ended. No other backend will run without another selection.");
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--gui" => {
            #[cfg(feature = "gui")]
            { gui::run()?; }
            #[cfg(not(feature = "gui"))]
            bail!("graphical manager requires a build with --features gui");
        }
        [mode, id, flag] if mode == "deployment-remove" && flag == "--offline" => selection::remove_offline(id)?,
        [mode, product, rest @ ..] if mode == "run-product" => selection::run_product(product, rest)?,
        [mode] if mode == "release-cache" => acquire::list_cache()?,
        [mode, action, id] if mode == "release-cache" && action == "remove" => acquire::remove_cache(id)?,
        [mode] if mode == "deployment-cleanup-plan" => selection::cleanup_plan()?,
        [mode, cache, flag, key] if mode == "release-update" && flag == "--key-id" => acquire::update_cached(cache, key)?,
        [mode, cache, flag, key] if mode == "release-stage" && flag == "--key-id" => acquire::stage_cached(cache, key)?,
        [mode, repo] if mode == "release-discover" => discovery::run(repo, false)?,
        [mode, repo, flag] if mode == "release-discover" && flag == "--prerelease" => discovery::run(repo, true)?,
        [mode, repo, release_id, flag, key] if mode == "release-fetch" && flag == "--key-id" => acquire::run(repo, release_id, key, false)?,
        [mode, repo, release_id, flag, key, pre] if mode == "release-fetch" && flag == "--key-id" && pre == "--prerelease" => acquire::run(repo, release_id, key, true)?,
        [mode] if mode == "release-trust" => trust::list()?,
        [mode, action, key] if mode == "release-trust" && action == "pin" => trust::pin(Path::new(key))?,
        [mode, action, id] if mode == "release-trust" && action == "revoke" => trust::revoke(id)?,
        [mode, manifest, sig_flag, sig, key_flag, id, from, source] if mode == "release-accept" && sig_flag == "--signature" && key_flag == "--key-id" && from == "--from" => trust::accept(Path::new(manifest), Path::new(sig), id, Path::new(source))?,
        [mode, rest @ ..] if mode == "release-manifest" => release::generate(rest)?,
        [mode, manifest, flag, source] if mode == "release-verify" && flag == "--from" => release::verify(Path::new(manifest), Path::new(source))?,
        [mode, manifest, sig_flag, sig, key_flag, key, from, source] if mode == "release-verify-signed" && sig_flag == "--signature" && key_flag == "--key" && from == "--from" => release::verify_signed(Path::new(manifest), Path::new(sig), Path::new(key), Path::new(source))?,
        [mode] if mode == "session-check" => session::check()?,
        [mode, backend] if mode == "session-launch" && backend == "--nested" => session::launch(false)?,
        [mode, backend, flag] if mode == "session-launch" && backend == "--nested" && flag == "--no-shell" => session::launch(true)?,
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
        [mode, rest @ ..] if mode == "locker" => locker::run(rest)?,
        [mode, rest @ ..] if mode == "login-theme" => login_theme::run(rest)?,
        [mode, rest @ ..] if mode == "customizations" => customization::run(rest)?,
        [mode] if mode == "catalog" => {
            for product in catalog::PRODUCTS {
                println!("{} — {} [{}]\n  {}", product.id, product.name, product.source, product.note);
            }
        }
        [mode, flag] if mode == "catalog" && flag == "--json" => {
            println!("{}", serde_json::to_string_pretty(&catalog::PRODUCTS)?);
        }
        [mode, rest @ ..] if mode == "plan" => {
            let json = rest.last().is_some_and(|arg| arg == "--json");
            let selected = if json { &rest[..rest.len() - 1] } else { rest };
            let plan = catalog::plan(selected, &report())?;
            if json { println!("{}", serde_json::to_string_pretty(&plan)?); }
            else {
                println!("Atelier installation preview — no changes");
                for step in &plan.steps {
                    println!("  {}: {} [{}]", step.name, step.status, step.source);
                    if !step.missing_binaries.is_empty() { println!("    Missing: {}", step.missing_binaries.join(", ")); }
                }
                for warning in &plan.warnings { println!("Note: {warning}"); }
            }
        }
        [mode, rest @ ..] if mode == "packages" => {
            let json = rest.last().is_some_and(|arg| arg == "--json");
            let selected = if json { &rest[..rest.len() - 1] } else { rest };
            let report = report();
            let plan = catalog::plan(selected, &report)?;
            let preview = packages::preview(&plan, &report, &[]);
            if json { println!("{}", serde_json::to_string_pretty(&preview)?); }
            else {
                println!("Package candidates — no installation");
                for candidate in &preview.candidates { println!("  {}: {} ({})", candidate.package, candidate.state, candidate.reasons.join(", ")); }
                for note in &preview.notes { println!("Note: {note}"); }
            }
        }
        [mode, products @ ..] if mode == "install-packages" => {
            let report = report();
            let (products, providers) = packages::parse_install_args(products)?;
            let plan = catalog::plan(&products, &report)?;
            packages::install(&plan, &report, &providers)?;
        }
        [mode, flag] if mode == "setup" && flag == "--packages" => guided_packages()?,
        [mode, flag] if mode == "setup" && flag == "--install" => installer_menu()?,
        [mode, rest @ ..] if mode == "mango-setup" => mango::run(rest)?,
        [mode] if mode == "setup" => {
            let result = wizard::run(&mut io::stdin().lock(), &mut io::stdout().lock(), &report())?;
            println!("{}", if result.is_some() { "Preview accepted. No changes made." } else { "Setup cancelled. No changes made." });
        }
        [mode, flag, source] if mode == "setup" && flag == "--from" => guided_native(Path::new(source))?,
        [mode, flag] if mode == "setup" && flag == "--json" => {
            let result = wizard::run(&mut io::stdin().lock(), &mut io::stderr().lock(), &report())?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        [mode, flag, source, products @ ..] if (mode == "update" || mode == "update-plan") && flag == "--from" => deploy::update(Path::new(source), products, mode == "update-plan")?,
        [mode, flag, source, products @ ..] if mode == "stage" && flag == "--from" => { deploy::run(Path::new(source), products)?; }
        [mode] if mode == "package-history" => packages::show_history(false)?,
        [mode, flag] if mode == "package-history" && flag == "--json" => packages::show_history(true)?,
        [mode] if mode == "recovery-status" => {
            let native = selection::recovery().unwrap_or_else(|error| serde_json::json!({"error":format!("{error:#}")}));
            let packages = match packages::history() {
                Ok(entries) => serde_json::json!({"transactions":entries}),
                Err(error) => serde_json::json!({"error":format!("{error:#}")}),
            };
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({"schema":1,"native":native,"packages":packages}))?);
        }
        [mode] if mode == "deployments" => selection::list(false)?,
        [mode, flag] if mode == "deployments" && flag == "--json" => selection::list(true)?,
        [mode, id] if mode == "deployment-info" => selection::info(id)?,
        [mode, id] if mode == "verify" => selection::verify(id)?,
        [mode, id] if mode == "activate" => selection::activate(Some(id), false)?,
        [mode, id, flag] if mode == "activate" && flag == "--historical" => selection::activate(Some(id), true)?,
        [mode] if mode == "rollback" => selection::activate(None, false)?,
        [mode] if mode == "active-path" => selection::active_path()?,
        [flag] if flag == "--help" || flag == "-h" => println!("atelier-manager --gui (GTK build)\natelier-manager doctor [--json]\natelier-manager mango-setup --from CHECKOUT [--plan|--rollback]\natelier-manager catalog [--json]\natelier-manager customizations [list [CATEGORY] [--json] | preview ID [--json] | discover CATEGORY [KEYWORDS] [--json] | register --from FILE | register-source CATEGORY OWNER/REPO | unregister ID | restore-registry]\natelier-manager locker check [--json]\natelier-manager login-theme inspect --from DIR | prepare --from DIR | pack-prepare ID | selection-plan NAME | select NAME | rollback-selection\natelier-manager plan PRODUCT [PRODUCT...] [--json]\natelier-manager setup [--json]\natelier-manager setup --install\natelier-manager setup --from BUILD_DIR\natelier-manager setup --packages\natelier-manager packages PRODUCT [PRODUCT...] [--json]\natelier-manager install-packages PRODUCT [PRODUCT...] [--provider ID ...]\natelier-manager stage --from BUILD_DIR PRODUCT [PRODUCT...]\natelier-manager (update-plan | update) --from BUILD_DIR PRODUCT [PRODUCT...]\natelier-manager deployments [--json] | deployment-info ID | verify ID | activate ID [--historical] | rollback | active-path\natelier-manager session-check | session-launch --nested [--no-shell]\natelier-manager run-product PRODUCT [ARG...]\natelier-manager package-history [--json] | recovery-status\natelier-manager release-manifest --from BUILD_DIR --version VERSION --revision SHA PRODUCT [PRODUCT...]\natelier-manager release-verify MANIFEST --from BUILD_DIR\natelier-manager release-verify-signed MANIFEST --signature SIG --key PUBLIC_KEY --from BUILD_DIR\natelier-manager release-trust [pin PUBLIC_KEY | revoke KEY_ID]\natelier-manager release-discover native [--prerelease]\natelier-manager release-fetch native RELEASE_ID --key-id KEY_ID [--prerelease]\natelier-manager release-stage CACHE_ID --key-id KEY_ID\natelier-manager release-update CACHE_ID --key-id KEY_ID\natelier-manager release-cache [remove CACHE_ID]\natelier-manager deployment-cleanup-plan\natelier-manager deployment-remove ID --offline\natelier-manager release-accept MANIFEST --signature SIG --key-id KEY_ID --from BUILD_DIR\natelier-manager release-manifest --from BUILD_DIR --version VERSION --revision SHA --sequence N PRODUCT [PRODUCT...]\nPlain setup is preview-only; setup --install opens the interactive ecosystem menu; setup --packages offers a separately confirmed system package transaction; setup --from offers native staging and selection."),
        _ => bail!("usage: atelier-manager --help"),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installer_menu_requires_explicit_backend_selection() {
        for (answer, expected) in [
            ("1\n", Some(InstallAction::Packages)), ("2\n", Some(InstallAction::Native)),
            ("3\n", Some(InstallAction::Mango)), ("4\n", Some(InstallAction::MangoPlan)),
            ("invalid\nq\n", None), ("\n", None), ("", None),
        ] {
            assert_eq!(installer_choice(&mut io::Cursor::new(answer), &mut Vec::new()).unwrap(), expected);
        }
    }
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
