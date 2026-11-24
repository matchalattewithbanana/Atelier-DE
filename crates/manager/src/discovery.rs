//! Bounded, read-only discovery of native release candidates. API metadata is untrusted.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{env, fs, io::Read, os::unix::fs::{MetadataExt, PermissionsExt}, process::{Command, Stdio}};

const NATIVE: &str = "matchalattewithbanana/Atelier-DE";
const GH: &str = "/usr/bin/gh";
const TIMEOUT: &str = "/usr/bin/timeout";
const MAX_RESPONSE: u64 = 4 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct ApiRelease {
    id: u64,
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ApiAsset>,
}
#[derive(Debug, Deserialize)]
struct ApiAsset {
    id: u64,
    name: String,
    state: String,
    size: u64,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
struct Asset {
    id: u64,
    name: String,
    size: u64,
}
#[derive(Debug, Serialize)]
struct Candidate {
    id: u64,
    tag: String,
    prerelease: bool,
    metadata_status: &'static str,
    manifest: Option<Asset>,
    signature: Option<Asset>,
}
fn repository(choice: &str) -> Result<&'static str> {
    match choice {
        "native" => Ok(NATIVE),
        _ => bail!("unknown release source; native is the only supported release source"),
    }
}
fn trusted_tool(path: &str) -> Result<()> {
    let m = fs::metadata(path).with_context(|| format!("required system executable unavailable: {path}"))?;
    if !m.is_file() || m.uid() != 0 || m.permissions().mode() & 0o022 != 0 || m.permissions().mode() & 0o111 == 0 {
        bail!("unsafe system executable: {path}");
    }
    Ok(())
}
fn fetch(repo: &str) -> Result<Vec<u8>> {
    if nix::unistd::Uid::effective().is_root() { bail!("run release discovery as your desktop user, not root"); }
    for path in [GH, TIMEOUT] { trusted_tool(path)?; }
    let endpoint = format!("repos/{repo}/releases?per_page=20");
    let mut child = Command::new(TIMEOUT)
        .args(["--kill-after=2s", "25s", GH, "api", "--hostname", "github.com", "--method", "GET"])
        .arg(endpoint)
        .env("GH_PROMPT_DISABLED", "1").env("GH_PAGER", "cat").env_remove("GH_DEBUG")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().context("launch GitHub release query")?;
    let mut bytes = Vec::new();
    let read = child.stdout.take().context("missing query stdout")?
        .take(MAX_RESPONSE + 1).read_to_end(&mut bytes);
    let status = child.wait().context("wait for GitHub release query")?;
    read.context("read GitHub release response")?;
    if bytes.len() as u64 > MAX_RESPONSE { bail!("release response exceeds 4 MiB"); }
    if !status.success() { bail!("GitHub release query failed ({status}); check network and gh auth status"); }
    Ok(bytes)
}
fn asset(assets: &[ApiAsset], name: &str, limit: u64) -> Option<Asset> {
    let mut matches = assets.iter().filter(|a| a.name == name);
    let a = matches.next()?;
    if matches.next().is_some() || a.id == 0 || a.state != "uploaded" || a.size == 0 || a.size > limit {
        return None;
    }
    Some(Asset { id: a.id, name: a.name.clone(), size: a.size })
}
fn candidates(bytes: &[u8], arch: &str, prerelease: bool) -> Result<Vec<Candidate>> {
    let releases: Vec<ApiRelease> = serde_json::from_slice(bytes).context("parse GitHub release metadata")?;
    if releases.len() > 20 { bail!("unexpected release page size"); }
    let name = format!("atelier-release-linux-{arch}.json");
    let signature = format!("{name}.sig");
    let mut result = Vec::new();
    for r in releases {
        if r.draft || (r.prerelease && !prerelease) { continue; }
        if r.id == 0 || r.tag_name.is_empty() || r.tag_name.len() > 256 || r.assets.len() > 1024 {
            bail!("invalid release metadata");
        }
        let manifest = asset(&r.assets, &name, 64 * 1024);
        let sig = asset(&r.assets, &signature, 16 * 1024);
        let metadata_status = if manifest.is_some() && sig.is_some() { "signature-not-verified" } else { "metadata-missing-or-invalid" };
        result.push(Candidate { id: r.id, tag: r.tag_name, prerelease: r.prerelease, metadata_status, manifest, signature: sig });
    }
    Ok(result)
}
pub fn run(choice: &str, prerelease: bool) -> Result<()> {
    let repo = repository(choice)?;
    let bytes = fetch(repo)?;
    let releases = candidates(&bytes, env::consts::ARCH, prerelease)?;
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "schema": 1, "repository": repo, "host": "github.com",
        "target": format!("linux-{}", env::consts::ARCH),
        "include_prerelease": prerelease, "page_limit":20,
        "releases": releases,
        "note":"Discovery only. API tags, asset IDs and sizes are not authenticated release identities. No assets downloaded, no update selected, no installation."
    }))?);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn release(id: u64, draft: bool, prerelease: bool, assets: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"id":id,"tag_name":format!("v{id}"),"draft":draft,"prerelease":prerelease,"assets":assets})
    }
    fn assets() -> serde_json::Value {
        serde_json::json!([
            {"id":10,"name":"atelier-release-linux-x86_64.json","state":"uploaded","size":100},
            {"id":11,"name":"atelier-release-linux-x86_64.json.sig","state":"uploaded","size":200}
        ])
    }
    #[test]
    fn source_is_allowlisted_and_drafts_never_appear() {
        assert_eq!(repository("native").unwrap(), NATIVE);
        assert!(repository("other/repo").is_err());
        let data = serde_json::to_vec(&vec![release(1,false,false,assets()), release(2,false,true,assets()), release(3,true,false,assets())]).unwrap();
        let stable = candidates(&data,"x86_64",false).unwrap();
        assert_eq!(stable.len(),1);
        assert_eq!(stable[0].metadata_status,"signature-not-verified");
        assert_eq!(candidates(&data,"x86_64",true).unwrap().len(),2);
        assert_eq!(candidates(&data,"aarch64",false).unwrap()[0].metadata_status,"metadata-missing-or-invalid");
    }
    #[test]
    fn rejects_duplicate_or_oversized_metadata_assets() {
        let mut a = assets();
        let duplicate = a[0].clone();
        a.as_array_mut().unwrap().push(duplicate);
        let data = serde_json::to_vec(&vec![release(1,false,false,a)]).unwrap();
        assert!(candidates(&data,"x86_64",false).unwrap()[0].manifest.is_none());
        let mut a = assets();
        a[1]["size"] = serde_json::json!(16385);
        let data = serde_json::to_vec(&vec![release(1,false,false,a)]).unwrap();
        assert!(candidates(&data,"x86_64",false).unwrap()[0].signature.is_none());
    }
}
