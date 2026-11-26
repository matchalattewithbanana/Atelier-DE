//! Opt-in local cover art. Remote URLs never reach GTK or the decoder.
use crate::mpris::{Details,Identity,Track};
use atelier_config::{wallpaper::{self,Raster},dynamic_palette::{self,Options,Variant,Contrast},design::Color};
use gtk::glib;
use std::{path::PathBuf,sync::Arc};

#[derive(Debug,Clone,PartialEq,Eq)]
struct Key{identity:Identity,track:Option<Track>,title:String,album:String,uri:String}
pub struct Artwork{pub raster:Raster,pub light:Option<Colors>,pub dark:Option<Colors>}
#[derive(Clone,Copy)]
pub struct Colors{pub background:Color,pub foreground:Color,pub accent:Color}
fn colors(seed:[u8;3],light:bool)->Option<Colors>{
    let p=dynamic_palette::generate(seed,Options{variant:Variant::TonalSpot,light,contrast:Contrast::Normal}).ok()?.access;
    Some(Colors{background:Color::parse(&p.panel).ok()?,foreground:Color::parse(&p.text).ok()?,accent:Color::parse(&p.accent).ok()?})
}
fn local_path(uri:&str)->Option<PathBuf>{
    if uri.len()>4096||!uri.starts_with("file:///")||uri.contains(['?','#']){return None;}
    let (path,host)=glib::filename_from_uri(uri).ok()?;
    if host.is_some()||!path.is_absolute()||path.components().any(|c|matches!(c,std::path::Component::ParentDir)){return None;}
    Some(path)
}
fn decode(uri:&str)->Option<Artwork>{
    let path=local_path(uri)?;
    let raster=wallpaper::decode(&path,512).ok()?;
    let seed=wallpaper::seed_from_raster(&raster).ok();
    Some(Artwork{raster,light:seed.and_then(|s|colors(s,true)),dark:seed.and_then(|s|colors(s,false))})
}
/// Cache exactly one result (including failures), scoped to player, track
/// and URL. Dropping consent discards the cache. No disk or network writes.
#[derive(Default)]
pub struct Cache{key:Option<Key>,image:Option<Arc<Artwork>>}
impl Cache{
    pub fn update(&mut self,enabled:bool,details:Option<&Details>)->Option<Arc<Artwork>>{
        let key=details.filter(|_|enabled).and_then(|d|Some(Key{identity:d.identity.clone(),track:d.track.clone(),
            title:d.title.clone(),album:d.album.clone(),uri:d.art_uri.clone()?}));
        if key!=self.key{
            self.image=key.as_ref().and_then(|k|decode(&k.uri)).map(Arc::new);
            self.key=key;
        }
        self.image.clone()
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn accepts_only_local_file_uris(){
        assert_eq!(local_path("file:///tmp/cover%20art.png"),Some(PathBuf::from("/tmp/cover art.png")));
        for uri in ["https://example.org/cover.png","file://remote/tmp/cover.png","file:///tmp/a.png?x=1","file:///tmp/a.png#x","/tmp/a.png","file:///tmp/../cover.png","data:image/png;base64,abc"]{
            assert!(local_path(uri).is_none(),"{uri}");
        }
    }
    #[test]
    fn palettes_keep_wave_and_text_readable(){
        for light in [true,false]{
            let c=colors([100,60,180],light).unwrap();
            assert!(c.background.contrast(c.foreground)>=4.5&&c.background.contrast(c.accent)>=4.5);
        }
    }
}
