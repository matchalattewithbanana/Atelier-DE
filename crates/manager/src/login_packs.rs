//! Curated upstream collections: exact revisions, one reviewed theme at a
//! time. QML is executable code; preparation never runs it.
use anyhow::{bail,Context,Result};
use serde::Deserialize;
use sha1::{Digest,Sha1};
use std::{collections::BTreeSet,fs,io::{self,IsTerminal,Write},path::{Component,Path,PathBuf}};
use crate::{customization::Item,discovery,login_theme};

const RD:&str="rd6260/sddm-themes";
const RD_REV:&str="e5362dae5b38e79527737d4b32e95f001f8aa9e2";
const QY:&str="Darkkal44/qylock";
const QY_REV:&str="c2933f27b655a999911e6699e1b2c67e59cbd428";
const RD_THEMES:&[&str]=&["CBRPNK","ironclad","nothing","pinkslip"];
const QY_THEMES:&[&str]=&["Genshin","R1999_1","R1999_2","clockwork/neo-orbital","clockwork/orbital","clockwork/tape","dog-samurai","enfield","field","forest","girl-coffee","girl-pillow","last-of-us","man-bicycle","material-you-dark","material-you","minecraft","nier-automata","ninesols","ninja_gaiden","nothing","osu","osumania","pixel-coffee","pixel-cyberpunk","pixel-dusk-city","pixel-emerald","pixel-hollowknight","pixel-munchlax","pixel-night-city","pixel-rainyroom","pixel-sakura","pixel-skyscrapers","pixel-waterfall","skyrim","star-rail","sword","terraria","windows_7","winter","women-umbrella","wuwa"];
const MAX_FILE:u64=32*1024*1024;
const MAX_TOTAL:u64=64*1024*1024;
#[derive(Clone,Copy)]
struct Pack{prefix:&'static str,repo:&'static str,revision:&'static str,license:&'static str,themes:&'static [&'static str]}
const PACKS:[Pack;2]=[
    Pack{prefix:"rd",repo:RD,revision:RD_REV,license:"MIT",themes:RD_THEMES},
    Pack{prefix:"qylock",repo:QY,revision:QY_REV,license:"GPL-3.0",themes:QY_THEMES},
];
fn oversized(pack:Pack,theme:&str)->bool{
    pack.prefix=="qylock"&&matches!(theme,"forest"|"last-of-us"|"pixel-hollowknight"|"sword")
}
fn id(pack:Pack,theme:&str)->String{format!("login-pack:{}:{theme}",pack.prefix)}
fn lookup(value:&str)->Result<(Pack,&'static str)>{
    for pack in PACKS{for &theme in pack.themes{if id(pack,theme)==value{return Ok((pack,theme));}}}
    bail!("unknown curated login theme ID")
}
pub(super) fn items()->Vec<Item>{
    let mut result=Vec::new();
    for pack in PACKS{for &theme in pack.themes{
        let blocked=oversized(pack,theme);
        result.push(Item{id:id(pack,theme),category:"login",name:format!("{} / {theme}",pack.prefix),
            source:format!("https://github.com/{}/tree/{}/themes/{theme}",pack.repo,pack.revision),
            declared_targets:vec![],scope:"user-local staging",
            install:if blocked{"exceeds current staging limits; upstream reference only"}else{"atelier-manager login-theme pack-prepare ID (reviewed inert staging)"},
            activation:"separate trusted SDDM review; no automatic installation or activation",
            readiness:if blocked{"Oversized upstream media"}else{"Pinned upstream source; not installed"}.into(),
            detail:format!("{} code license; assets and fonts may have separate terms. Requires compatible SDDM/Qt. QML trust and compatibility not verified.",pack.license)});
    }}
    result.push(Item{id:"qylock-locker-pack".into(),category:"locker",name:"Qylock — Quickshell lock-screen collection".into(),
        source:format!("https://github.com/{QY}/tree/{QY_REV}"),declared_targets:vec![],scope:"external session",
        install:"external reference only; Quickshell adapter requires independent review",activation:"not implemented; never use cosmetic fullscreen windows as locks",
        readiness:"Pinned external source; not validated".into(),detail:"GPL-3.0 code, separately licensed media/fonts. Wayland ext-session-lock support and PAM integration must be validated. Never run upstream cleanup or unlock commands from Atelier.".into()});
    result
}

