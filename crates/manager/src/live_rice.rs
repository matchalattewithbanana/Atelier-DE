//! Live-only connection to the versioned Mango Atelier supervisor contract.
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{env, fs, io::{BufRead, BufReader, Read, Write}, os::{fd::AsRawFd, unix::{fs::{FileTypeExt, MetadataExt}, net::UnixStream}}, path::{Path, PathBuf}, time::Duration};

const LIMIT: u64 = 16 * 1024;
const MARKERS: &[(&str, &str)] = &[("mango","MANGO_INSTANCE_SIGNATURE"), ("hyprland","HYPRLAND_INSTANCE_SIGNATURE"), ("niri","NIRI_SOCKET")];

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Preview {
    pub rice: String,
    pub previous: String,
    pub seconds: u32,
    pub token: String,
}
#[derive(Debug, Deserialize)]
pub(super) struct Status {
    pub active: Option<String>,
    pub preview: Option<Preview>,
}
pub(super) enum Operation { Status, Begin(String), Apply(String), Revert(String) }

struct Session {
    compositor: &'static str,
    instance: String,
    display: String,
    socket: PathBuf,
}
fn valid_id(id: &str) -> bool {
    (2..=48).contains(&id.len()) && id.as_bytes()[0].is_ascii_lowercase()
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b,b'-'|b'_'))
}
fn valid_token(token: &str) -> bool {
    token.len() == 24 && token.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn private_directory(path: &Path, uid: u32) -> Result<()> {
    let meta = fs::symlink_metadata(path).with_context(|| format!("missing private runtime directory {}",path.display()))?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        bail!("unsafe runtime directory {}",path.display());
    }
    Ok(())
}
impl Session {
    fn detect() -> Result<Self> {
        if nix::unistd::Uid::effective().is_root() { bail!("run as the desktop user, not root"); }
        let markers: Vec<_> = MARKERS.iter().filter_map(|&(c,key)| env::var(key).ok().filter(|v| !v.is_empty()).map(|v|(c,v))).collect();
        let [(compositor, instance)] = markers.as_slice() else { bail!("exactly one live MangoWM, Hyprland or Niri instance marker is required"); };
        let display = env::var("WAYLAND_DISPLAY").ok().filter(|v| !v.is_empty()).context("live Wayland display required")?;
        if instance.len() > 1024 || display.len() > 1024 { bail!("invalid session identity"); }
        let runtime = PathBuf::from(env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is required")?);
        if !runtime.is_absolute() { bail!("XDG_RUNTIME_DIR must be absolute"); }
        let uid = nix::unistd::Uid::effective().as_raw();
        private_directory(&runtime,uid)?;
        let parent = runtime.join("mango-atelier");
        private_directory(&parent,uid)?;
        let socket = parent.join(if *compositor == "mango" { "rice.sock".to_owned() } else { format!("rice-{compositor}.sock") });
        let metadata = fs::symlink_metadata(&socket).context("no live Atelier supervisor socket; start a supported session first")?;
        if !metadata.file_type().is_socket() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
            bail!("unsafe rice supervisor socket");
        }
        Ok(Self { compositor, instance: instance.clone(), display, socket })
    }
    fn request(&self, operation: Operation) -> Result<Value> {
        let mut request = json!({"action":"manager-preview","version":1,"compositor":self.compositor,
            "display":self.display,"instance":self.instance});
        match operation {
            Operation::Status => request["operation"] = json!("status"),
            Operation::Begin(id) => {
                if !valid_id(&id) { bail!("enter a valid prepared rice ID"); }
                request["operation"] = json!("begin"); request["rice"] = json!(id);
            }
            Operation::Apply(token) | Operation::Revert(token) => {
                if !valid_token(&token) { bail!("refresh the preview first"); }
                // The caller must name the exact displayed preview.
                request["token"] = json!(token);
            }
        }
        Ok(request)
    }
}
fn same_user(stream: &UnixStream) -> Result<()> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: valid writable credential buffer with its exact length.
    let ret = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED,
        (&mut cred as *mut libc::ucred).cast(), &mut len) };
    if ret != 0 { return Err(std::io::Error::last_os_error().into()); }
    if len as usize != std::mem::size_of::<libc::ucred>() || cred.uid != nix::unistd::Uid::effective().as_raw() {
        bail!("rice supervisor peer is not the desktop user");
    }
    Ok(())
}
fn parse_reply(bytes: &[u8]) -> Result<Status> {
    let value: Value = serde_json::from_slice(bytes).context("invalid supervisor response")?;
    if value["ok"] != true {
        bail!("supervisor rejected request: {}",value["error"].as_str().unwrap_or("unknown error; update and restart the supervisor"));
    }
    if value["version"] != 1 || !value.as_object().is_some_and(|o| o.contains_key("preview") && o.contains_key("active")) {
        bail!("unsupported preview protocol; update Mango Atelier and log in again");
    }
    let status: Status = serde_json::from_value(value)?;
    if status.active.as_deref().is_some_and(|id| !valid_id(id)) { bail!("invalid active rice"); }
    if let Some(p) = &status.preview {
        if !valid_id(&p.rice) || !valid_id(&p.previous) || !valid_token(&p.token)
            || p.seconds > 30 || status.active.as_deref() != Some(p.rice.as_str()) {
            bail!("invalid preview identity");
        }
    }
    Ok(status)
}
pub(super) fn execute(operation: Operation) -> Result<Status> {
    let session = Session::detect()?;
    let action = match &operation { Operation::Apply(_) => Some("apply"), Operation::Revert(_) => Some("revert"), _ => None };
    let mut request = session.request(operation)?;
    if let Some(action) = action { request["operation"] = json!(action); }
    let mut stream = UnixStream::connect(&session.socket).context("connect to live rice supervisor")?;
    same_user(&stream)?;
    stream.set_read_timeout(Some(Duration::from_secs(40)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let mut bytes = serde_json::to_vec(&request)?;
    if bytes.len() > 4000 { bail!("session request exceeds protocol size"); }
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    let mut response = Vec::new();
    BufReader::new(stream).take(LIMIT+1).read_until(b'\n',&mut response)
        .context("preview response unavailable; refresh status before retrying, timer remains supervisor-owned")?;
    if response.len() as u64 > LIMIT || response.last() != Some(&b'\n') { bail!("incomplete or oversized supervisor response; refresh status"); }
    parse_reply(&response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replies_require_exact_version_and_token() {
        let valid = br#"{"ok":true,"version":1,"active":"sekaipop","preview":{"rice":"sekaipop","previous":"atelier","seconds":23,"token":"0123456789abcdef01234567"}}"#;
        let state = parse_reply(valid).unwrap();
        assert_eq!(state.preview.unwrap().seconds,23);
        assert!(parse_reply(br#"{"ok":true,"active":"atelier","preview":null}"#).is_err());
        assert!(parse_reply(br#"{"ok":true,"version":1,"active":"atelier"}"#).is_err());
        assert!(parse_reply(br#"{"ok":true,"version":1,"active":"atelier","preview":{"rice":"sekaipop","previous":"atelier","seconds":23,"token":"0123456789abcdef01234567"}}"#).is_err());
        assert!(parse_reply(br#"{"ok":true,"version":1,"active":"sekaipop","preview":{"rice":"sekaipop","previous":"atelier","seconds":23,"token":"stale"}}"#).is_err());
    }
    #[test]
    fn request_rejects_invalid_ids_and_tokens() {
        let session = Session { compositor:"mango",instance:"test".into(),display:"wayland-1".into(),socket:PathBuf::new() };
        assert!(session.request(Operation::Begin("--yes".into())).is_err());
        assert!(session.request(Operation::Apply("stale".into())).is_err());
        assert!(session.request(Operation::Revert("".into())).is_err());
        let req = session.request(Operation::Begin("sekaipop".into())).unwrap();
        assert_eq!(req["operation"],"begin");
        assert_eq!(req["display"],"wayland-1");
    }
}
