//! Optional MPRIS bridge using the installed playerctl command.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::process::{Command, Output};

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MediaSnapshot {
    pub schema: u32,
    pub players: Vec<Player>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Player {
    pub id: String,
    pub status: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum MediaAction { Play, Pause, Toggle, Next, Previous }

impl MediaAction {
    pub fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "play" => Self::Play, "pause" => Self::Pause, "play-pause" => Self::Toggle,
            "next" => Self::Next, "previous" => Self::Previous,
            _ => bail!("unsupported media action: {value}"),
        })
    }
    fn command(self) -> &'static str {
        match self { Self::Play => "play", Self::Pause => "pause", Self::Toggle => "play-pause", Self::Next => "next", Self::Previous => "previous" }
    }
}

fn run(args: &[&str]) -> Result<Output> {
    Command::new("timeout").args(["--kill-after=1s", "3s", "playerctl"]).args(args).env("LC_ALL", "C").output()
        .context("run playerctl through coreutils timeout (both required for MPRIS support)")
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 255 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !id.starts_with('-')
}

fn discover(output: Output) -> Result<Vec<String>> {
    if !output.status.success() {
        // playerctl reports no players as an error on some versions.
        let message = String::from_utf8_lossy(&output.stderr);
        if message.trim() == "No players found" && output.stdout.is_empty() { return Ok(Vec::new()); }
        bail!("playerctl discovery failed: {}", message.trim());
    }
    let text = String::from_utf8(output.stdout).context("invalid player list")?;
    let mut ids: Vec<_> = text.lines().map(str::trim).filter(|id| valid_id(id)).map(str::to_owned).collect();
    ids.sort(); ids.dedup();
    Ok(ids)
}

fn property(output: Result<Output>) -> Option<String> {
    let output = output.ok()?;
    if !output.status.success() { return None; }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim_end_matches(['\r', '\n']);
    if text.is_empty() { None } else { Some(text.to_owned()) }
}

fn snapshot_with(mut call: impl FnMut(&[&str]) -> Result<Output>) -> Result<MediaSnapshot> {
    let ids = discover(call(&["--list-all"])?)?;
    let mut players = Vec::new();
    for id in ids {
        let selector = format!("--player={id}");
        let status = property(call(&[&selector, "status"])).filter(|s| matches!(s.as_str(), "Playing" | "Paused" | "Stopped"));
        let title = property(call(&[&selector, "metadata", "xesam:title"]));
        let artist = property(call(&[&selector, "metadata", "xesam:artist"]));
        players.push(Player { id, status, title, artist });
    }
    Ok(MediaSnapshot { schema: 1, players })
}

pub fn media_snapshot() -> Result<MediaSnapshot> { snapshot_with(run) }

fn action_with(id: &str, action: MediaAction, mut call: impl FnMut(&[&str]) -> Result<Output>) -> Result<()> {
    if !valid_id(id) { bail!("invalid media player identifier"); }
    let ids = discover(call(&["--list-all"])?)?;
    if !ids.iter().any(|candidate| candidate == id) { bail!("selected player is not available"); }
    // A base name can otherwise match another instance in playerctl.
    if ids.iter().any(|candidate| candidate.starts_with(&format!("{id}."))) {
        bail!("ambiguous player name; select an instance");
    }
    let selector = format!("--player={id}");
    let result = call(&[&selector, action.command()])?;
    if !result.status.success() { bail!("media action failed: {}", String::from_utf8_lossy(&result.stderr).trim()); }
    Ok(())
}

pub fn media_action(id: &str, action: MediaAction) -> Result<()> { action_with(id, action, run) }

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    fn output(code: i32, stdout: &str, stderr: &str) -> Output {
        Output { status: std::process::ExitStatus::from_raw(code << 8), stdout: stdout.as_bytes().to_vec(), stderr: stderr.as_bytes().to_vec() }
    }
    #[test]
    fn unavailable_and_empty_are_distinct() {
        assert!(discover(output(1, "", "No players found\n")).unwrap().is_empty());
        assert!(discover(output(1, "", "DBus unavailable")).is_err());
    }
    #[test]
    fn metadata_is_literal_and_missing_fields_are_null() {
        let snapshot = snapshot_with(|args| Ok(match args {
            ["--list-all"] => output(0, "vlc\nvlc\n--all-players\n", ""),
            ["--player=vlc", "status"] => output(0, "Playing\n", ""),
            ["--player=vlc", "metadata", "xesam:title"] => output(0, "A | B\n", ""),
            _ => output(1, "", "missing"),
        })).unwrap();
        assert_eq!(snapshot.players.len(), 1);
        assert_eq!(snapshot.players[0].title.as_deref(), Some("A | B"));
        assert_eq!(snapshot.players[0].artist, None);
    }
    #[test]
    fn actions_require_a_known_unambiguous_player() {
        assert!(MediaAction::parse("open").is_err());
        assert!(action_with("vlc", MediaAction::Next, |_| Ok(output(0, "vlc\nvlc.instance1\n", ""))).is_err());
        assert!(action_with("missing", MediaAction::Play, |_| Ok(output(0, "vlc\n", ""))).is_err());
        let mut calls = Vec::new();
        action_with("vlc.instance1", MediaAction::Pause, |args| {
            calls.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            Ok(output(0, "vlc.instance1\n", ""))
        }).unwrap();
        assert_eq!(calls[1], vec!["--player=vlc.instance1", "pause"]);
    }
}
