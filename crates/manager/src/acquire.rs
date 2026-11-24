//! Fetch authenticated release assets into isolated, non-installed cache bundles.
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, env, fs::{self, File, OpenOptions}, io::{self, IsTerminal, Read, Write}, os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt}, path::{Path, PathBuf}};
use crate::{deploy, discovery::{self, ApiAsset, Asset}, trust, release::ReleaseManifest};

fn cache_root() -> Result<PathBuf> {
    let base = env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).map(|p| p.join(".cache")))
        .context("no absolute cache or home directory")?;
    Ok(base.join("atelier-manager/releases"))
}

fn required_asset(assets: &[ApiAsset], name: &str, limit: u64) -> Result<Asset> {
    discovery::asset(assets, name, limit).with_context(|| format!("missing, duplicate or invalid asset: {name}"))
}
fn binary_assets(assets: &[ApiAsset], release: &ReleaseManifest) -> Result<Vec<(String, Asset)>> {
    let mut ids = BTreeSet::new();
    release.artifacts.iter().map(|expected| {
        let name = format!("{}-linux-{}", expected.name, release.arch);
        let asset = required_asset(assets, &name, expected.bytes)?;
        if asset.size != expected.bytes || !ids.insert(asset.id) {
            bail!("inconsistent binary asset metadata: {name}");
        }
        Ok((expected.name.clone(), asset))
    }).collect()
}

fn download_bundle(root: &Path, bytes: &[u8], signature: &[u8], release: &ReleaseManifest,
    assets: &[(String, Asset)], mut fetch: impl FnMut(&Asset, &mut File) -> Result<()>,
    final_check: impl FnOnce(&Path) -> Result<()>) -> Result<PathBuf> {
    deploy::ensure_root(root)?;
    let pending = tempfile::Builder::new().prefix("release-").tempdir_in(root)?;
    let bin = pending.path().join("bin");
    fs::create_dir(&bin)?;
    for (name, asset) in assets {
        let path = bin.join(name);
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
        fetch(asset, &mut file)?;
        file.sync_all()?;
        // Never make network bytes executable until their signed hash matches.
        let expected = release.artifacts.iter().find(|a| a.name == *name).context("missing signed artifact")?;
        let input = File::open(&path)?;
        let mut hash = Sha256::new();
        let count = io::copy(&mut input.take(expected.bytes + 1), &mut hash)?;
        if count != expected.bytes || format!("{:x}", hash.finalize()) != expected.sha256 {
            bail!("downloaded artifact hash mismatch: {name}");
        }
        file.set_permissions(fs::Permissions::from_mode(0o700))?;
        file.sync_all()?;
    }
    release.verify_local(&bin)?;
    final_check(&bin)?;
    File::open(&bin)?.sync_all()?;
    // Exact signed bytes are preserved. The manifest is the completion marker.
    for (name, content) in [("manifest.sig", signature), ("manifest.json", bytes)] {
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(pending.path().join(name))?;
        file.write_all(content)?;
        file.sync_all()?;
    }
    File::open(pending.path())?.sync_all()?;
    File::open(root)?.sync_all()?;
    Ok(pending.keep())
}

pub fn run(choice: &str, id: &str, key: &str, prerelease: bool) -> Result<()> {
    let repo = discovery::repository(choice)?;
    let id: u64 = id.parse().context("expected numeric release ID")?;
    let metadata = discovery::detail(repo, id, prerelease)?;
    let manifest_name = format!("atelier-release-linux-{}.json", env::consts::ARCH);
    let manifest = required_asset(&metadata.assets, &manifest_name, 64 * 1024)?;
    let signature = required_asset(&metadata.assets, &format!("{manifest_name}.sig"), 16 * 1024)?;
    if manifest.id == signature.id { bail!("duplicate metadata asset ID"); }
    let mut bytes = Vec::new();
    let mut sig = Vec::new();
    discovery::download(repo, &manifest, &mut bytes)?;
    discovery::download(repo, &signature, &mut sig)?;
    let release = trust::check_candidate(&bytes, &sig, key)?;
    let assets = binary_assets(&metadata.assets, &release)?;
    if assets.iter().any(|(_, a)| a.id == manifest.id || a.id == signature.id) { bail!("duplicate asset ID"); }
    eprintln!("Authenticated release {}: downloading {} signed artifacts into an isolated cache.", release.version, assets.len());
    let path = download_bundle(&cache_root()?, &bytes, &sig, &release, &assets,
        |asset, output| discovery::download(repo, asset, output),
        |bin| trust::check_candidate(&bytes, &sig, key)?.verify_local(bin))?;
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "schema":1,"cache":path,"release_id":id,"version":release.version,"key_id":key,
        "status":"verified-cache-only","note":"Revalidate current trust and artifacts before acceptance or installation. No high-water marks advanced."
    }))?);
    Ok(())
}