#[derive(Deserialize)]
struct Tree{truncated:bool,tree:Vec<Node>}
#[derive(Deserialize)]
struct Node{path:String,mode:String,#[serde(rename="type")] kind:String,sha:String,size:Option<u64>}
struct FileSpec{path:PathBuf,sha:String,size:u64}
fn valid_sha(value:&str)->bool{value.len()==40&&value.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))}
fn relative(value:&str)->Result<PathBuf>{
    let p=Path::new(value);
    if value.is_empty()||value.len()>512||value.contains('\\')||value.chars().any(char::is_control)
        ||p.components().count()>8||!p.components().all(|c|matches!(c,Component::Normal(_))){bail!("unsafe upstream path");}
    Ok(p.into())
}
fn plan(tree:Tree,theme:&str)->Result<Vec<FileSpec>>{
    if tree.truncated||tree.tree.len()>20000{bail!("incomplete or oversized upstream tree");}
    let prefix=format!("themes/{theme}/");
    let mut result=Vec::new();let mut paths=BTreeSet::new();let mut total=0u64;
    for node in tree.tree{
        let relative_name=if node.path=="LICENSE"{"UPSTREAM-LICENSE.txt"}else if let Some(path)=node.path.strip_prefix(&prefix){path}else{continue;};
        if node.kind=="tree"&&node.mode=="040000"{continue;}
        if node.kind!="blob"||node.mode!="100644"&&node.mode!="100755"||!valid_sha(&node.sha){bail!("unsupported upstream entry");}
        let path=relative(relative_name)?;
        if path==Path::new("UPSTREAM-SOURCE.txt")||!paths.insert(path.clone()){bail!("duplicate or reserved upstream path");}
        let size=node.size.context("missing blob size")?;
        total=total.checked_add(size).context("theme too large")?;
        // Reserve space and entries for provenance and staging metadata.
        if size>MAX_FILE||total>MAX_TOTAL-4096||result.len()>=500{bail!("selected theme exceeds existing staging limits");}
        result.push(FileSpec{path,sha:node.sha,size});
    }
    if !paths.contains(Path::new("Main.qml"))||!paths.contains(Path::new("UPSTREAM-LICENSE.txt")){bail!("theme entry point or upstream license missing");}
    Ok(result)
}
fn verify(bytes:&[u8],file:&FileSpec)->Result<()>{
    let mut hash=Sha1::new();hash.update(format!("blob {}\0",bytes.len()).as_bytes());hash.update(bytes);
    if bytes.len() as u64!=file.size||format!("{:x}",hash.finalize())!=file.sha{bail!("upstream blob integrity mismatch");}Ok(())
}
fn confirm(phrase:&str)->Result<bool>{
    print!("Type '{phrase}': ");io::stdout().flush()?;
    let mut input=String::new();io::stdin().read_line(&mut input)?;Ok(input.trim()==phrase)
}
pub(super) fn prepare(value:&str)->Result<()>{
    if nix::unistd::Uid::effective().is_root()||!io::stdin().is_terminal()||!io::stdout().is_terminal(){bail!("interactive desktop-user review required");}
    let (pack,theme)=lookup(value)?;
    if oversized(pack,theme){bail!("selected upstream media exceeds the current staging limit");}
    println!("Source: https://github.com/{}\nRevision: {}\nTheme: {theme}\nCode license: {} (media/fonts may be separately licensed).\nDownload only this theme and upstream license into temporary user data. QML is untrusted executable code; no execution or system changes.",pack.repo,pack.revision,pack.license);
    if !confirm(&format!("fetch {value}"))?{return Ok(());}
    let review=fetch_review(value)?;
    println!("{}",review.detail()?);
    let phrase=review.phrase();
    if confirm(&phrase)?{println!("Prepared inert theme copy: {}",review.apply(&phrase)?.display());}
    Ok(())
}
/// Fetch only after explicit CLI or graphical user selection. Retain the
/// temporary directory for the existing screenshot and fingerprint review.
pub(super) fn fetch_review(value:&str)->Result<login_theme::Review>{
    if nix::unistd::Uid::effective().is_root(){bail!("run as the desktop user, not root");}
    let (pack,theme)=lookup(value)?;
    if oversized(pack,theme){bail!("selected upstream media exceeds the current staging limit");}
    let tree=serde_json::from_slice(&discovery::theme_tree(pack.repo,pack.revision)?)?;
    let files=plan(tree,theme)?;
    let temporary=tempfile::tempdir()?;let root=temporary.path().join("theme");fs::create_dir(&root)?;
    for file in files{
        let bytes=discovery::theme_blob(pack.repo,&file.sha,file.size)?;
        verify(&bytes,&file)?;
        let dest=root.join(&file.path);fs::create_dir_all(dest.parent().context("missing parent")?)?;
        let mut output=fs::OpenOptions::new().create_new(true).write(true).open(dest)?;output.write_all(&bytes)?;
    }
    fs::write(root.join("UPSTREAM-SOURCE.txt"),format!("https://github.com/{}/tree/{}/themes/{theme}\nCode license: {}\nAsset/font rights and QML compatibility require independent review.\n",pack.repo,pack.revision,pack.license))?;
    Ok(login_theme::inspect(&root)?.retain_source(temporary))
}
#[cfg(test)]
mod tests{
    use super::*;
    fn node(path:&str,mode:&str,size:u64)->Node{Node{path:path.into(),mode:mode.into(),kind:"blob".into(),sha:"a".repeat(40),size:Some(size)}}
    fn tree(extra:Option<Node>)->Tree{
        let mut nodes=vec![node("LICENSE","100644",10),node("themes/nothing/Main.qml","100644",20),node("themes/other/Main.qml","100644",99)];
        nodes.extend(extra);Tree{truncated:false,tree:nodes}
    }
    #[test]
    fn exact_curated_ids_and_bounds(){
        assert_eq!(items().len(),47);
        assert!(lookup("login-pack:qylock:clockwork/tape").is_ok());
        assert!(lookup("login-pack:rd:../nothing").is_err());
        assert!(oversized(PACKS[1],"forest"));
        assert_eq!(plan(tree(None),"nothing").unwrap().len(),2);
        for bad in [node("themes/nothing/../evil","100644",1),node("themes/nothing/link","120000",1),node("themes/nothing/video","100644",MAX_FILE+1)]{
            assert!(plan(tree(Some(bad)),"nothing").is_err());
        }
    }
    #[test]
    fn blob_size_and_git_object_hash_checked(){
        let file=FileSpec{path:"x".into(),sha:"b6fc4c620b67d95f953a5c1c1230aaab5db5a1b0".into(),size:5};
        assert!(verify(b"hello",&file).is_ok());assert!(verify(b"world",&file).is_err());
    }
}
