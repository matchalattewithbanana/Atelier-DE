//! Fetch authenticated release assets into isolated, non-installed cache bundles.
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::Serialize;
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
    let root = cache_root()?;
    deploy::ensure_root(&root)?;
    let _lock = cache_lock(&root)?;
    if let Some(path) = reusable(&root, &bytes, &sig, key)? {
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({
            "schema":1,"cache":path,"release_id":id,"version":release.version,"key_id":key,
            "status":"verified-cache-reused","note":"Revalidate before acceptance or installation."
        }))?);
        return Ok(());
    }
    eprintln!("Authenticated release {}: downloading {} signed artifacts into an isolated cache.", release.version, assets.len());
    let path = download_bundle(&root, &bytes, &sig, &release, &assets,
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
    let _lock = cache_lock(&root)?;
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
    let _lock = cache_lock(&root)?;
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


fn cache_lock(root: &Path) -> Result<File> {
    cache_directory(root)?;
    let file = OpenOptions::new().read(true).write(true).create(true).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(root.join("cache.lock"))?;
    let m = file.metadata()?;
    if !m.is_file() || m.uid() != nix::unistd::Uid::effective().as_raw() || m.permissions().mode() & 0o022 != 0 {
        bail!("unsafe cache lock");
    }
    FileExt::lock_exclusive(&file)?;
    Ok(file)
}
fn cache_ids(root: &Path) -> Result<Vec<String>> {
    match fs::symlink_metadata(root) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
        Ok(_) => cache_directory(root)?,
    }
    let mut ids = BTreeSet::new();
    for entry in fs::read_dir(root)? {
        if let Ok(id) = entry?.file_name().into_string() {
            if valid_cache_id(&id) { ids.insert(id); }
        }
    }
    Ok(ids.into_iter().collect())
}
fn cached_metadata(path: &Path) -> Result<(Vec<u8>, Vec<u8>)> {
    cache_directory(path)?;
    cache_directory(&path.join("bin"))?;
    Ok((crate::release::bounded_file(&path.join("manifest.json"), 64 * 1024)?,
        crate::release::bounded_file(&path.join("manifest.sig"), 16 * 1024)?))
}
fn reusable(root: &Path, bytes: &[u8], sig: &[u8], key: &str) -> Result<Option<PathBuf>> {
    for id in cache_ids(root)? {
        let path = root.join(id);
        if let Ok((old, old_sig)) = cached_metadata(&path) {
            if old == bytes && old_sig == sig {
                // An invalid cached copy must never prevent a fresh acquisition.
                let release = trust::check_candidate(bytes, sig, key)?;
                if release.verify_local(&path.join("bin")).is_ok() { return Ok(Some(path)); }
            }
        }
    }
    Ok(None)
}
#[derive(Serialize)]
struct CacheEntry {
    id: String,
    status: &'static str,
    bytes: Option<u64>,
    error: Option<String>,
}
fn inspect_cache(path: &Path) -> Result<u64> {
    let (bytes, sig) = cached_metadata(path)?;
    let release: ReleaseManifest = serde_json::from_slice(&bytes)?;
    release.validate_host()?;
    let bin = path.join("bin");
    let names = fs::read_dir(&bin)?.map(|entry| {
        entry?.file_name().into_string().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid artifact name"))
    }).collect::<io::Result<BTreeSet<_>>>()?;
    let expected = release.artifacts.iter().map(|a| a.name.clone()).collect::<BTreeSet<_>>();
    if names != expected { bail!("unexpected or missing cached artifacts"); }
    let mut total = (bytes.len() + sig.len()) as u64;
    for artifact in &release.artifacts {
        let m = fs::symlink_metadata(bin.join(&artifact.name))?;
        if !m.is_file() || m.uid() != nix::unistd::Uid::effective().as_raw() || m.permissions().mode() & 0o022 != 0
            || m.len() != artifact.bytes || m.permissions().mode() & 0o111 == 0 {
            bail!("unsafe or invalid cached artifact: {}", artifact.name);
        }
        total = total.checked_add(m.len()).context("cache size overflow")?;
    }
    Ok(total)
}
pub fn list_cache() -> Result<()> {
    let root = cache_root()?;
    let entries: Vec<_> = cache_ids(&root)?.into_iter().map(|id| {
        match inspect_cache(&root.join(&id)) {
            Ok(bytes) => CacheEntry { id, status: "structural-check-passed", bytes: Some(bytes), error: None },
            Err(error) => CacheEntry { id, status: "invalid-or-incomplete", bytes: None, error: Some(format!("{error:#}")) },
        }
    }).collect();
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "schema":1,"entries":entries,"note":"Sizes are logical artifact and metadata bytes, not disk allocation. No signature or hash verification."
    }))?);
    Ok(())
}
fn remove_entry(root: &Path, id: &str) -> Result<()> {
    if !valid_cache_id(id) { bail!("invalid cache ID"); }
    let path = root.join(id);
    cache_directory(&path)?;
    // Rename out of the cache namespace before deletion; do not follow symlinks.
    let quarantine = tempfile::Builder::new().prefix("removed-").tempdir_in(root)?;
    fs::rename(&path, quarantine.path().join("bundle"))?;
    File::open(root)?.sync_all()?;
    quarantine.close()?;
    File::open(root)?.sync_all()?;
    Ok(())
}
pub fn remove_cache(id: &str) -> Result<()> {
    if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() { bail!("interactive cache removal requires a terminal"); }
    if !valid_cache_id(id) { bail!("invalid cache ID"); }
    let root = cache_root()?;
    let _lock = cache_lock(&root)?;
    let path = root.join(id);
    cache_directory(&path)?;
    println!("Remove cached release {id}? Installed deployments and trust records remain untouched.");
    print!("Type 'remove cache' to proceed: "); io::stdout().flush()?;
    let mut answer = String::new(); io::stdin().read_line(&mut answer)?;
    if answer.trim() != "remove cache" { println!("Cancelled."); return Ok(()); }
    remove_entry(&root, id)?;
    println!("Cache removed.");
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
    fn cache_removal_is_scoped_and_does_not_follow_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("keep"), b"keep").unwrap();
        let dir = root.path().join("release-A");
        fs::create_dir(&dir).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.join("link")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("release-B")).unwrap();
        let _lock = cache_lock(root.path()).unwrap();
        assert!(remove_entry(root.path(), "release-B").is_err());
        assert!(remove_entry(root.path(), "../release-A").is_err());
        remove_entry(root.path(), "release-A").unwrap();
        assert!(!dir.exists());
        assert!(outside.path().join("keep").exists());
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
