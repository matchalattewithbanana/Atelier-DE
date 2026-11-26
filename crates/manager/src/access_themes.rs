//! Native theme browsing and export, independent of the active login service.
use anyhow::{bail,Result};
use atelier_config::{access_theme::{self,Theme,Backdrop,ImageFit,PRESETS},access_bundle::{self,Bundle}};
use std::path::Path;
use atelier_config::{design::Color,dynamic_palette::{self,Contrast,Options,Variant}};

pub(super) fn run(args:&[String])->Result<()>{
    match args{
        [action] if action=="list"=>{
            for (id,name) in PRESETS{println!("{id} — {name}");}
        }
        [action,id] if action=="show"=>println!("{}",access_theme::preset(id)?.json()?),
        [action,path] if action=="check"=>{
            let path=Path::new(path);
            let theme=if path.extension().and_then(|v|v.to_str())==Some("atelier-access"){Bundle::import(path)?.theme}else{
                let theme=Theme::import(path)?;
                if theme.backdrop.needs_image(){bail!("wallpaper themes require an .atelier-access bundle");}theme
            };
            println!("Valid native access theme: {} (v{}). Appearance only; no settings changed.",theme.name,theme.version);
        }
        [action,id,path] if action=="export"=>{
            access_theme::preset(id)?.export(Path::new(path))?;
            println!("Exported new theme file: {path}");
        }
        [action,id,image,path] if action=="wallpaper-bundle"=>{
            let mut theme=access_theme::preset(id)?;
            theme.backdrop=Backdrop::Wallpaper{fit:ImageFit::Cover,dim:25};
            let raster=access_bundle::load_wallpaper(Path::new(image))?;
            Bundle::export(&theme,&raster,Path::new(path))?;
            println!("Exported new static wallpaper bundle: {path}");
        }
        [action,id,seed,variant,mode,contrast,path] if action=="generate"=>{
            let mut theme=access_theme::preset(id)?;
            let light=match mode.as_str(){"light"=>true,"dark"=>false,_=>bail!("choose light or dark")};
            let contrast=match contrast.as_str(){"normal"=>Contrast::Normal,"high"=>Contrast::High,_=>bail!("choose normal or high contrast")};
            theme.palette=dynamic_palette::generate(Color::parse(seed)?.rgb(),Options{variant:Variant::parse(variant)?,light,contrast})?.access;
            theme.export(Path::new(path))?;
            println!("Exported new Material palette theme: {path}");
        }
        _=>bail!("use access-theme list | show ID | check FILE_OR_BUNDLE | export ID FILE | wallpaper-bundle ID IMAGE DIR | generate ID HEX VARIANT light|dark normal|high FILE"),
    }
    Ok(())
}
