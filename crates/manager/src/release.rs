//! Local release manifest contract. Hashes establish identity, not authenticity.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, env, fs, io::{self, Read}, os::unix::fs::OpenOptionsExt, path::Path};
use crate::{catalog, deploy};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseArtifact {
    name: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseManifest {
    schema: u32,
    version: String,
    revision: String,
    os: String,
    arch: String,
    products: Vec<String>,
    artifacts: Vec<ReleaseArtifact>,
}

fn token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+' | b'_'))
}
fn lowercase_hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn expected_products(ids: &[String]) -> Result<(Vec<String>, BTreeSet<&'static str>)> {
    if ids.len() > 64 { bail!("too many product IDs"); }
    let products = catalog::resolve(ids)?;
    if products.iter().any(|p| !matches!(p.source, "atelier-de" | "aggregate")) {
        bail!("release manifests support native Atelier products only");
    }
    let ids = products.iter().filter(|p| !p.binaries.is_empty()).map(|p| p.id.to_owned()).collect();
    let names = products.iter().flat_map(|p| p.binaries.iter().copied()).collect();
    Ok((ids, names))
}

impl ReleaseManifest {
    fn validate(&self) -> Result<()> {
        if self.schema != 1 { bail!("unsupported release manifest schema"); }
        if !token(&self.version) || !lowercase_hex(&self.revision, 40) {
            bail!("invalid release version label or full lowercase Git revision");
        }
        if self.os != "linux" || !token(&self.arch) { bail!("unsupported target OS or invalid architecture"); }
        let (resolved, expected) = expected_products(&self.products)?;
        let products: BTreeSet<_> = self.products.iter().map(String::as_str).collect();
        if products.len() != self.products.len()
            || resolved.iter().map(String::as_str).collect::<BTreeSet<_>>() != products {
            bail!("release products must be unique, concrete and dependency-complete");
        }
        let actual: BTreeSet<_> = self.artifacts.iter().map(|a| a.name.as_str()).collect();
        if actual != expected || actual.len() != self.artifacts.len() {
            bail!("release artifacts do not match declared products");
        }
        for a in &self.artifacts {
            if a.bytes == 0 || a.bytes > 1024 * 1024 * 1024 || !lowercase_hex(&a.sha256, 64) {
                bail!("invalid release artifact metadata: {}", a.name);
            }
        }
        Ok(())
    }

    fn verify_local(&self, source: &Path) -> Result<()> {
        self.validate()?;
        if self.os != env::consts::OS || self.arch != env::consts::ARCH {
            bail!("manifest target does not match this host");
        }
        let source = source.canonicalize().context("resolve local artifact directory")?;
        if !source.is_dir() { bail!("artifact source must be a directory"); }
        for expected in &self.artifacts {
            let actual = deploy::read_artifact(&source.join(&expected.name), &expected.name, io::sink())?;
            if actual.bytes != expected.bytes || actual.sha256 != expected.sha256 {
                bail!("release artifact mismatch: {}", expected.name);
            }
        }
        Ok(())
    }
}

fn create(source: &Path, version: &str, revision: &str, ids: &[String]) -> Result<ReleaseManifest> {
    if !token(version) || !lowercase_hex(revision, 40) { bail!("invalid release identity"); }
    let source = source.canonicalize().context("resolve trusted build directory")?;
    if !source.is_dir() { bail!("source must be a directory"); }
    let (products, names) = expected_products(ids)?;
    let mut artifacts = Vec::new();
    for name in names {
        let a = deploy::read_artifact(&source.join(name), name, io::sink())?;
        artifacts.push(ReleaseArtifact { name: a.name, bytes: a.bytes, sha256: a.sha256 });
    }
    let release = ReleaseManifest { schema: 1, version: version.into(), revision: revision.into(),
        os: env::consts::OS.into(), arch: env::consts::ARCH.into(), products, artifacts };
    release.validate()?;
    Ok(release)
}

fn load(path: &Path) -> Result<ReleaseManifest> {
    let file = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path).context("open release manifest")?;
    if !file.metadata()?.is_file() { bail!("release manifest must be a regular file"); }
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 { bail!("release manifest exceeds 64 KiB"); }
    Ok(serde_json::from_slice(&bytes).context("parse release manifest")?)
}

pub fn generate(args: &[String]) -> Result<()> {
    match args {
        [from, source, version_flag, version, revision_flag, revision, products @ ..]
            if from == "--from" && version_flag == "--version" && revision_flag == "--revision" => {
                let release = create(Path::new(source), version, revision, products)?;
                println!("{}", serde_json::to_string_pretty(&release)?);
                Ok(())
            }
        _ => bail!("usage: atelier-manager release-manifest --from BUILD_DIR --version VERSION --revision SHA PRODUCT [PRODUCT...]"),
    }
}

pub fn verify(path: &Path, source: &Path) -> Result<()> {
    let release = load(path)?;
    release.verify_local(source)?;
    println!("Release {} ({}) matches local artifact sizes and SHA-256 hashes.", release.version, release.revision);
    println!("Manifest authenticity, declared revision, ABI and hardware compatibility are not verified.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    const REV: &str = "0123456789abcdef0123456789abcdef01234567";
    fn artifact(root: &Path, name: &str, bytes: &[u8]) {
        fs::write(root.join(name), bytes).unwrap();
        fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn dependency_complete_manifest_verifies_and_detects_same_size_corruption() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["atelier-session", "atelier-compositor"] { artifact(tmp.path(), name, b"old"); }
        let release = create(tmp.path(), "0.1.0-alpha.1", REV, &["session".into()]).unwrap();
        assert_eq!(release.products, vec!["compositor", "session"]);
        release.verify_local(tmp.path()).unwrap();
        artifact(tmp.path(), "atelier-compositor", b"new");
        assert!(release.verify_local(tmp.path()).is_err());
    }

    #[test]
    fn rejects_unknown_products_wrong_target_and_unsafe_artifact_names() {
        let tmp = tempfile::tempdir().unwrap();
        artifact(tmp.path(), "atelier-shell", b"test");
        assert!(create(tmp.path(), "v1", REV, &["niri".into()]).is_err());
        assert!(create(tmp.path(), "../v1", REV, &["shell".into()]).is_err());
        let mut release = create(tmp.path(), "v1", REV, &["shell".into()]).unwrap();
        release.arch = "other-arch".into();
        assert!(release.verify_local(tmp.path()).is_err());
        release.arch = env::consts::ARCH.into();
        release.artifacts[0].name = "../atelier-shell".into();
        assert!(release.validate().is_err());
    }

    #[test]
    fn strict_reader_rejects_symlinks_unknown_fields_and_oversized_input() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("manifest.json");
        fs::write(&path, vec![b' '; 64 * 1024 + 1]).unwrap();
        assert!(load(&path).is_err());
        std::os::unix::fs::symlink(&path, tmp.path().join("link")).unwrap();
        assert!(load(&tmp.path().join("link")).is_err());
        artifact(tmp.path(), "atelier-shell", b"test");
        let release = create(tmp.path(), "v1", REV, &["shell".into()]).unwrap();
        let mut json = serde_json::to_value(release).unwrap();
        json["unknown"] = serde_json::json!(true);
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(load(&path).is_err());
    }
}
