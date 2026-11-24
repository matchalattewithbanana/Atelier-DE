//! Explicit local publisher trust and per-artifact high-water marks.
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, env, fs::{self, File, OpenOptions}, io::{self, IsTerminal, Read, Write}, os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt}, path::{Path, PathBuf}};
use crate::release::{self, ReleaseManifest};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Key { public: String, pub revoked: bool }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Accepted { sequence: u64, version: String, revision: String, sha256: String, bytes: u64 }

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct State {
    schema: u32,
    pub keys: BTreeMap<String, Key>,
    accepted: BTreeMap<String, Accepted>,
}
impl Default for State {
    fn default() -> Self { Self { schema: 1, keys: BTreeMap::new(), accepted: BTreeMap::new() } }
}
fn key_id(public: &str) -> String { format!("{:x}", Sha256::digest(public.as_bytes())) }
fn root() -> Result<PathBuf> {
    let base = env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).map(|p| p.join(".local/state")))
        .context("no absolute state or home directory")?;
    Ok(base.join("atelier-manager/release-trust"))
}
fn owned(m: &fs::Metadata) -> bool {
    m.uid() == nix::unistd::Uid::effective().as_raw() && m.permissions().mode() & 0o022 == 0
}
fn directory(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || !owned(&m) { bail!("unsafe release trust directory"); }
    Ok(())
}
fn load(root: &Path) -> Result<State> {
    match fs::symlink_metadata(root) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(State::default()),
        Err(e) => return Err(e.into()),
        Ok(_) => directory(root)?,
    }
    let file = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(root.join("trust.json")) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(State::default()),
        Err(e) => return Err(e.into()),
    };
    let m = file.metadata()?;
    if !m.is_file() || !owned(&m) { bail!("unsafe release trust state"); }
    let mut bytes = Vec::new();
    file.take(128 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 128 * 1024 { bail!("release trust state too large"); }
    let state: State = serde_json::from_slice(&bytes)?;
    if state.schema != 1 || state.keys.len() > 32 || state.accepted.len() > 128 { bail!("invalid release trust state"); }
    for (id, key) in &state.keys {
        if key_id(&key.public) != *id || release::canonical_key(key.public.as_bytes())? != key.public {
            bail!("invalid pinned key identity");
        }
    }
    Ok(state)
}
fn locked(root: &Path) -> Result<File> {
    fs::create_dir_all(root)?;
    directory(root)?;
    let file = OpenOptions::new().read(true).write(true).create(true).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(root.join("trust.lock"))?;
    let m = file.metadata()?;
    if !m.is_file() || !owned(&m) { bail!("unsafe release trust lock"); }
    FileExt::lock_exclusive(&file)?;
    Ok(file)
}
fn save(root: &Path, state: &State) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(state)?;
    if bytes.len() > 128 * 1024 { bail!("release trust state too large"); }
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(root.join("trust.json"))?;
    File::open(root)?.sync_all()?;
    Ok(())
}
fn confirm(prompt: &str, phrase: &str) -> Result<bool> {
    if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() { bail!("interactive confirmation required"); }
    println!("{prompt}");
    print!("Type '{phrase}' to proceed: "); io::stdout().flush()?;
    let mut answer = String::new(); io::stdin().read_line(&mut answer)?;
    Ok(answer.trim() == phrase)
}
fn pin_key(state: &mut State, public: String) -> Result<String> {
    let id = key_id(&public);
    if let Some(key) = state.keys.get(&id) {
        if key.revoked { bail!("revoked key cannot be pinned again"); }
        return Ok(id);
    }
    if state.keys.len() >= 32 { bail!("publisher key limit reached"); }
    state.keys.insert(id.clone(), Key { public, revoked: false });
    Ok(id)
}
fn revoke_key(state: &mut State, id: &str) -> Result<()> {
    state.keys.get_mut(id).context("unknown publisher key")?.revoked = true;
    Ok(())
}
fn active_key<'a>(state: &'a State, id: &str) -> Result<&'a str> {
    let key = state.keys.get(id).context("publisher key is not pinned")?;
    if key.revoked { bail!("publisher key is revoked"); }
    Ok(&key.public)
}
pub(super) fn snapshot() -> Result<State> { load(&root()?) }
pub fn list() -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&snapshot()?)?);
    Ok(())
}
pub fn pin(path: &Path) -> Result<()> {
    let public = release::canonical_key(&release::bounded_file(path, 8 * 1024)?)?;
    let id = key_id(&public);
    if !confirm(&format!("Pin publisher key ID {id}\n{public}\nVerify this exact key independently. It can authenticate releases for all native products."), "trust publisher")? {
        println!("Cancelled."); return Ok(());
    }
    let root = root()?;
    let _lock = locked(&root)?;
    let mut state = load(&root)?;
    pin_key(&mut state, public)?;
    save(&root, &state)?;
    println!("Pinned publisher key {id}.");
    Ok(())
}
pub fn revoke(id: &str) -> Result<()> {
    if !confirm(&format!("Permanently revoke local publisher key {id}. Existing deployments are unchanged."), "revoke publisher")? {
        println!("Cancelled."); return Ok(());
    }
    let root = root()?;
    let _lock = locked(&root)?;
    let mut state = load(&root)?;
    revoke_key(&mut state, id)?;
    save(&root, &state)?;
    println!("Publisher key revoked.");
    Ok(())
}
fn advance(state: &mut State, release: &ReleaseManifest) -> Result<()> {
    let sequence = release.sequence.context("persistent trust requires a schema-2 release sequence")?;
    let mut next = state.accepted.clone();
    for artifact in &release.artifacts {
        let slot = format!("{}/{}", release.arch, artifact.name);
        let value = Accepted { sequence, version: release.version.clone(), revision: release.revision.clone(),
            sha256: artifact.sha256.clone(), bytes: artifact.bytes };
        if let Some(old) = next.get(&slot) {
            if sequence < old.sequence { bail!("older release sequence for {}", artifact.name); }
            if sequence == old.sequence && &value != old { bail!("conflicting identity at accepted sequence for {}", artifact.name); }
        }
        next.insert(slot, value);
    }
    if next.len() > 128 { bail!("release high-water mark limit reached"); }
    state.accepted = next;
    Ok(())
}
/// Check a candidate against the current local policy without advancing it.
pub(super) fn check_candidate(bytes: &[u8], signature: &[u8], id: &str) -> Result<ReleaseManifest> {
    let mut state = load(&root()?)?;
    release::authenticate(bytes, signature, active_key(&state, id)?.as_bytes())?;
    let release: ReleaseManifest = serde_json::from_slice(bytes).context("parse authenticated release")?;
    release.validate_host()?;
    advance(&mut state, &release)?;
    Ok(release)
}

