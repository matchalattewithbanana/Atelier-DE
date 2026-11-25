//! Shared customization discovery and read-only planning. Catalog records
//! are data, not executable instructions. Activation is a separate operation.
use anyhow::{bail,Context,Result};
use serde::Serialize;
use std::{collections::BTreeSet,path::{Path,PathBuf}};

pub const CATEGORIES:&[&str]=&["cursor","login","locker","icons","application-theme","wallpaper","shell","rice"];

#[derive(Clone,Debug,Serialize)]
pub struct Item{
    pub id:String,
    pub category:&'static str,
    pub name:String,
    pub source:String,
    pub declared_targets:Vec<String>,
    pub scope:&'static str,
    pub install:&'static str,
    pub activation:&'static str,
    pub readiness:String,
    pub detail:String,
}

struct Component{
    id:&'static str, category:&'static str,name:&'static str,source:&'static str,
    package:&'static str, binary:Option<&'static str>,scope:&'static str,detail:&'static str,
}
const COMPONENTS:&[Component]=&[
    Component{id:"sddm",category:"login",name:"SDDM",source:"https://github.com/sddm/sddm",package:"sddm",binary:Some("sddm"),scope:"system",detail:"Display manager with QML themes. Separate reviewed configuration, trusted theme and fallback greeter required. Never auto-enable or replace a running display manager."},
    Component{id:"greetd",category:"login",name:"greetd",source:"https://sr.ht/~kennylevinsen/greetd/",package:"greetd",binary:Some("greetd"),scope:"system",detail:"Login daemon; choose a compatible greeter separately. Requires explicit service/configuration and recovery review."},
    Component{id:"swaylock",category:"locker",name:"Swaylock",source:"https://github.com/swaywm/swaylock",package:"swaylock",binary:Some("swaylock"),scope:"session",detail:"Requires ext-session-lock-v1. Native Atelier locking not implemented; external compositor protocol and authentication must be verified."},
    Component{id:"hyprlock",category:"locker",name:"Hyprlock",source:"https://github.com/hyprwm/hyprlock",package:"hyprlock",binary:Some("hyprlock"),scope:"session",detail:"Configurable Wayland locker. Verify protocol, PAM and compositor support. Native Atelier locking not implemented."},
    Component{id:"gtklock",category:"locker",name:"GTKLock",source:"https://github.com/jovanlanik/gtklock",package:"gtklock",binary:Some("gtklock"),scope:"session",detail:"GTK-based Wayland locker. Verify session-lock, layer-shell and authentication support; native activation not implemented."},
    Component{id:"papirus",category:"icons",name:"Papirus",source:"https://github.com/PapirusDevelopmentTeam/papirus-icon-theme",package:"papirus-icon-theme",binary:None,scope:"user-preference",detail:"Icon pack. Installation does not select the theme in GTK, Qt or other desktop settings."},
    Component{id:"kvantum",category:"application-theme",name:"Kvantum",source:"https://github.com/tsujan/Kvantum",package:"kvantum",binary:Some("kvantummanager"),scope:"user-preference",detail:"Qt style engine; select a compatible Qt theme separately. Does not configure GTK."},
];
pub fn provider_package(id:&str)->Option<&'static str>{COMPONENTS.iter().find(|c|c.id==id).map(|c|c.package)}

pub(super) fn icon_roots()->Vec<PathBuf>{
    let home=std::env::var_os("HOME").map(PathBuf::from).filter(|p|p.is_absolute());
    let data=std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p|p.is_absolute())
        .or_else(||home.as_ref().map(|p|p.join(".local/share")));
    let mut roots=Vec::new();
    if let Some(data)=data{roots.push(data.join("icons"));}
    if let Some(home)=home{roots.push(home.join(".icons"));}
    if let Some(path)=std::env::var_os("XCURSOR_PATH"){
        roots.extend(std::env::split_paths(&path).filter(|p|p.is_absolute()).take(32));
    }
    let data=std::env::var_os("XDG_DATA_DIRS").unwrap_or_else(||"/usr/local/share:/usr/share".into());
    roots.extend(std::env::split_paths(&data).filter(|p|p.is_absolute()).take(32).map(|p|p.join("icons")));
    roots
}
fn cursor_present(roots:&[PathBuf],theme:&str)->bool{
    roots.iter().any(|p|["default","left_ptr"].iter().any(|name|p.join(theme).join("cursors").join(name).is_file()))
}
fn login_themes(root:&Path)->Vec<String>{
    let mut names=BTreeSet::new();
    if let Ok(entries)=std::fs::read_dir(root){
        for entry in entries.take(1024).flatten(){
            let Ok(name)=entry.file_name().into_string()else{continue;};
            if name.len()>128||name.is_empty()||!name.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b'_'|b'.')){continue;}
            if entry.path().join("Main.qml").is_file()||entry.path().join("metadata.desktop").is_file(){names.insert(name);}
        }
    }
    names.into_iter().collect()
}

