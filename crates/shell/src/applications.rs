//! Shared installed-application discovery and desktop-entry launch.
use gtk::{gio,glib,prelude::*};

pub fn installed()->Vec<gio::AppInfo>{
    let mut apps:Vec<_>=gio::AppInfo::all().into_iter().filter(|app|app.should_show()).collect();
    apps.sort_by_key(|app|app.display_name().to_lowercase());
    apps
}
pub fn launch(app:&gio::AppInfo)->Result<(),glib::Error>{
    let context=gtk::gdk::Display::default().map(|display|display.app_launch_context());
    app.launch(&[],context.as_ref())
}
