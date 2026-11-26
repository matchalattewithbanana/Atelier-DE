//! Native theme browsing and export, independent of the active login service.
use anyhow::{bail,Result};
use atelier_config::access_theme::{self,Theme,PRESETS};
use std::path::Path;

pub(super) fn run(args:&[String])->Result<()>{
    match args{
        [action] if action=="list"=>{
            for (id,name) in PRESETS{println!("{id} — {name}");}
        }
        [action,id] if action=="show"=>println!("{}",access_theme::preset(id)?.json()?),
        [action,path] if action=="check"=>{
            let theme=Theme::import(Path::new(path))?;
            println!("Valid native access theme: {} (v{}). Appearance only; no settings changed.",theme.name,theme.version);
        }
        [action,id,path] if action=="export"=>{
            access_theme::preset(id)?.export(Path::new(path))?;
            println!("Exported new theme file: {path}");
        }
        _=>bail!("use access-theme list | show ID | check FILE | export ID FILE"),
    }
    Ok(())
}
