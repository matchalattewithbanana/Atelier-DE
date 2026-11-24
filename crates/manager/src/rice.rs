//! Optional trusted-checkout integration with Mango Atelier's existing Rice Manager.
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{fs, io::{self, Read}, os::unix::fs::{MetadataExt, PermissionsExt}, path::{Path, PathBuf}, process::{Command, Stdio}, thread};

const MAX_OUTPUT: u64 = 2 * 1024 * 1024;
const COMPOSITORS: &[&str] = &["mango", "hyprland", "niri"];
const MARKERS: &[&str] = &["MANGO_INSTANCE_SIGNATURE", "HYPRLAND_INSTANCE_SIGNATURE", "NIRI_SOCKET"];
#[derive(Clone, Copy)]
pub(super) enum Action { Local, Discover, Snapshots, Prepare, Select }
pub(super) struct Item { pub title: String, pub status: String, pub detail: String }

#[derive(Deserialize)]
struct Catalog { compositor: String, rices: Vec<Profile>, warnings: Vec<String> }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    id: String, name: String, summary: String, ready: bool, selected: bool, supported: bool,
    compositors: Vec<String>, install_method: String, missing_tools: Vec<String>,
}
#[derive(Deserialize)]
struct Community { status: String, rices: Vec<Listing> }
#[derive(Deserialize)]
struct Listing { name: String, author: String, source: String, compatibility: String }
#[derive(Deserialize)]
struct Snapshot { id: String, rice: String, name: String, files: u64 }

fn valid_id(id: &str) -> bool {
    (2..=48).contains(&id.len()) && id.as_bytes()[0].is_ascii_lowercase()
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
}
fn trusted_tool(path: &str) -> Result<()> {
    let m = fs::metadata(path).with_context(|| format!("required system tool unavailable: {path}"))?;
    if !m.is_file() || m.uid() != 0 || m.permissions().mode() & 0o022 != 0 || m.permissions().mode() & 0o111 == 0 {
        bail!("unsafe system tool: {path}");
    }
    Ok(())
}
fn read_limited(stream: impl Read, limit: u64) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    stream.take(limit + 1).read_to_end(&mut data)?;
    Ok(data)
}
fn capture(mut command: Command) -> Result<Vec<u8>> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;
    let worker = thread::spawn(move || read_limited(stderr, 16 * 1024));
    let out = read_limited(stdout, MAX_OUTPUT);
    let status = child.wait()?;
    let err = worker.join().map_err(|_| anyhow::anyhow!("stderr reader failed"))??;
    let out = out?;
    if out.len() as u64 > MAX_OUTPUT { bail!("rice response exceeds size limit"); }
    if !status.success() {
        bail!("Mango Atelier returned {status}: {}", String::from_utf8_lossy(&err));
    }
    Ok(out)
}