fn cache_directory(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.uid() != nix::unistd::Uid::effective().as_raw() || m.permissions().mode() & 0o022 != 0 {
        bail!("unsafe cache directory: {}", path.display());
    }
    Ok(())
}
fn valid_cache_id(id: &str) -> bool {
    id.strip_prefix("release-").is_some_and(|suffix| !suffix.is_empty() && suffix.len() <= 64
        && suffix.bytes().all(|b| b.is_ascii_alphanumeric()))
}
pub fn stage_cached(id: &str, key: &str) -> Result<()> {
    if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() { bail!("interactive staging requires a terminal"); }
    if !valid_cache_id(id) { bail!("expected cache ID release-... (the final directory name from release-fetch)"); }
    let root = cache_root()?;
    cache_directory(&root)?;
    let path = root.join(id);
    cache_directory(&path)?;
    let bin = path.join("bin");
    cache_directory(&bin)?;
    let bytes = crate::release::bounded_file(&path.join("manifest.json"), 64 * 1024)?;
    let signature = crate::release::bounded_file(&path.join("manifest.sig"), 16 * 1024)?;
    let release = trust::check_candidate(&bytes, &signature, key)?;
    release.verify_local(&bin)?;
    println!("Authenticated release {} (sequence {})", release.version, release.sequence.context("missing release sequence")?);
    println!("Publisher: {key}\nProducts: {}", release.products.join(", "));
    println!("Stage a new complete deployment and accept its release sequences. No running session, package, settings or active selection changes.");
    print!("Type 'stage release' to proceed: "); io::stdout().flush()?;
    let mut answer = String::new(); io::stdin().read_line(&mut answer)?;
    if answer.trim() != "stage release" { println!("Cancelled."); return Ok(()); }
    cache_directory(&root)?;
    cache_directory(&path)?;
    cache_directory(&bin)?;
    let staged = trust::stage_authenticated(&bytes, &signature, key, &bin)?;
    println!("Release staged at {}. Activate separately after reviewing deployments.", staged.display());
    Ok(())
}

pub fn update_cached(id: &str, key: &str) -> Result<()> {
    if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() { bail!("interactive update requires a terminal"); }
    if !valid_cache_id(id) { bail!("expected cache ID release-..."); }
    let root = cache_root()?;
    let path = root.join(id);
    let bin = path.join("bin");
    for dir in [&root, &path, &bin] { cache_directory(dir)?; }
    let bytes = crate::release::bounded_file(&path.join("manifest.json"), 64 * 1024)?;
    let signature = crate::release::bounded_file(&path.join("manifest.sig"), 16 * 1024)?;
    let release = trust::check_candidate(&bytes, &signature, key)?;
    release.verify_local(&bin)?;
    let Some(plan) = deploy::review_signed_update(&bin, &release, key)? else { return Ok(()); };
    println!("The whole signed component bundle is accepted. Retained components may be local builds; mixed-build compatibility is not established.");
    println!("The active deployment remains unchanged. Release provenance applies only to the listed signed artifacts.");
    print!("Type 'stage update' to proceed: "); io::stdout().flush()?;
    let mut answer = String::new(); io::stdin().read_line(&mut answer)?;
    if answer.trim() != "stage update" { println!("Cancelled."); return Ok(()); }
    for dir in [&root, &path, &bin] { cache_directory(dir)?; }
    let staged = trust::update_authenticated(&bytes, &signature, key, plan)?;
    println!("Authenticated update staged at {}. Activate separately.", staged.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(bytes: &[u8]) -> (ReleaseManifest, Vec<(String, Asset)>) {
        let hash = format!("{:x}", Sha256::digest(bytes));
        let release: ReleaseManifest = serde_json::from_value(serde_json::json!({
            "schema":2,"sequence":1,"version":"v1","revision":"0123456789abcdef0123456789abcdef01234567",
            "os":"linux","arch":env::consts::ARCH,"products":["shell"],
            "artifacts":[{"name":"atelier-shell","bytes":bytes.len(),"sha256":hash}]
        })).unwrap();
        let api: Vec<ApiAsset> = serde_json::from_value(serde_json::json!([
            {"id":1,"name":format!("atelier-shell-linux-{}", env::consts::ARCH),"state":"uploaded","size":bytes.len()}
        ])).unwrap();
        let assets = binary_assets(&api, &release).unwrap();
        (release, assets)
    }
    #[test]
    fn cache_ids_cannot_escape_storage() {
        assert!(valid_cache_id("release-Ab12"));
        for id in ["release-", "../release-A", "release-A/../B", "/tmp/release-A"] {
            assert!(!valid_cache_id(id));
        }
    }
    #[test]
    fn cache_only_completes_after_hash_and_final_policy_check() {
        let tmp = tempfile::tempdir().unwrap();
        let (release, assets) = fixture(b"test");
        let root = tmp.path().join("cache");
        let fail = download_bundle(&root,b"manifest",b"signature",&release,&assets,
            |_,f| { f.write_all(b"evil")?; Ok(()) }, |_| Ok(()));
        assert!(fail.is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(),0);
        assert!(download_bundle(&root,b"manifest",b"signature",&release,&assets,
            |_,f| { f.write_all(b"test")?; Ok(()) }, |_| bail!("revoked")).is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(),0);
        let path = download_bundle(&root,b"manifest",b"signature",&release,&assets,
            |_,f| { f.write_all(b"test")?; Ok(()) }, |_| Ok(())).unwrap();
        assert_eq!(fs::read(path.join("manifest.json")).unwrap(),b"manifest");
        assert_eq!(fs::read(path.join("bin/atelier-shell")).unwrap(),b"test");
    }
}
