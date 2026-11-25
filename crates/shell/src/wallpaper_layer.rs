//! Noninteractive native wallpaper client using standard layer shell.
use atelier_config::{Appearance,WallpaperFit,wallpaper};
use gtk::{glib,prelude::*};
use gtk4_layer_shell::{Edge,KeyboardMode,Layer,LayerShell};
use std::{path::PathBuf,time::Duration};

pub fn start(app:&gtk::Application,panel:&gtk::ApplicationWindow,appearance:&Appearance){
    if appearance.wallpaper_path.is_empty(){return;}
    let path=PathBuf::from(&appearance.wallpaper_path);let fit=appearance.wallpaper_fit;
    let (tx,rx)=std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move||{let _=tx.send(wallpaper::decode(&path,4096));});
    let app=app.downgrade();let panel=panel.downgrade();
    glib::timeout_add_local(Duration::from_millis(50),move||{
        let (Some(app),Some(panel))=(app.upgrade(),panel.upgrade())else{return glib::ControlFlow::Break;};
        match rx.try_recv(){
            Ok(Ok(raster))=>{
                let texture=gtk::gdk::MemoryTexture::new(raster.width as i32,raster.height as i32,gtk::gdk::MemoryFormat::R8g8b8a8,&glib::Bytes::from_owned(raster.rgba),raster.width as usize*4);
                let window=gtk::ApplicationWindow::builder().application(&app).title("Atelier wallpaper").decorated(false).build();
                window.init_layer_shell();window.set_namespace(Some("atelier-wallpaper"));window.set_layer(Layer::Background);
                for edge in [Edge::Left,Edge::Right,Edge::Top,Edge::Bottom]{window.set_anchor(edge,true);}
                window.set_exclusive_zone(-1);window.set_keyboard_mode(KeyboardMode::None);
                window.connect_realize(|window|{if let Some(surface)=window.surface(){surface.set_input_region(&gtk::cairo::Region::create());}});
                let picture=gtk::Picture::for_paintable(&texture);
                picture.set_can_shrink(true);picture.set_content_fit(match fit{WallpaperFit::Cover=>gtk::ContentFit::Cover,WallpaperFit::Contain=>gtk::ContentFit::Contain,WallpaperFit::Stretch=>gtk::ContentFit::Fill});
                picture.set_can_target(false);window.set_child(Some(&picture));
                let weak=window.downgrade();panel.connect_destroy(move|_|{if let Some(w)=weak.upgrade(){w.close();}});
                window.present();
                glib::ControlFlow::Break
            }
            Ok(Err(error))=>{eprintln!("Native wallpaper unavailable; retaining solid background: {error:#}");glib::ControlFlow::Break}
            Err(std::sync::mpsc::TryRecvError::Empty)=>glib::ControlFlow::Continue,
            Err(_)=>glib::ControlFlow::Break,
        }
    });
}
