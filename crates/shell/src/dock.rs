//! Optional launch-only dock. No compositor-specific window enumeration.
use atelier_config::{Appearance,component};
use gtk::prelude::*;
use gtk4_layer_shell::{Edge,KeyboardMode,Layer,LayerShell};
use crate::applications;

pub fn build(app:&gtk::Application,appearance:&Appearance){
    let window=gtk::ApplicationWindow::builder().application(app).title("Atelier dock").decorated(false).resizable(false).build();
    window.init_layer_shell();window.set_namespace(Some("atelier-dock"));window.set_layer(Layer::Top);
    window.set_anchor(Edge::Bottom,true);window.set_keyboard_mode(KeyboardMode::None);
    // A bottom-only anchor centers the surface. Reserve space rather than
    // silently covering maximized windows. No auto-hide/fullscreen inference.
    window.auto_exclusive_zone_enable();
    let row=gtk::Box::new(gtk::Orientation::Horizontal,6);row.add_css_class("atelier-dock");
    let launcher=gtk::Button::with_label("Apps");launcher.set_tooltip_text(Some("Open application launcher"));
    let app_ref=app.downgrade();
    launcher.connect_clicked(move |_|{if let Some(app)=app_ref.upgrade(){crate::build_shell(&app);}});
    row.append(&launcher);
    let apps=applications::installed();
    for id in &appearance.dock_pins{
        let Some(info)=apps.iter().find(|info|info.id().as_deref()==Some(id.as_str())).cloned()else{
            let missing=gtk::Button::with_label("?");
            missing.set_tooltip_text(Some(&format!("Unavailable desktop entry: {id}")));
            missing.set_sensitive(false);row.append(&missing);continue;
        };
        let name=info.display_name();
        let button=gtk::Button::new();button.set_tooltip_text(Some(&name));
        let icon=info.icon().map(|icon|gtk::Image::from_gicon(&icon)).unwrap_or_else(||gtk::Image::from_icon_name("application-x-executable"));
        icon.set_pixel_size(appearance.dock_icon_size as i32);
        button.set_child(Some(&icon));
        button.update_property(&[gtk::accessible::Property::Label(&name)]);
        button.connect_clicked(move |button|{
            match applications::launch(&info){
                Ok(())=>button.set_tooltip_text(Some(&info.display_name())),
                Err(error)=>{eprintln!("Dock launch failed: {error}");button.set_tooltip_text(Some(&format!("Launch failed: {error}")));}
            }
        });
        row.append(&button);
    }
    let settings=gtk::Button::with_label("Settings");settings.set_tooltip_text(Some("Configure dock appearance"));
    settings.connect_clicked(move |_|{
        match component("atelier-settings").args(["--page","shell-appearance"]).spawn(){
            Ok(mut child)=>{std::thread::spawn(move||{let _=child.wait();});}
            Err(error)=>eprintln!("Cannot open dock settings: {error}"),
        }
    });row.append(&settings);
    let scroll=gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Automatic).vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(true).propagate_natural_height(true).max_content_width(640).child(&row).build();
    window.set_child(Some(&scroll));window.present();
}