pub fn catalog()->Vec<Item>{
    let roots=icon_roots();
    let mut items=Vec::new();
    for theme in atelier_config::CURSOR_PRESETS{
        let source=if theme.starts_with("Bibata-"){"https://github.com/ful1e5/Bibata_Cursor/releases/tag/v2.0.7"}else{"https://github.com/ful1e5/Google_Cursor/releases/tag/v2.0.0"};
        items.push(Item{id:(*theme).into(),category:"cursor",name:(*theme).into(),source:source.into(),declared_targets:vec![],scope:"user",install:"user-local cursor pack installer; explicit --install",
            activation:"native Settings; next session",readiness:if cursor_present(&roots,theme){"Files detected"}else{"Not detected"}.into(),
            detail:"GPL-3.0 upstream asset. File presence is not proof of image validity. External desktops use their own cursor settings.".into()});
    }
    for c in COMPONENTS{
        let found=c.binary.is_some_and(|bin|Path::new("/usr/bin").join(bin).is_file());
        items.push(Item{id:c.id.into(),category:c.category,name:c.name.into(),source:c.source.into(),declared_targets:vec![],scope:c.scope,
            install:"optional reviewed Arch-family package transaction",activation:"not implemented in catalog",
            readiness:if c.binary.is_none(){"Inspect installed files"}else if found{"Executable detected"}else{"Not detected"}.into(),
            detail:format!("Package candidate: {}. {}",c.package,c.detail)});
    }
    for name in login_themes(Path::new("/usr/share/sddm/themes")){
        items.push(Item{id:format!("sddm-theme:{name}"),category:"login",name:format!("SDDM theme: {name}"),source:"https://github.com/sddm/sddm/wiki/Theming".into(),declared_targets:vec![],scope:"system",
            install:"existing local files; no installation",activation:"not implemented; trusted greeter test and reviewed system config required",
            readiness:"Theme files detected".into(),detail:"Local discovery only. QML themes are executable code; inspect and trust before running a test greeter. No greeter will be launched by the catalog.".into()});
    }
    for (id,category,name,detail) in [
        ("native-wallpaper","wallpaper","Native wallpaper","Use the existing Wallpapers page and appearance backup."),
        ("native-shell","shell","Native shell","Use verified deployments and native session shell profiles."),
        ("mango-rices","rice","Mango Atelier rices","Use the existing Rice Manager and rice backups for MangoWM, Hyprland and Niri."),
    ]{
        items.push(Item{id:id.into(),category,name:name.into(),source:"https://github.com/matchalattewithbanana/Atelier-DE".into(),declared_targets:vec![],scope:"user",install:"existing dedicated workflow",activation:"existing dedicated workflow",readiness:"Delegated".into(),detail:detail.into()});
    }
    items
}
pub fn filtered(category:Option<&str>)->Result<Vec<Item>>{
    if category.is_some_and(|c|!CATEGORIES.contains(&c)){bail!("unknown customization category");}
    let mut items=catalog();
    items.extend(crate::customization_registry::items()?);
    Ok(items.into_iter().filter(|item|category.is_none_or(|c|item.category==c)).collect())
}
pub fn preview(id:&str)->Result<Item>{filtered(None)?.into_iter().find(|item|item.id==id).context("unknown customization ID")}
pub fn run(args:&[String])->Result<()>{
    if args.first().is_some_and(|a|a=="discover"){
        return crate::customization_discovery::run(&args[1..]);
    }
    if args.first().is_some_and(|a|matches!(a.as_str(),"register"|"register-source"|"unregister"|"restore-registry")){
        return crate::customization_registry::run(args);
    }
    let json=args.last().is_some_and(|arg|arg=="--json");
    let args=if json{&args[..args.len()-1]}else{args};
    let items=match args{
        []=>filtered(None)?,
        [action] if action=="list"=>filtered(None)?,
        [action,category] if action=="list"=>filtered(Some(category))?,
        [action,id] if action=="preview"=>vec![preview(id)?],
        _=>bail!("use customizations list [CATEGORY] or customizations preview ID [--json]"),
    };
    if json{println!("{}",serde_json::to_string_pretty(&items)?);}
    else{
        println!("Customization review — no changes");
        for item in items{println!("{} — {} [{}; {}]\n  Source: {}\n  Scope: {}\n  Declared targets: {}\n  Prepare: {}\n  Select: {}\n  {}",item.id,item.name,item.category,item.readiness,item.source,item.scope,if item.declared_targets.is_empty(){"not specified".into()}else{item.declared_targets.join(", ")},item.install,item.activation,item.detail);}
    }
    Ok(())
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn catalog_has_unique_ids_and_no_implicit_activation(){
        let items=catalog();
        let ids:BTreeSet<_>=items.iter().map(|item|&item.id).collect();
        assert_eq!(ids.len(),items.len());
        assert_eq!(items.iter().filter(|i|i.category=="cursor").count(),16);
        assert!(filtered(Some("unknown")).is_err());
        assert_eq!(provider_package("sddm"),Some("sddm"));
        assert_eq!(provider_package("Bibata-Modern-Ice"),None);
        assert!(items.iter().find(|i|i.id=="swaylock").unwrap().detail.contains("not implemented"));
        assert!(items.iter().find(|i|i.id=="sddm").unwrap().activation.contains("not implemented"));
    }
    #[test]
    fn discovers_only_named_local_theme_records(){
        let dir=tempfile::tempdir().unwrap();
        for name in ["good","empty","bad name"]{std::fs::create_dir(dir.path().join(name)).unwrap();}
        for name in ["good","bad name"]{std::fs::write(dir.path().join(name).join("Main.qml"),"unexecuted").unwrap();}
        assert_eq!(login_themes(dir.path()),vec!["good"]);
    }
}
