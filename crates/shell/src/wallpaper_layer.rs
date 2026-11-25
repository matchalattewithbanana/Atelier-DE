//! A single owned wallpaper surface and at most one decode worker.
//! Keep the last successful image until a valid replacement is ready.
use atelier_config::{Appearance,WallpaperFit,WallpaperSlideshow,wallpaper};
use gtk::{glib,prelude::*};
use gtk4_layer_shell::{Edge,KeyboardMode,Layer,LayerShell};
use std::{path::PathBuf,sync::mpsc,time::{Duration,Instant}};

/// Session-local clock; configuration edits restart at the first image.
#[derive(Default)]
struct Rotation{
    playlist:Option<WallpaperSlideshow>,
    index:usize,
    since:Option<Instant>,
}
impl Rotation{
    fn target(&mut self,prefs:&Appearance,now:Instant,busy:bool)->String{
        if self.playlist!=prefs.wallpaper_slideshow{
            self.playlist=prefs.wallpaper_slideshow.clone();self.index=0;self.since=Some(now);
        }
        let Some(playlist)=self.playlist.as_ref()else{return prefs.wallpaper_path.clone();};
        if playlist.images.is_empty(){return prefs.wallpaper_path.clone();}
        let since=self.since.get_or_insert(now);
        if !busy&&now.saturating_duration_since(*since)>=Duration::from_secs(u64::from(playlist.interval_minutes)*60){
            self.index=(self.index+1)%playlist.images.len();*since=now;
        }
        playlist.images[self.index].clone()
    }
}
type DecodeResult=anyhow::Result<wallpaper::Raster>;
#[derive(Default)]
pub struct WallpaperLayer{
    rotation:Rotation,
    surface:Option<(gtk::ApplicationWindow,gtk::Picture)>,
    displayed:Option<String>,
    attempted:Option<String>,
    pending:Option<(String,mpsc::Receiver<DecodeResult>)>,
}
impl WallpaperLayer{
    pub fn update(&mut self,app:&gtk::Application,prefs:&Appearance){
        let path=self.rotation.target(prefs,Instant::now(),self.pending.is_some());
        let path=path.as_str();let fit=prefs.wallpaper_fit;
        if path.is_empty(){
            self.clear_surface();self.attempted=None;
        }
        // Drain the single worker before considering another request, even
        // when the user has cleared or changed the path during decoding.
        if let Some((requested,receiver))=self.pending.take(){
            match receiver.try_recv(){
                Ok(result) if requested==path&&!path.is_empty()=>{
                    match result{
                        Ok(raster)=>{self.show(app,raster,fit);self.displayed=Some(requested);}
                        Err(error)=>eprintln!("Native wallpaper rejected; retaining previous background: {error:#}"),
                    }
                }
                Ok(_)=>{},
                Err(mpsc::TryRecvError::Empty)=>self.pending=Some((requested,receiver)),
                Err(mpsc::TryRecvError::Disconnected)=>eprintln!("Native wallpaper worker stopped; retaining previous background"),
            }
        }
        if let Some((_,picture))=&self.surface{
            let fit=content_fit(fit);
            if picture.content_fit()!=fit{picture.set_content_fit(fit);}
        }
        if path.is_empty()||self.pending.is_some(){return;}
        if self.displayed.as_deref()==Some(path){
            self.attempted=Some(path.into());return;
        }
        if self.attempted.as_deref()==Some(path){return;}
        self.attempted=Some(path.into());
        let (tx,rx)=mpsc::sync_channel(1);let source=PathBuf::from(path);
        std::thread::spawn(move||{let _=tx.send(wallpaper::decode(&source,4096));});
        self.pending=Some((path.into(),rx));
    }
    fn show(&mut self,app:&gtk::Application,raster:wallpaper::Raster,fit:WallpaperFit){
        let texture=gtk::gdk::MemoryTexture::new(raster.width as i32,raster.height as i32,gtk::gdk::MemoryFormat::R8g8b8a8,&glib::Bytes::from_owned(raster.rgba),raster.width as usize*4);
        if let Some((_,picture))=&self.surface{
            picture.set_paintable(Some(&texture));picture.set_content_fit(content_fit(fit));return;
        }
        let window=gtk::ApplicationWindow::builder().application(app).title("Atelier wallpaper").decorated(false).build();
        window.init_layer_shell();window.set_namespace(Some("atelier-wallpaper"));window.set_layer(Layer::Background);
        for edge in [Edge::Left,Edge::Right,Edge::Top,Edge::Bottom]{window.set_anchor(edge,true);}
        window.set_exclusive_zone(-1);window.set_keyboard_mode(KeyboardMode::None);
        window.connect_realize(|window|{if let Some(surface)=window.surface(){surface.set_input_region(&gtk::cairo::Region::create());}});
        let picture=gtk::Picture::for_paintable(&texture);
        picture.set_can_shrink(true);picture.set_content_fit(content_fit(fit));picture.set_can_target(false);
        window.set_child(Some(&picture));window.present();self.surface=Some((window,picture));
    }
    fn clear_surface(&mut self){
        if let Some((window,_))=self.surface.take(){window.destroy();}
        self.displayed=None;
    }
}
impl Drop for WallpaperLayer{fn drop(&mut self){self.clear_surface();}}
fn content_fit(fit:WallpaperFit)->gtk::ContentFit{
    match fit{WallpaperFit::Cover=>gtk::ContentFit::Cover,WallpaperFit::Contain=>gtk::ContentFit::Contain,WallpaperFit::Stretch=>gtk::ContentFit::Fill}
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn rotation_wraps_defers_while_busy_and_resets_on_edit(){
        let mut prefs=Appearance::default();prefs.wallpaper_path="/static.png".into();
        let mut r=Rotation::default();let start=Instant::now();
        assert_eq!(r.target(&prefs,start,false),"/static.png");
        prefs.wallpaper_slideshow=Some(WallpaperSlideshow{images:vec!["/a.png".into(),"/b.png".into()],interval_minutes:1});
        assert_eq!(r.target(&prefs,start,false),"/a.png");
        assert_eq!(r.target(&prefs,start+Duration::from_secs(60),true),"/a.png");
        assert_eq!(r.target(&prefs,start+Duration::from_secs(61),false),"/b.png");
        assert_eq!(r.target(&prefs,start+Duration::from_secs(121),false),"/a.png");
        prefs.wallpaper_slideshow.as_mut().unwrap().images.reverse();
        assert_eq!(r.target(&prefs,start+Duration::from_secs(122),false),"/b.png");
        prefs.wallpaper_slideshow=None;assert_eq!(r.target(&prefs,start+Duration::from_secs(123),false),"/static.png");
    }
}