struct Checkout { root: PathBuf }
impl Checkout {
    fn open(path: &Path) -> Result<Self> {
        if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
        if !path.is_absolute() { bail!("enter the absolute path to a trusted Mango Atelier checkout"); }
        let root = path.canonicalize()?;
        for file in ["setup.sh", "scripts/rice_manager.py"] {
            if !fs::symlink_metadata(root.join(file))?.is_file() { bail!("checkout lacks regular {file}"); }
        }
        for path in ["/usr/bin/bash", "/usr/bin/python3", "/usr/bin/timeout"] { trusted_tool(path)?; }
        let checkout = Self { root };
        let mut cmd = checkout.command("25s");
        cmd.args(["/usr/bin/bash", "--noprofile", "--norc"]).arg(checkout.root.join("setup.sh")).arg("--manager-api");
        if capture(cmd)? != format!("{}\n", crate::mango::API).as_bytes() {
            bail!("unsupported Mango Atelier checkout; use the trusted integration branch");
        }
        Ok(checkout)
    }
    fn command(&self, limit: &str) -> Command {
        let mut cmd = Command::new("/usr/bin/timeout");
        cmd.args(["--kill-after=2s", limit]).current_dir(&self.root)
            .env_remove("BASH_ENV").env_remove("ENV").env_remove("ATELIER_ALLOW_ROOT_FOR_TESTS")
            .env_remove("PYTHONPATH").env_remove("PYTHONHOME");
        cmd
    }
    fn run(&self, target: &str, args: &[&str], limit: &str) -> Result<Vec<u8>> {
        if !COMPOSITORS.contains(&target) { bail!("unsupported compositor"); }
        let mut cmd = self.command(limit);
        // Catalog/next-login operations are explicitly offline-targeted;
        // never inherit an unrelated live compositor's detection markers.
        for marker in MARKERS { cmd.env_remove(marker); }
        cmd.env("ATELIER_COMPOSITOR", target)
            .args(["/usr/bin/python3", "-E", "-s", "-B"]).arg(self.root.join("scripts/rice_manager.py")).args(args);
        capture(cmd)
    }
    fn catalog(&self, target: &str) -> Result<Catalog> {
        parse_catalog(&self.run(target, &["list", "--json"], "25s")?, target)
    }
}
fn parse_catalog(bytes: &[u8], target: &str) -> Result<Catalog> {
    let data: Catalog = serde_json::from_slice(bytes).context("unsupported rice catalog response")?;
    if data.compositor != target || data.rices.len() > 1024 || data.rices.iter().any(|r| !valid_id(&r.id)) {
        bail!("invalid or mismatched rice catalog");
    }
    let mut ids = std::collections::BTreeSet::new();
    if data.rices.iter().any(|r| !ids.insert(&r.id)) { bail!("duplicate rice ID"); }
    Ok(data)
}
fn mutation_args(action: Action, target: &str, id: &str) -> Result<Vec<String>> {
    if !valid_id(id) || !COMPOSITORS.contains(&target) { bail!("invalid rice or compositor ID"); }
    let mut args = vec![match action {
        Action::Prepare if id == "sekaipop" => "install",
        Action::Prepare => bail!("only the curated SekaiPop preparation is integrated"),
        Action::Select => "use",
        _ => bail!("not a mutation"),
    }.into(), id.into(), "--compositor".into(), target.into()];
    if matches!(action, Action::Select) { args.push("--next-login".into()); }
    Ok(args)
}
pub(super) fn execute(path: &Path, target: &str, action: Action, id: &str) -> Result<Vec<Item>> {
    if !COMPOSITORS.contains(&target) { bail!("select a supported target"); }
    let checkout = Checkout::open(path)?;
    let mut items = Vec::new();
    match action {
        Action::Local => {
            let catalog = checkout.catalog(target)?;
            for r in catalog.rices {
                let status = if !r.supported { "Unsupported" } else if r.selected { if r.ready { "Selected · ready" } else { "Selected · needs setup" } } else if r.ready { "Ready" } else { "Needs setup" };
                items.push(Item { title: r.name, status: status.into(), detail: format!("ID: {} · {}\n{}\nCompatible: {}\nMissing tools: {}", r.id, r.install_method, r.summary, r.compositors.join(", "), r.missing_tools.join(", ")) });
            }
            for warning in catalog.warnings { items.push(Item { title: "Registry warning".into(), status: "Ignored invalid rice".into(), detail: warning }); }
        }
        Action::Discover => {
            let data: Community = serde_json::from_slice(&checkout.run(target, &["discover", "--refresh", "--json"], "30s")?)?;
            items.push(Item { title: "Rice Cooker".into(), status: data.status, detail: "Community metadata only; no community install commands are accepted. Hyprland listings are not proof of compatibility.".into() });
            for entry in data.rices { items.push(Item { title: entry.name, status: entry.compatibility, detail: format!("By {}\n{}", entry.author, entry.source) }); }
        }
        Action::Snapshots => {
            let data: Vec<Snapshot> = serde_json::from_slice(&checkout.run(target, &["snapshot", "list", "--json"], "30s")?)?;
            for entry in data { items.push(Item { title: entry.name, status: format!("{} · {} files", entry.rice, entry.files), detail: entry.id }); }
        }
        Action::Prepare | Action::Select => {
            let args = mutation_args(action, target, id)?;
            let catalog = checkout.catalog(target)?;
            let profile = catalog.rices.iter().find(|r| r.id == id).context("rice not found in local catalog")?;
            if !profile.supported { bail!("rice is unsupported on the selected compositor"); }
            if matches!(action, Action::Prepare) && profile.install_method != "managed" { bail!("rice is not a managed profile"); }
            if matches!(action, Action::Select) && !profile.ready { bail!("prepare and validate the rice before selecting it"); }
            let args: Vec<_> = args.iter().map(String::as_str).collect();
            let output = checkout.run(target, &args, "180s")?;
            items.push(Item { title: "Mango Atelier operation completed".into(), status: "Review delegated output".into(), detail: String::from_utf8_lossy(&output).into_owned() });
        }
    }
    Ok(items)
}

/// Snapshot operations are a closed vocabulary; arbitrary CLI flags and
/// community-provided paths cannot become delegated commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SnapshotAction { List, Inspect, Create, Import, Export, Rename, Restore, Delete }

fn valid_snapshot_id(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 29 && b[8] == b'T' && b[15] == b'Z' && b[16] == b'-'
        && b[..8].iter().chain(&b[9..15]).all(u8::is_ascii_digit)
        && b[17..].iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
}
fn snapshot_label(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        bail!("snapshot name must have 1–80 characters without control characters");
    }
    Ok(())
}
fn archive_path(path: &str) -> Result<()> {
    let p = Path::new(path);
    if !p.is_absolute() || p.extension().and_then(|s| s.to_str()) != Some("zip")
        || path.chars().any(char::is_control) {
        bail!("enter an absolute .zip archive path");
    }
    Ok(())
}
impl SnapshotAction {
    /// Fields are validated before constructing any argument list.
    fn arguments(self, id: &str, value: &str) -> Result<Vec<String>> {
        let mut args = vec!["snapshot".into()];
        if matches!(self, Self::Inspect | Self::Export | Self::Rename | Self::Restore | Self::Delete)
            && !valid_snapshot_id(id) { bail!("enter an exact snapshot ID from the inventory"); }
        match self {
            Self::List => args.extend(["list".into(), "--json".into()]),
            Self::Inspect => args.extend(["show".into(), id.into()]),
            Self::Create => {
                if !valid_id(id) { bail!("enter a registered rice ID"); }
                snapshot_label(value)?;
                args.extend(["create".into(), id.into(), format!("--name={value}")]);
            }
            Self::Import => {
                archive_path(value)?;
                args.extend(["import".into(), value.into()]);
            }
            Self::Export => {
                archive_path(value)?;
                args.extend(["export".into(), id.into(), value.into()]);
            }
            Self::Rename => {
                snapshot_label(value)?;
                args.extend(["rename".into(), id.into(), "--".into(), value.into()]);
            }
            Self::Restore => args.extend(["restore".into(), id.into()]),
            Self::Delete => args.extend(["delete".into(), id.into()]),
        }
        Ok(args)
    }

