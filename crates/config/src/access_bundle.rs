//! Portable static wallpaper bundles. Only a fixed JSON manifest and a
//! normalized raster are read or published. No external references or code.
use anyhow::{bail,Context,Result};
use crate::{access_theme::Theme,wallpaper::{self,Raster}};
use image::ImageEncoder;
use std::{ffi::CString,fs::{self,File,OpenOptions},io::Write,os::unix::{ffi::OsStrExt,fs::OpenOptionsExt},path::Path};

pub const MAX_EDGE:u32=2048;
fn bundle_path(path:&Path)->Result<()>{
    if !path.is_absolute()||path.extension().and_then(|s|s.to_str())!=Some("atelier-access"){
        bail!("use an absolute .atelier-access bundle directory");
    }
    Ok(())
}
fn validate(theme:&Theme,image:&Raster)->Result<()>{
    theme.validate()?;
    if !theme.backdrop.needs_image(){bail!("wallpaper bundle requires a wallpaper backdrop");}
    if image.width==0||image.height==0||image.width>MAX_EDGE||image.height>MAX_EDGE||
        u64::from(image.width)*u64::from(image.height)*4!=image.rgba.len() as u64{
        bail!("invalid wallpaper raster");
    }
    Ok(())
}
/// Read a user-selected static image with the same bounded decoder used by
/// the wallpaper page. Decode once; callers retain only normalized pixels.
pub fn load_wallpaper(path:&Path)->Result<Raster>{wallpaper::decode(path,MAX_EDGE)}
pub struct Bundle{pub theme:Theme,pub wallpaper:Raster}
impl Bundle{
    pub fn import(path:&Path)->Result<Self>{
        bundle_path(path)?;
        if !fs::symlink_metadata(path)?.is_dir(){bail!("bundle must be a real directory");}
        let mut found=std::collections::BTreeSet::new();
        for entry in fs::read_dir(path)?.take(3){
            let entry=entry?;let name=entry.file_name();
            if !matches!(name.to_str(),Some("theme.json"|"wallpaper.png"))||!entry.file_type()?.is_file(){bail!("unexpected bundle entry");}
            found.insert(name);
        }
        if found.len()!=2{bail!("bundle requires theme.json and wallpaper.png");}
        let theme=Theme::import(&path.join("theme.json"))?;
        let wallpaper=load_wallpaper(&path.join("wallpaper.png"))?;
        validate(&theme,&wallpaper)?;
        Ok(Self{theme,wallpaper})
    }
    /// Publish a fresh directory atomically, with no replacement of any
    /// existing destination. Only reviewed raster pixels are re-encoded;
    /// original image metadata and the original path are not exported.
    pub fn export(theme:&Theme,wallpaper:&Raster,path:&Path)->Result<()>{
        bundle_path(path)?;validate(theme,wallpaper)?;
        let parent=path.parent().context("missing bundle parent")?;
        let pending=tempfile::Builder::new().prefix(".atelier-access-").tempdir_in(parent)?;
        let mut manifest=OpenOptions::new().write(true).create_new(true).mode(0o600).open(pending.path().join("theme.json"))?;
        manifest.write_all(theme.json()?.as_bytes())?;manifest.sync_all()?;
        let mut image=OpenOptions::new().write(true).create_new(true).mode(0o600).open(pending.path().join("wallpaper.png"))?;
        image::codecs::png::PngEncoder::new(&mut image).write_image(&wallpaper.rgba,wallpaper.width,wallpaper.height,image::ExtendedColorType::Rgba8)?;
        image.sync_all()?;File::open(pending.path())?.sync_all()?;
        let source=CString::new(pending.path().as_os_str().as_bytes())?;
        let target=CString::new(path.as_os_str().as_bytes())?;
        if unsafe{libc::renameat2(libc::AT_FDCWD,source.as_ptr(),libc::AT_FDCWD,target.as_ptr(),libc::RENAME_NOREPLACE)}!=0{
            return Err(std::io::Error::last_os_error()).context("bundle destination exists or cannot be published");
        }
        File::open(parent)?.sync_all().context("bundle published but parent directory sync failed; inspect before retrying")?;
        Ok(())
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    use crate::access_theme::{self,Backdrop,ImageFit};
    fn fixture()->(Theme,Raster){
        let mut t=access_theme::preset("classic").unwrap();
        t.backdrop=Backdrop::Wallpaper{fit:ImageFit::Cover,dim:30};
        (t,Raster{width:2,height:1,rgba:vec![20,30,40,255,90,80,70,128]})
    }
    #[test]
    fn bundle_round_trip_and_no_overwrite(){
        let root=tempfile::tempdir().unwrap();let path=root.path().join("sample.atelier-access");
        let (theme,raster)=fixture();
        Bundle::export(&theme,&raster,&path).unwrap();
        let read=Bundle::import(&path).unwrap();
        assert_eq!(read.theme,theme);assert_eq!(read.wallpaper.rgba,raster.rgba);
        assert!(Bundle::export(&theme,&raster,&path).is_err());
        fs::write(path.join("unexpected"),b"x").unwrap();assert!(Bundle::import(&path).is_err());
    }
    #[test]
    fn rejects_symlink_and_missing_asset_or_invalid_raster(){
        let root=tempfile::tempdir().unwrap();let path=root.path().join("sample.atelier-access");
        let (theme,mut raster)=fixture();raster.rgba.pop();
        assert!(Bundle::export(&theme,&raster,&path).is_err());
        assert!(!path.exists());
        fs::create_dir(&path).unwrap();fs::write(path.join("theme.json"),theme.json().unwrap()).unwrap();
        assert!(Bundle::import(&path).is_err());
        std::os::unix::fs::symlink(path.join("theme.json"),path.join("wallpaper.png")).unwrap();
        assert!(Bundle::import(&path).is_err());
    }
}
