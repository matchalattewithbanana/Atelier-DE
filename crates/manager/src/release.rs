//! Local release manifest contract. Hashes establish identity, not authenticity.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, env, fs, io::{self, Read}, os::unix::fs::{OpenOptionsExt, MetadataExt, PermissionsExt}, path::Path, process::{Command, Stdio}};
use crate::{catalog, deploy};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReleaseArtifact {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReleaseManifest {
    schema: u32,
    pub version: String,
    pub revision: String,
    os: String,
    pub arch: String,
    products: Vec<String>,
    pub artifacts: Vec<ReleaseArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u64>,
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
        if !matches!((self.schema, self.sequence), (1, None) | (2, Some(1..))) { bail!("unsupported release manifest schema or sequence"); }
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

    pub(super) fn validate_host(&self) -> Result<()> {
        self.validate()?;
        if self.os != env::consts::OS || self.arch != env::consts::ARCH {
            bail!("manifest target does not match this host");
        }
        Ok(())
    }

    pub(super) fn verify_local(&self, source: &Path) -> Result<()> {
        self.validate_host()?;
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
        os: env::consts::OS.into(), arch: env::consts::ARCH.into(), products, artifacts, sequence: None };
    release.validate()?;
    Ok(release)
}

pub(super) fn bounded_file(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path).with_context(|| format!("open {}", path.display()))?;
    if !file.metadata()?.is_file() { bail!("expected a regular file"); }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit { bail!("release metadata exceeds size limit"); }
    Ok(bytes)
}
fn load(path: &Path) -> Result<ReleaseManifest> {
    Ok(serde_json::from_slice(&bounded_file(path, 64 * 1024)?).context("parse release manifest")?)
}

const SIGNER: &str = "atelier-release";
const NAMESPACE: &str = "atelier-release-v1";
const SSH_KEYGEN: &str = "/usr/bin/ssh-keygen";

pub(super) fn canonical_key(key: &[u8]) -> Result<String> {
    let key = std::str::from_utf8(key)?.trim();
    if key.lines().count() != 1 { bail!("expected one Ed25519 public key"); }
    let mut fields = key.split_ascii_whitespace();
    if fields.next() != Some("ssh-ed25519") { bail!("only plain Ed25519 publisher keys are supported"); }
    let blob = fields.next().context("missing public key blob")?;
    // Ed25519's SSH wire blob is 51 bytes, encoded as 68 base64 characters.
    if blob.len() != 68 || !blob.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/')) {
        bail!("invalid Ed25519 public key encoding");
    }
    Ok(format!("ssh-ed25519 {blob}"))
}

fn allowed_signer(key: &[u8]) -> Result<String> {
    Ok(format!("{SIGNER} namespaces=\"{NAMESPACE}\" {}\n", canonical_key(key)?))
}

pub(super) fn authenticate(bytes: &[u8], signature: &[u8], key: &[u8]) -> Result<()> {
    let metadata = fs::metadata(SSH_KEYGEN).context("OpenSSH ssh-keygen is required")?;
    if !metadata.is_file() || metadata.uid() != 0 || metadata.permissions().mode() & 0o022 != 0
        || metadata.permissions().mode() & 0o111 == 0 {
        bail!("untrusted system ssh-keygen executable");
    }
    // Snapshot the exact bytes to verify, including the signature and public key.
    // OpenSSH performs cryptography and namespace checking; no shell is invoked.
    let temp = tempfile::tempdir()?;
    let allowed = temp.path().join("allowed-signers");
    let sig = temp.path().join("manifest.sig");
    let input = temp.path().join("manifest");
    fs::write(&allowed, allowed_signer(key)?)?;
    fs::write(&sig, signature)?;
    fs::write(&input, bytes)?;
    let result = Command::new(SSH_KEYGEN)
        .args(["-Y", "verify", "-I", SIGNER, "-n", NAMESPACE, "-f"]).arg(&allowed)
        .arg("-s").arg(&sig).stdin(Stdio::from(fs::File::open(input)?))
        .stdout(Stdio::null()).stderr(Stdio::null()).status().context("verify publisher signature")?;
    if !result.success() { bail!("release signature rejected for the supplied publisher key and namespace"); }
    Ok(())
}

pub fn verify_signed(path: &Path, signature: &Path, key: &Path, source: &Path) -> Result<()> {
    let bytes = bounded_file(path, 64 * 1024)?;
    let signature = bounded_file(signature, 16 * 1024)?;
    let key = bounded_file(key, 8 * 1024)?;
    authenticate(&bytes, &signature, &key)?;
    let release: ReleaseManifest = serde_json::from_slice(&bytes).context("parse authenticated manifest")?;
    release.verify_local(source)?;
    println!("Release {} ({}) signature and local artifact hashes verified against the supplied publisher key.", release.version, release.revision);
    println!("Trust the public key only after independent authentication. Freshness, revocation, ABI and hardware compatibility are not established.");
    Ok(())
}

pub fn generate(args: &[String]) -> Result<()> {
    match args {
        [from, source, version_flag, version, revision_flag, revision, sequence_flag, sequence, products @ ..]
            if from == "--from" && version_flag == "--version" && revision_flag == "--revision" && sequence_flag == "--sequence" => {
                let mut release = create(Path::new(source), version, revision, products)?;
                release.schema = 2;
                release.sequence = Some(sequence.parse().context("invalid release sequence")?);
                release.validate()?;
                println!("{}", serde_json::to_string_pretty(&release)?);
                Ok(())
            }
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
    fn signer_rejects_policy_injection_and_unsupported_keys() {
        assert!(allowed_signer(b"ssh-rsa abc").is_err());
        assert!(allowed_signer(b"ssh-ed25519 abc\nattacker ssh-ed25519 abc").is_err());
        let blob = "A".repeat(68);
        let key = format!("ssh-ed25519 {blob} comment");
        assert_eq!(allowed_signer(key.as_bytes()).unwrap(),
            format!("atelier-release namespaces=\"atelier-release-v1\" ssh-ed25519 {blob}\n"));
    }

    #[test]
    fn openssh_signature_authenticates_exact_bytes_key_and_namespace() {
        let tmp = tempfile::tempdir().unwrap();
        let key = tmp.path().join("publisher");
        let other = tmp.path().join("other");
        for path in [&key, &other] {
            assert!(Command::new(SSH_KEYGEN).args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(path).status().unwrap().success());
        }
        let data = tmp.path().join("manifest.json");
        let bytes = b"fixture release bytes\n";
        fs::write(&data, bytes).unwrap();
        let sign = |namespace: &str| {
            assert!(Command::new(SSH_KEYGEN).args(["-Y", "sign", "-n", namespace, "-f"])
                .arg(&key).arg(&data).status().unwrap().success());
            let sig = tmp.path().join("manifest.json.sig");
            let result = fs::read(&sig).unwrap();
            fs::remove_file(sig).unwrap();
            result
        };
        let public = fs::read(tmp.path().join("publisher.pub")).unwrap();
        let wrong = fs::read(tmp.path().join("other.pub")).unwrap();
        let sig = sign(NAMESPACE);
        authenticate(bytes, &sig, &public).unwrap();
        assert!(authenticate(b"modified", &sig, &public).is_err());
        assert!(authenticate(bytes, &sig, &wrong).is_err());
        assert!(authenticate(bytes, &sign("other-namespace"), &public).is_err());
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