    pub(super) fn phrase(self, id: &str, value: &str) -> Result<Option<String>> {
        self.arguments(id, value)?;
        Ok(match self {
            Self::List | Self::Inspect => None,
            Self::Create => Some(format!("create snapshot {id}")),
            Self::Import => Some("import archive".into()),
            Self::Export => Some(format!("export {id}")),
            Self::Rename => Some(format!("rename {id}")),
            Self::Restore => Some(format!("restore offline {id}")),
            Self::Delete => Some(format!("delete {id}")),
        })
    }
}
fn live_desktop_present() -> bool {
    MARKERS.iter().any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty()))
        || ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP"].iter().any(|key| {
            std::env::var(key).unwrap_or_default().split(':')
                .any(|name| matches!(name.to_ascii_lowercase().as_str(), "mango" | "mangowm" | "hyprland" | "niri"))
        })
}
pub(super) fn snapshots(path: &Path, action: SnapshotAction, id: &str, value: &str, confirmation: &str) -> Result<Vec<Item>> {
    let args = action.arguments(id, value)?;
    if let Some(expected) = action.phrase(id, value)? {
        if confirmation != expected { bail!("type exactly: {expected}"); }
    }
    // Do not clear live detection markers to bypass the delegated restore guard.
    // A different or unmanaged session may still exist: typed offline assurance
    // is required in addition to this conservative environment check.
    if action == SnapshotAction::Restore && live_desktop_present() {
        bail!("restore from a separate desktop after closing all Mango Atelier sessions and previews");
    }
    let checkout = Checkout::open(path)?;
    // Snapshot store and rice configuration are shared across compositors.
    // Restore is offline only; never pass the backend's --allow-active switch.
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    let output = checkout.run("mango", &args, "180s")?;
    if action == SnapshotAction::List {
        let data: Vec<Snapshot> = serde_json::from_slice(&output)?;
        return Ok(data.into_iter().map(|entry| Item {
            title: entry.name, status: format!("{} · {} files", entry.rice, entry.files), detail: entry.id,
        }).collect());
    }
    let (title, status) = match action {
        SnapshotAction::Inspect => ("Snapshot manifest", "Validated by Mango Atelier"),
        SnapshotAction::Restore => ("Restore result", "Review recovery snapshot ID"),
        SnapshotAction::Import => ("Import result", "Stored only; not restored"),
        SnapshotAction::Export => ("Export result", "Private archive; review before sharing"),
        SnapshotAction::Create => ("Snapshot creation", "Saved"),
        SnapshotAction::Rename => ("Rename result", "Completed"),
        SnapshotAction::Delete => ("Deletion result", "Completed"),
        SnapshotAction::List => unreachable!(),
    };
    Ok(vec![Item { title: title.into(), status: status.into(), detail: String::from_utf8_lossy(&output).into_owned() }])
}