/// Accept and stage in one serialized policy operation. Persist the high-water
/// marks before publishing a complete deployment, so failure cannot publish
/// a staged release that was never accepted.
pub(super) fn stage_authenticated(bytes: &[u8], signature: &[u8], id: &str, source: &Path) -> Result<PathBuf> {
    let root = root()?;
    active_key(&load(&root)?, id)?;
    let _lock = locked(&root)?;
    let mut state = load(&root)?;
    release::authenticate(bytes, signature, active_key(&state, id)?.as_bytes())?;
    let release: ReleaseManifest = serde_json::from_slice(bytes).context("parse authenticated release")?;
    release.validate_host()?;
    advance(&mut state, &release)?;
    crate::deploy::stage_release(source, &release, id, |pending| {
        for (name, content) in [("release-manifest.json", bytes), ("release-manifest.sig", signature)] {
            let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(pending.join(name))?;
            file.write_all(content)?;
            file.sync_all()?;
        }
        File::open(pending)?.sync_all()?;
        save(&root, &state)?;
        Ok(())
    })
}

/// Reauthenticate the exact reviewed bytes and serialize acceptance with staging.
pub(super) fn update_authenticated(bytes: &[u8], signature: &[u8], id: &str,
    plan: crate::deploy::StagePlan) -> Result<PathBuf> {
    let root = root()?;
    active_key(&load(&root)?, id)?;
    let _lock = locked(&root)?;
    let mut state = load(&root)?;
    release::authenticate(bytes, signature, active_key(&state, id)?.as_bytes())?;
    let release: ReleaseManifest = serde_json::from_slice(bytes).context("parse authenticated release")?;
    release.validate_host()?;
    advance(&mut state, &release)?;
    crate::deploy::complete_signed_update(plan, |pending| {
        for (name, content) in [("release-manifest.json", bytes), ("release-manifest.sig", signature)] {
            let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(pending.join(name))?;
            file.write_all(content)?;
            file.sync_all()?;
        }
        File::open(pending)?.sync_all()?;
        save(&root, &state)?;
        Ok(())
    })
}

pub fn accept(manifest: &Path, signature: &Path, id: &str, source: &Path) -> Result<()> {
    if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
    let bytes = release::bounded_file(manifest, 64 * 1024)?;
    let signature = release::bounded_file(signature, 16 * 1024)?;
    let root = root()?;
    // Do not create an empty store just because an unknown key was requested.
    active_key(&load(&root)?, id)?;
    let _lock = locked(&root)?;
    let mut state = load(&root)?;
    release::authenticate(&bytes, &signature, active_key(&state, id)?.as_bytes())?;
    let release: ReleaseManifest = serde_json::from_slice(&bytes).context("parse authenticated release")?;
    release.verify_local(source)?;
    advance(&mut state, &release)?;
    save(&root, &state)?;
    println!("Authenticated release {} accepted; local high-water marks recorded. No installation performed.", release.version);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(seq: u64, hash: &str) -> ReleaseManifest {
        serde_json::from_value(serde_json::json!({"schema":2,"version":"v1","revision":"0123456789abcdef0123456789abcdef01234567",
            "os":"linux","arch":"x86_64","products":["shell"],"artifacts":[{"name":"atelier-shell","bytes":1,"sha256":hash}],"sequence":seq})).unwrap()
    }
    #[test]
    fn high_water_mark_is_monotonic_and_conflicts_do_not_mutate() {
        let mut s = State::default();
        advance(&mut s, &release(2, "a")).unwrap();
        advance(&mut s, &release(2, "a")).unwrap();
        assert!(advance(&mut s, &release(1, "a")).is_err());
        assert!(advance(&mut s, &release(2, "b")).is_err());
        assert_eq!(s.accepted["x86_64/atelier-shell"].sequence, 2);
        advance(&mut s, &release(3, "b")).unwrap();
        assert_eq!(s.accepted["x86_64/atelier-shell"].sequence, 3);
    }
    #[test]
    fn revocation_is_persistent_and_cannot_be_bypassed_by_repinning() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("trust");
        assert!(load(&root).unwrap().keys.is_empty());
        assert!(!root.exists());
        let _lock = locked(&root).unwrap();
        let mut s = load(&root).unwrap();
        let public = format!("ssh-ed25519 {}", "A".repeat(68));
        let id = pin_key(&mut s, public.clone()).unwrap();
        assert!(active_key(&s, &id).is_ok());
        revoke_key(&mut s, &id).unwrap();
        save(&root, &s).unwrap();
        let mut s = load(&root).unwrap();
        assert!(active_key(&s, &id).is_err());
        assert!(pin_key(&mut s, public).is_err());
    }
}
