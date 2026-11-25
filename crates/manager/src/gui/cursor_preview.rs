//! Local cursor contact sheet. Only decoded pixel data reaches GTK.
use gtk::{gdk,glib,prelude::*};
use super::{background,label};

pub(super) fn build()->gtk::Box{
    let outer=gtk::Box::new(gtk::Orientation::Vertical,12);
    outer.set_margin_top(24);outer.set_margin_bottom(24);outer.set_margin_start(24);outer.set_margin_end(24);
    outer.append(&label("Cursor previews","title-1"));
    outer.append(&label("Inspect installed XCursor images before selecting them. Static first-frame samples only; no theme activation or remote fetch.","dim-label"));
    let theme=gtk::Entry::new();theme.set_placeholder_text(Some("Installed XCursor theme name"));
    let presets=gtk::ComboBoxText::new();
    for name in atelier_config::CURSOR_PRESETS{presets.append(Some(name),name);}
    {let theme=theme.downgrade();presets.connect_changed(move|p|{if let (Some(t),Some(id))=(theme.upgrade(),p.active_id()){t.set_text(&id);}});}
    outer.append(&presets);outer.append(&theme);
    let size=gtk::SpinButton::with_range(8.0,128.0,1.0);size.set_value(24.0);
    outer.append(&label("Nominal pointer size","body"));outer.append(&size);
    let show=gtk::Button::with_label("Preview installed images");outer.append(&show);
    let status=label("Select a theme to preview.","dim-label");outer.append(&status);
    let grid=gtk::Grid::builder().column_spacing(12).row_spacing(12).column_homogeneous(true).build();outer.append(&grid);
    let (theme,size,presets,status,grid)=(theme.downgrade(),size.downgrade(),presets.downgrade(),status.downgrade(),grid.downgrade());
    show.connect_clicked(move|button|{
        let (Some(theme),Some(size),Some(presets),Some(status),Some(grid))=(theme.upgrade(),size.upgrade(),presets.upgrade(),status.upgrade(),grid.upgrade())else{return;};
        let name=theme.text().trim().to_string();let selected_size=size.value_as_int() as u32;
        theme.set_sensitive(false);size.set_sensitive(false);presets.set_sensitive(false);button.set_sensitive(false);
        while let Some(child)=grid.first_child(){grid.remove(&child);}
        status.set_text("Reading local cursor images…");
        let (theme,size,presets,status,grid,button)=(theme.downgrade(),size.downgrade(),presets.downgrade(),status.downgrade(),grid.downgrade(),button.downgrade());
        background(move||crate::cursor_preview::samples(&name,selected_size),move|result|{
            let (Some(theme),Some(size),Some(presets),Some(status),Some(grid),Some(button))=(theme.upgrade(),size.upgrade(),presets.upgrade(),status.upgrade(),grid.upgrade(),button.upgrade())else{return;};
            match result{
                Ok(samples)=>{
                    status.set_text("Local first-frame samples. Missing shapes are not substituted; inheritance may supply them during use.");
                    for (index,sample) in samples.into_iter().enumerate(){
                        let tile=gtk::Box::new(gtk::Orientation::Vertical,6);
                        tile.append(&label(sample.label,"heading"));
                        if let Some(frame)=sample.frame{
                            let bytes=glib::Bytes::from_owned(frame.rgba);
                            let texture=gdk::MemoryTexture::new(frame.width as i32,frame.height as i32,
                                gdk::MemoryFormat::R8g8b8a8Premultiplied,&bytes,frame.width as usize*4);
                            let picture=gtk::Picture::for_paintable(&texture);
                            picture.set_can_shrink(true);picture.set_size_request(80,80);
                            tile.append(&picture);
                            tile.append(&label(&format!("{} × {} · hotspot {},{}",frame.width,frame.height,frame.xhot,frame.yhot),"dim-label"));
                        }else{tile.append(&label("Not found","dim-label"));}
                        grid.attach(&tile,(index%3) as i32,(index/3) as i32,1,1);
                    }
                }
                Err(error)=>status.set_text(&format!("Preview unavailable: {error:#}")),
            }
            theme.set_sensitive(true);size.set_sensitive(true);presets.set_sensitive(true);button.set_sensitive(true);
        });
    });
    outer
}