/// The Classic backend owns the palette and its recovery transaction.
/// Only these documented editable fields may be sent to it.
pub(super) const APPEARANCE_FIELDS: &[(&str, &str, &str, Option<(u32,u32)>)] = &[
    ("accent", "--accent", "Accent color (#RRGGBB)", None),
    ("radius", "--radius", "Corner radius", Some((0,30))),
    ("innerGap", "--inner-gap", "Inner gap", Some((0,36))),
    ("outerGap", "--outer-gap", "Outer gap", Some((0,48))),
    ("barHeight", "--bar-height", "Panel height", Some((32,64))),
    ("dockHeight", "--dock-height", "Dock height", Some((40,84))),
    ("panelOpacity", "--panel-opacity", "Panel opacity (%)", Some((60,100))),
    ("fontSize", "--font-size", "Font size", Some((10,18))),
    ("fontFamily", "--font-family", "Font family", None),
    ("background", "--background", "Background color (#RRGGBB)", None),
    ("surface", "--surface", "Surface color (#RRGGBB)", None),
    ("elevated", "--elevated", "Elevated color (#RRGGBB)", None),
    ("text", "--text", "Text color (#RRGGBB)", None),
    ("muted", "--muted", "Muted color (#RRGGBB)", None),
    ("border", "--border", "Border color (#RRGGBB)", None),
    ("success", "--success", "Success color (#RRGGBB)", None),
    ("danger", "--danger", "Danger color (#RRGGBB)", None),
];
#[derive(Deserialize)]
pub(super) struct AppearancePreset { pub id: String, pub name: String, pub palette: serde_json::Value }
#[derive(Deserialize)]
struct AppearanceResponse { schema: u32, sha256: String, palette: serde_json::Value, presets: Vec<AppearancePreset> }
pub(super) struct AppearanceReview {
    path: PathBuf,
    target: String,
    fingerprint: String,
    pub palette: serde_json::Value,
    pub presets: Vec<AppearancePreset>,
}
fn appearance_value(key: &str, title: &str, range: Option<(u32,u32)>, value: &str) -> Result<serde_json::Value> {
    let value=value.trim();
    if let Some((min,max))=range {
        let number:u32=value.parse().with_context(||format!("{title} must be an integer"))?;
        if number<min || number>max { bail!("{title} must be between {min} and {max}"); }
        Ok(serde_json::json!(number))
    } else {
        if key!="fontFamily" && !(value.len()==7 && value.starts_with('#') && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)) {
            bail!("{title} must be a six-digit hex color");
        }
        if key=="fontFamily" && (value.is_empty() || value.chars().count()>80 || value.chars().any(char::is_control)) {
            bail!("font family must contain 1–80 printable characters");
        }
        Ok(serde_json::json!(value))
    }
}
fn appearance_edits(values:&[String], palette:&serde_json::Value)->Result<Vec<String>> {
    if values.len()!=APPEARANCE_FIELDS.len(){bail!("invalid appearance field set");}
    let mut args=Vec::new();
    for ((key,flag,title,range),value) in APPEARANCE_FIELDS.iter().zip(values) {
        let value=value.trim();
        if value.is_empty(){continue;}
        let candidate=appearance_value(key,title,*range,value)?;
        if palette[*key]!=candidate {args.push(format!("{flag}={value}"));}
    }
    Ok(args)
}
const THEME_LIMIT:u64=64*1024;
#[derive(serde::Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableTheme {
    format:String,
    schema:u32,
    name:String,
    palette:std::collections::BTreeMap<String,serde_json::Value>,
}
fn required_theme_field(key:&str)->bool {
    !matches!(key,"barHeight"|"dockHeight"|"panelOpacity"|"fontSize"|"fontFamily")
}
fn validate_theme(theme:&PortableTheme)->Result<()> {
    if theme.format!="atelier-classic-theme" || theme.schema!=1 {bail!("unsupported portable theme format");}
    snapshot_label(&theme.name)?;
    for key in theme.palette.keys() {
        if !APPEARANCE_FIELDS.iter().any(|(k,_,_,_)|*k==key.as_str()){bail!("unknown portable theme field: {key}");}
    }
    for (key,_,title,range) in APPEARANCE_FIELDS {
        let Some(value)=theme.palette.get(*key) else {
            if required_theme_field(key){bail!("missing required theme field: {key}");}
            continue;
        };
        let text=if range.is_some(){value.as_u64().map(|v|v.to_string())}else{value.as_str().map(str::to_owned)}
            .with_context(||format!("invalid type for {title}"))?;
        appearance_value(key,title,*range,&text)?;
    }
    Ok(())
}
fn theme_path(path:&Path)->Result<()> {
    if !path.is_absolute() || path.extension().and_then(|s|s.to_str())!=Some("json"){bail!("use an absolute .json theme path");}
    Ok(())
}
pub(super) fn import_theme(path:&Path)->Result<(String,Vec<Option<String>>)> {
    use std::os::unix::fs::OpenOptionsExt;
    theme_path(path)?;
    let file=fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path)?;
    let metadata=file.metadata()?;
    if !metadata.is_file() || metadata.len()>THEME_LIMIT {bail!("theme must be a regular file no larger than 64 KiB");}
    let bytes=read_limited(file,THEME_LIMIT)?;
    if bytes.len() as u64>THEME_LIMIT{bail!("theme exceeds size limit");}
    let theme:PortableTheme=serde_json::from_slice(&bytes).context("invalid portable theme JSON")?;
    validate_theme(&theme)?;
    let values=APPEARANCE_FIELDS.iter().map(|(key,_,_,_)|theme.palette.get(*key).map(|v|
        v.as_str().map(str::to_owned).unwrap_or_else(||v.to_string()))).collect();
    Ok((theme.name,values))
}
fn draft_theme(name:&str,values:&[String],baseline:&serde_json::Value)->Result<PortableTheme>{
    if values.len()!=APPEARANCE_FIELDS.len(){bail!("invalid appearance field set");}
    let mut palette=std::collections::BTreeMap::new();
    for ((key,_,title,range),value) in APPEARANCE_FIELDS.iter().zip(values){
        if !value.trim().is_empty(){palette.insert((*key).into(),appearance_value(key,title,*range,value)?);}
        else if !baseline[*key].is_null(){palette.insert((*key).into(),baseline[*key].clone());}
    }
    let theme=PortableTheme{format:"atelier-classic-theme".into(),schema:1,name:name.trim().into(),palette};
    validate_theme(&theme)?;
    Ok(theme)
}
pub(super) fn export_theme(path:&Path,name:&str,values:&[String],baseline:&serde_json::Value)->Result<()>{
    use std::io::Write;
    theme_path(path)?;
    let theme=draft_theme(name,values,baseline)?;
    let parent=path.parent().context("theme destination has no parent")?;
    let mut temp=tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(temp.as_file_mut(),&theme)?;
    temp.as_file_mut().write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(path).context("export refused: destination exists or cannot be created")?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub(super) fn review_appearance(path: &Path, target: &str) -> Result<AppearanceReview> {
    if live_desktop_present() { bail!("use a separate desktop after closing all Mango Atelier sessions"); }
    let checkout = Checkout::open(path)?;
    let bytes = checkout.run(target, &["appearance","--review-json"], "180s")?;
    let response: AppearanceResponse = serde_json::from_slice(&bytes).context("unsupported reviewed appearance API; update Mango Atelier")?;
    if response.schema != 1 || response.sha256.len()!=64
        || !response.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !response.palette.is_object() { bail!("invalid appearance review response"); }
    let mut ids = std::collections::BTreeSet::new();
    if response.presets.len() > 64 || response.presets.iter().any(|p|
        p.id.is_empty() || p.id.len()>48 || !p.id.bytes().all(|b|b.is_ascii_alphanumeric() || matches!(b,b'-'|b'_'))
        || !ids.insert(p.id.clone()) || p.name.is_empty() || !p.palette.is_object()) {
        bail!("invalid bundled appearance presets");
    }
    for preset in &response.presets {
        let values = APPEARANCE_FIELDS.iter().map(|(key,_,_,_)| {
            let value=&preset.palette[*key];
            value.as_str().map(str::to_owned).or_else(||value.as_u64().map(|n|n.to_string())).unwrap_or_default()
        }).collect::<Vec<_>>();
        appearance_edits(&values,&response.palette)?;
    }
    Ok(AppearanceReview { path: checkout.root, target: target.into(), fingerprint:response.sha256,palette:response.palette,presets:response.presets })
}
impl AppearanceReview {
    pub(super) fn wallpaper_source(&self)->(PathBuf,String){(self.path.clone(),self.target.clone())}
    pub(super) fn changes(&self, values: &[String]) -> Result<Vec<String>> {
        appearance_edits(values,&self.palette)
    }
    pub(super) fn apply(self, values: Vec<String>) -> Result<String> {
        if live_desktop_present() { bail!("use a separate desktop after closing all Mango Atelier sessions"); }
        let edits = self.changes(&values)?;
        if edits.is_empty() { bail!("no appearance changes to save"); }
        let checkout = Checkout::open(&self.path)?;
        let mut args = vec!["appearance".into(), format!("--expected-sha256={}",self.fingerprint)];
        args.extend(edits);
        let args:Vec<_> = args.iter().map(String::as_str).collect();
        let output=checkout.run(&self.target,&args,"180s")?;
        let result:serde_json::Value=serde_json::from_slice(&output)?;
        match result["status"].as_str() {
            Some("saved") if result["recovery"].as_str().is_some_and(valid_snapshot_id) =>
                Ok(format!("Appearance saved. Recovery snapshot: {}. Changes apply on next login.",result["recovery"].as_str().unwrap())),
            Some("unchanged") => Ok("No changes saved.".into()),
            _ => bail!("Unrecognized result; inspect the backup inventory before retrying"),
        }
    }
}

#[derive(Clone,Deserialize)]
pub(super) struct WallpaperState {
    schema:u32,
    sha256:String,
    pub wallpapers:std::collections::BTreeMap<String,String>,
    pub slideshows:std::collections::BTreeMap<String,String>,
}
#[derive(Deserialize)]
pub(super) struct WallpaperItem { pub name:String, pub path:String }
#[derive(Clone)]
pub(super) struct WallpaperReview {
    path:PathBuf,
    target:String,
    pub state:WallpaperState,
}
#[derive(Clone,Copy)]
pub(super) enum WallpaperMode { Image, Slideshow, Clear }
fn wallpaper_args(mode:WallpaperMode,screen:&str,value:&str)->Result<Vec<String>>{
    if screen.is_empty() || screen.len()>128 || !screen.bytes().all(|b|b.is_ascii_alphanumeric() || matches!(b,b'_'|b'-'|b'.'|b':')){
        bail!("enter the exact output name (e.g. DP-1)");
    }
    let mode=match mode {WallpaperMode::Image=>"image",WallpaperMode::Slideshow=>"slideshow",WallpaperMode::Clear=>"clear"};
    if mode!="clear" && (!Path::new(value).is_absolute() || value.chars().any(char::is_control)){bail!("use an absolute local image or folder path");}
    let value=if mode=="clear"{""}else{value};
    Ok(vec![format!("--screen={screen}"),format!("--mode={mode}"),format!("--path={value}")])
}
pub(super) fn review_wallpaper(path:&Path,target:&str)->Result<WallpaperReview>{
    if live_desktop_present(){bail!("close all Mango Atelier sessions before editing wallpapers");}
    let checkout=Checkout::open(path)?;
    let state:WallpaperState=serde_json::from_slice(&checkout.run(target,&["wallpaper-review","inspect"],"180s")?)
        .context("unsupported wallpaper review API; update Mango Atelier")?;
    if state.schema!=1 || state.sha256.len()!=64 || !state.sha256.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)){bail!("invalid wallpaper review");}
    Ok(WallpaperReview{path:checkout.root,target:target.into(),state})
}
impl WallpaperReview{
    pub(super) fn gallery(&self,folder:&str)->Result<Vec<WallpaperItem>>{
        if !Path::new(folder).is_absolute(){bail!("use an absolute folder path");}
        let checkout=Checkout::open(&self.path)?;
        let items:Vec<WallpaperItem>=serde_json::from_slice(&checkout.run(&self.target,&["wallpaper-review","gallery",folder],"30s")?)?;
        if items.len()>1000 || items.iter().any(|i|!Path::new(&i.path).is_absolute() || i.path.chars().any(char::is_control)){bail!("invalid wallpaper gallery response");}
        Ok(items)
    }
    pub(super) fn thumbnail(&self,image:&str)->Result<Vec<u8>>{
        if !Path::new(image).is_absolute(){bail!("use an absolute image path");}
        let checkout=Checkout::open(&self.path)?;
        let bytes=checkout.run(&self.target,&["wallpaper-review","thumbnail",image],"30s")?;
        // Only accept the backend's small, re-encoded PNG, never arbitrary source bytes.
        if bytes.len()<24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || &bytes[12..16]!=b"IHDR" {bail!("invalid wallpaper preview");}
        let width=u32::from_be_bytes(bytes[16..20].try_into()?);
        let height=u32::from_be_bytes(bytes[20..24].try_into()?);
        if width==0 || height==0 || width>480 || height>320{bail!("wallpaper preview exceeds dimensions");}
        Ok(bytes)
    }
    pub(super) fn apply(self,mode:WallpaperMode,screen:&str,value:&str)->Result<String>{
        if live_desktop_present(){bail!("close all Mango Atelier sessions before editing wallpapers");}
        let mut args=vec!["wallpaper-review".into(),"apply".into(),format!("--expected-sha256={}",self.state.sha256)];
        args.extend(wallpaper_args(mode,screen,value)?);
        let checkout=Checkout::open(&self.path)?;
        let args:Vec<_>=args.iter().map(String::as_str).collect();
        let result:serde_json::Value=serde_json::from_slice(&checkout.run(&self.target,&args,"180s")?)?;
        match result["status"].as_str(){
            Some("saved") if result["recovery"].as_str().is_some_and(valid_snapshot_id)=>Ok(format!("Wallpaper saved for {screen}. Recovery snapshot: {}. Takes effect on next login.",result["recovery"].as_str().unwrap())),
            Some("unchanged")=>Ok("No wallpaper changes saved.".into()),
            _=>bail!("Unknown wallpaper result; inspect Rice backups before retrying"),
        }
    }
}

