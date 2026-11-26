//! Bounded, read-only discovery of native release candidates. API metadata is untrusted.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{env, fs, io::{self, Read, Write}, os::unix::fs::{MetadataExt, PermissionsExt}, process::{Command, Stdio}};

const NATIVE: &str = "matchalattewithbanana/Atelier-DE";
const GH: &str = "/usr/bin/gh";
const TIMEOUT: &str = "/usr/bin/timeout";
const MAX_RESPONSE: u64 = 4 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub(super) struct ApiRelease {
    pub id: u64,
    tag_name: String,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<ApiAsset>,
}
#[derive(Debug, Deserialize)]
pub(super) struct ApiAsset {
    id: u64,
    name: String,
    state: String,
    size: u64,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
pub(super) struct Asset {
    pub id: u64,
    pub name: String,
    pub size: u64,
}
#[derive(Debug, Serialize)]
pub(super) struct Candidate {
    pub id: u64,
    pub tag: String,
    pub prerelease: bool,
    pub metadata_status: &'static str,
    manifest: Option<Asset>,
    signature: Option<Asset>,
}
pub(super) fn repository(choice: &str) -> Result<&'static str> {
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
fn request(endpoint: &str, binary: bool, limit: u64, output: &mut impl Write) -> Result<u64> {
    request_media(endpoint,binary,limit,output,if binary{Some("application/octet-stream")}else{None})
}
fn request_media(endpoint:&str,binary:bool,limit:u64,output:&mut impl Write,accept:Option<&str>)->Result<u64>{
    if nix::unistd::Uid::effective().is_root() { bail!("run release requests as your desktop user, not root"); }
    for path in [GH, TIMEOUT] { trusted_tool(path)?; }
    let mut command = Command::new(TIMEOUT);
    command.args(["--kill-after=2s", if binary { "180s" } else { "25s" }, GH, "api", "--hostname", "github.com", "--method", "GET"]);
    if let Some(media)=accept { command.args(["-H",&format!("Accept: {media}")]); }
    let mut child = command.arg(endpoint)
        .env("GH_PROMPT_DISABLED", "1").env("GH_PAGER", "cat").env_remove("GH_DEBUG")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().context("launch GitHub release query")?;
    let read = match child.stdout.take() {
        Some(stdout) => io::copy(&mut stdout.take(limit + 1), output),
        None => Err(io::Error::other("missing query stdout")),
    };
    // The pipe is closed after the bounded read. Keep the timeout supervisor
    // alive so its process-group deadline also applies to an abandoned gh child.
    let status = child.wait().context("wait for GitHub release query")?;
    let count = read.context("read GitHub release response")?;
    if count > limit { bail!("release response exceeds size limit"); }
    if !status.success() { bail!("GitHub release query failed ({status}); check network and gh auth status"); }
    Ok(count)
}
/// Public repository metadata search. Encode the entire query as data,
/// not an endpoint, shell expression, or caller-supplied option.
pub(super) fn search_repositories(query:&str)->Result<Vec<u8>>{
    let mut encoded=String::new();
    for byte in query.bytes(){
        if byte.is_ascii_alphanumeric()||matches!(byte,b'-'|b'_'|b'.'|b'~'){
            encoded.push(char::from(byte));
        }else{
            use std::fmt::Write;
            write!(&mut encoded,"%{byte:02X}")?;
        }
    }
    let mut bytes=Vec::new();
    request(&format!("search/repositories?q={encoded}&sort=stars&order=desc&per_page=20"),false,MAX_RESPONSE,&mut bytes)?;
    Ok(bytes)
}
pub(super) fn theme_tree(repo:&str,revision:&str)->Result<Vec<u8>>{
    let mut bytes=Vec::new();request(&format!("repos/{repo}/git/trees/{revision}?recursive=1"),false,MAX_RESPONSE,&mut bytes)?;Ok(bytes)
}
pub(super) fn theme_blob(repo:&str,sha:&str,size:u64)->Result<Vec<u8>>{
    let mut bytes=Vec::new();request_media(&format!("repos/{repo}/git/blobs/{sha}"),true,size,&mut bytes,Some("application/vnd.github.raw+json"))?;Ok(bytes)
}
fn fetch(repo: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    request(&format!("repos/{repo}/releases?per_page=20"), false, MAX_RESPONSE, &mut bytes)?;
    Ok(bytes)
}
pub(super) fn detail(repo: &str, id: u64, prerelease: bool) -> Result<ApiRelease> {
    if id == 0 { bail!("invalid release ID"); }
    let mut bytes = Vec::new();
    request(&format!("repos/{repo}/releases/{id}"), false, MAX_RESPONSE, &mut bytes)?;
    let release: ApiRelease = serde_json::from_slice(&bytes)?;
    if release.id != id || release.draft || (release.prerelease && !prerelease) || release.assets.len() > 1024 {
        bail!("release is unavailable or outside selected channel");
    }
    Ok(release)
}
pub(super) fn download(repo: &str, asset: &Asset, output: &mut impl Write) -> Result<()> {
    let count = request(&format!("repos/{repo}/releases/assets/{}", asset.id), true, asset.size, output)?;
    if count != asset.size { bail!("asset size mismatch: {}", asset.name); }
    Ok(())
}
pub(super) fn asset(assets: &[ApiAsset], name: &str, limit: u64) -> Option<Asset> {
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
pub(super) fn browse(choice: &str, prerelease: bool) -> Result<Vec<Candidate>> {
    let repo = repository(choice)?;
    candidates(&fetch(repo)?, env::consts::ARCH, prerelease)
}
pub fn run(choice: &str, prerelease: bool) -> Result<()> {
    let repo = repository(choice)?;
    let releases = browse(choice, prerelease)?;
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