pub(super) const LAYOUT_FIELDS:&[(&str,&str,&str)]=&[
    ("bar.compact","Compact panel","Panel"),
    ("bar.showWindowTitle","Show active window title","Panel"),
    ("bar.showDate","Show date beside clock","Panel"),
    ("dock.enabled","Enable dock","Dock"),
    ("dock.hideFullscreen","Hide dock on fullscreen","Dock"),
    ("widgets.calendar","Calendar","Dashboard widgets"),
    ("widgets.media","Media controls","Dashboard widgets"),
    ("widgets.focusTimer","Focus timer","Dashboard widgets"),
    ("widgets.system","System information","Dashboard widgets"),
];
#[derive(Deserialize)]
struct LayoutState{
    schema:u32,
    sha256:String,
    values:std::collections::BTreeMap<String,serde_json::Value>,
}
pub(super) struct LayoutReview{
    path:PathBuf,
    target:String,
    state:LayoutState,
}
pub(super) fn layout_pins(text:&str)->Result<Vec<String>>{
    let mut pins=Vec::new();
    for raw in text.lines().map(str::trim).filter(|s|!s.is_empty()){
        if raw.len()>256 || !raw.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-'|b'.')){
            bail!("Invalid desktop application ID: {raw}");
        }
        let lower=raw.to_ascii_lowercase();
        let id=lower.strip_suffix(".desktop").unwrap_or(&lower);
        if !id.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric) || pins.iter().any(|p|p==id){
            bail!("Empty or duplicate desktop application ID: {raw}");
        }
        pins.push(id.to_owned());
        if pins.len()>64{bail!("At most 64 pinned applications");}
    }
    Ok(pins)
}
pub(super) fn review_layout(path:&Path,target:&str)->Result<LayoutReview>{
    if live_desktop_present(){bail!("close all Mango Atelier sessions before editing layout");}
    let checkout=Checkout::open(path)?;
    let state:LayoutState=serde_json::from_slice(&checkout.run(target,&["layout-review","inspect"],"180s")?)
        .context("unsupported layout review API; update Mango Atelier")?;
    if state.schema!=1 || state.sha256.len()!=64 || !state.sha256.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))
        || state.values.len()!=LAYOUT_FIELDS.len()+1 || LAYOUT_FIELDS.iter().any(|(key,_,_)|state.values.get(*key).and_then(|v|v.as_bool()).is_none()){
        bail!("invalid layout review response");
    }
    let pins:Vec<String>=serde_json::from_value(state.values.get("dock.pins").context("missing dock pins")?.clone())?;
    if pins.len()>64 || pins.iter().any(|p|p.is_empty()||p.len()>256||p.chars().any(char::is_control)){bail!("invalid saved dock pins");}
    Ok(LayoutReview{path:checkout.root,target:target.into(),state})
}
impl LayoutReview{
    pub(super) fn enabled(&self,key:&str)->bool{self.state.values[key].as_bool().unwrap_or(false)}
    pub(super) fn pins(&self)->String{
        self.state.values["dock.pins"].as_array().into_iter().flatten().filter_map(|v|v.as_str()).collect::<Vec<_>>().join("\n")
    }
    pub(super) fn changes(&self,toggles:&[bool],pins:&str)->Result<std::collections::BTreeMap<String,serde_json::Value>>{
        if toggles.len()!=LAYOUT_FIELDS.len(){bail!("invalid layout field set");}
        let mut edits=std::collections::BTreeMap::new();
        for ((key,_,_),enabled) in LAYOUT_FIELDS.iter().zip(toggles){
            if self.enabled(key)!=*enabled{edits.insert((*key).into(),serde_json::json!(enabled));}
        }
        // Do not normalize or materialize defaults unless the pin list was edited.
        if pins!=self.pins(){
            let value=serde_json::json!(layout_pins(pins)?);
            if value!=self.state.values["dock.pins"]{edits.insert("dock.pins".into(),value);}
        }
        Ok(edits)
    }
    pub(super) fn apply(self,toggles:Vec<bool>,pins:String)->Result<String>{
        if live_desktop_present(){bail!("close all Mango Atelier sessions before editing layout");}
        let edits=self.changes(&toggles,&pins)?;
        if edits.is_empty(){return Ok("No layout changes.".into());}
        let checkout=Checkout::open(&self.path)?;
        let fingerprint=format!("--expected-sha256={}",self.state.sha256);
        let payload=format!("--edits-json={}",serde_json::to_string(&edits)?);
        let output=checkout.run(&self.target,&["layout-review","apply",&fingerprint,&payload],"180s")?;
        let result:serde_json::Value=serde_json::from_slice(&output)?;
        match result["status"].as_str(){
            Some("saved") if result["recovery"].as_str().is_some_and(valid_snapshot_id)=>Ok(format!("Layout saved. Recovery snapshot: {}. Changes take effect on next login.",result["recovery"].as_str().unwrap())),
            Some("unchanged")=>Ok("No layout changes saved.".into()),
            _=>bail!("Unknown layout result; inspect Rice backups before retrying"),
        }
    }
}

#[derive(Deserialize)]
pub(super) struct WallpaperPalettes {
    schema:u32,
    pub seed:String,
    pub dark:std::collections::BTreeMap<String,String>,
    pub light:std::collections::BTreeMap<String,String>,
}
fn parse_wallpaper_palettes(bytes:&[u8])->Result<WallpaperPalettes>{
    let result:WallpaperPalettes=serde_json::from_slice(bytes)?;
    if result.schema!=1{bail!("unsupported wallpaper palette schema");}
    appearance_value("accent","Seed color",None,&result.seed)?;
    for palette in [&result.dark,&result.light]{
        if palette.len()!=9{bail!("incomplete wallpaper palette");}
        for (key,_,title,range) in APPEARANCE_FIELDS{
            if range.is_none() && *key!="fontFamily"{
                let value=palette.get(*key).with_context(||format!("missing {key}"))?;
                appearance_value(key,title,None,value)?;
            }
        }
    }
    Ok(result)
}
pub(super) fn wallpaper_palettes(path:&Path,target:&str,image:&str)->Result<WallpaperPalettes>{
    if !Path::new(image).is_absolute() || image.chars().any(char::is_control){bail!("use an absolute local image path");}
    let checkout=Checkout::open(path)?;
    parse_wallpaper_palettes(&checkout.run(target,&["wallpaper-review","palette",image],"30s")?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_palette_requires_all_color_roles_and_no_other_keys(){
        let palette:std::collections::BTreeMap<_,_>=APPEARANCE_FIELDS.iter().filter(|(key,_,_,range)|range.is_none()&&*key!="fontFamily").map(|(key,_,_,_)|(*key,"#123456")).collect();
        let mut data=serde_json::json!({"schema":1,"seed":"#123456","dark":palette,"light":palette});
        assert!(parse_wallpaper_palettes(&serde_json::to_vec(&data).unwrap()).is_ok());
        data["dark"]["radius"]=serde_json::json!("#123456");
        assert!(parse_wallpaper_palettes(&serde_json::to_vec(&data).unwrap()).is_err());
        data["dark"].as_object_mut().unwrap().remove("radius");
        data["light"]["accent"]=serde_json::json!("invalid");
        assert!(parse_wallpaper_palettes(&serde_json::to_vec(&data).unwrap()).is_err());
    }
    #[test]
    fn ordered_layout_pins_are_normalized_and_validated(){
        assert_eq!(layout_pins("Foot.desktop\nFirefox\n").unwrap(),vec!["foot","firefox"]);
        assert!(layout_pins("Firefox\nfirefox.desktop").is_err());
        assert!(layout_pins("/bin/sh").is_err());
        assert!(layout_pins(".desktop").is_err());
        assert!(layout_pins("").unwrap().is_empty());
    }
    #[test]
    fn wallpaper_operations_validate_output_and_paths(){
        assert!(wallpaper_args(WallpaperMode::Image,"DP-1","relative.png").is_err());
        assert!(wallpaper_args(WallpaperMode::Clear,"--help","").is_ok()); // Passed as a value, not an option.
        assert!(wallpaper_args(WallpaperMode::Clear,"bad\nname","").is_err());
        assert_eq!(wallpaper_args(WallpaperMode::Slideshow,"DP-1","/home/user/Pictures").unwrap(),vec!["--screen=DP-1","--mode=slideshow","--path=/home/user/Pictures"]);
        assert_eq!(wallpaper_args(WallpaperMode::Clear,"DP-2","/ignored").unwrap()[2],"--path=");
    }
    #[test]
    fn portable_theme_round_trip_and_no_overwrite() {
        let tmp=tempfile::tempdir().unwrap();
        let path=tmp.path().join("theme.json");
        let mut values=vec![String::new();APPEARANCE_FIELDS.len()];
        for (i,(key,_,_,range)) in APPEARANCE_FIELDS.iter().enumerate(){
            if required_theme_field(key){values[i]=range.map(|(min,_)|min.to_string()).unwrap_or_else(||"#123456".into());}
        }
        export_theme(&path,"My theme",&values,&serde_json::json!({})).unwrap();
        let before=fs::read(&path).unwrap();
        let (name,imported)=import_theme(&path).unwrap();
        assert_eq!(name,"My theme");
        assert_eq!(imported[0].as_deref(),Some("#123456"));
        assert!(imported[8].is_none());
        assert!(export_theme(&path,"Another",&values,&serde_json::json!({})).is_err());
        assert_eq!(fs::read(&path).unwrap(),before);
        let mut data:serde_json::Value=serde_json::from_slice(&before).unwrap();
        data["palette"]["command"]=serde_json::json!("bad");
        fs::write(&path,serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(import_theme(&path).is_err());
        let link=tmp.path().join("link.json");
        std::os::unix::fs::symlink(&path,&link).unwrap();
        assert!(import_theme(&link).is_err());
    }
    #[test]
    fn appearance_edits_validate_bounds_and_keep_absent_values() {
        let palette=serde_json::json!({"accent":"#123456","radius":12});
        let mut values=vec![String::new();APPEARANCE_FIELDS.len()];
        values[0]="#123456".into(); values[1]="12".into();
        assert!(appearance_edits(&values,&palette).unwrap().is_empty());
        values[1]="31".into();
        assert!(appearance_edits(&values,&palette).is_err());
        values[1]="18".into(); values[8]="--help".into();
        assert_eq!(appearance_edits(&values,&palette).unwrap(),vec!["--radius=18","--font-family=--help"]);
        values[0]="red".into();
        assert!(appearance_edits(&values,&palette).is_err());
        values[0]="#123456".into(); values[9]="#10131B".into();
        assert!(appearance_edits(&values,&palette).unwrap().contains(&"--background=#10131B".to_owned()));
        values[16]="invalid".into();
        assert!(appearance_edits(&values,&palette).is_err());
    }
    #[test]
    fn snapshot_actions_restrict_ids_paths_and_flags() {
        let id = "20261124T160000Z-0123456789ab";
        assert!(valid_snapshot_id(id));
        for bad in ["../archive","--allow-active","20261124T160000Z-0123456789AB","20261124T160000Z-0123456789a"] {
            assert!(SnapshotAction::Restore.arguments(bad, "").is_err());
        }
        assert_eq!(SnapshotAction::Restore.arguments(id,"").unwrap(),vec!["snapshot","restore",id]);
        assert_eq!(SnapshotAction::Restore.phrase(id,"").unwrap().as_deref(),Some(format!("restore offline {id}").as_str()));
        assert!(SnapshotAction::Import.arguments("","relative.zip").is_err());
        assert!(SnapshotAction::Export.arguments(id,"/tmp/other.txt").is_err());
        assert_eq!(SnapshotAction::Create.arguments("atelier","--allow-active").unwrap(),vec!["snapshot","create","atelier","--name=--allow-active"]);
        assert_eq!(SnapshotAction::Rename.arguments(id,"--help").unwrap(),vec!["snapshot","rename",id,"--","--help"]);
        assert!(SnapshotAction::Create.arguments("atelier","bad\nname").is_err());
        assert!(SnapshotAction::Delete.phrase(id,"").unwrap().is_some());
        assert!(SnapshotAction::Inspect.phrase(id,"").unwrap().is_none());
    }
    #[test]
    fn actions_are_closed_and_selection_is_next_login_only() {
        assert_eq!(mutation_args(Action::Select,"niri","atelier").unwrap(), vec!["use","atelier","--compositor","niri","--next-login"]);
        assert!(mutation_args(Action::Prepare,"mango","community-rice").is_err());
        assert!(mutation_args(Action::Select,"mango","--yes").is_err());
        assert!(mutation_args(Action::Select,"unknown","atelier").is_err());
    }
    #[test]
    fn catalog_checks_target_and_duplicate_ids() {
        let data = br#"{"compositor":"mango","rices":[{"id":"atelier","name":"Classic","summary":"","ready":true,"selected":true,"supported":true,"compositors":["mango"],"installMethod":"included","missingTools":[]}],"warnings":[]}"#;
        assert!(parse_catalog(data,"mango").is_ok());
        assert!(parse_catalog(data,"niri").is_err());
        let mut value: serde_json::Value = serde_json::from_slice(data).unwrap();
        let row = value["rices"][0].clone();
        value["rices"].as_array_mut().unwrap().push(row);
        assert!(parse_catalog(&serde_json::to_vec(&value).unwrap(),"mango").is_err());
    }
}
