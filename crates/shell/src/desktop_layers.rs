//! Own auxiliary layer lifetimes independently of the persistent panel.
//! Only structural changes recreate a dock/widget group; color-only changes
//! are handled by the existing CSS provider without losing window state.
use atelier_config::{Appearance,DesktopWidgets};
use gtk::{glib,prelude::*};
use std::{cell::RefCell,rc::Rc,time::Duration};
use crate::{dock::Dock,wallpaper_layer::WallpaperLayer};

#[derive(Debug,Clone,PartialEq,Eq)]
struct DockPreferences{
    enabled:bool,auto_hide:bool,hide_fullscreen:bool,icon_size:u32,pins:Vec<String>,
}
impl From<&Appearance> for DockPreferences{
    fn from(value:&Appearance)->Self{
        Self{enabled:value.dock_enabled,auto_hide:value.dock_auto_hide,hide_fullscreen:value.dock_hide_fullscreen,
            icon_size:value.dock_icon_size,pins:value.dock_pins.clone()}
    }
}
#[derive(Default)]
struct DesktopLayers{
    wallpaper:WallpaperLayer,
    dock:Option<Dock>,
    dock_preferences:Option<DockPreferences>,
    widgets:Option<crate::desktop_widgets::WidgetGroup>,
    widget_preferences:Option<DesktopWidgets>,
}
impl DesktopLayers{
    fn update(&mut self,app:&gtk::Application,prefs:&Appearance,native:bool){
        let dock=DockPreferences::from(prefs);
        if self.dock_preferences.as_ref()!=Some(&dock){
            // Drop the old dock (including the hidden reveal surface) first.
            self.dock.take();
            if dock.enabled{self.dock=Some(crate::dock::build(app,prefs));}
            self.dock_preferences=Some(dock);
        }
        if native{
            self.wallpaper.update(app,prefs);
            if self.widget_preferences.as_ref()!=Some(&prefs.widgets){
                self.widgets.take();
                self.widgets=crate::desktop_widgets::build(app,&prefs.widgets,prefs.button_radius);
                self.widget_preferences=Some(prefs.widgets.clone());
            }
            if let Some(group)=&self.widgets{group.set_radius(prefs.button_radius);}
        }
    }
}
impl Drop for DesktopLayers{
    fn drop(&mut self){
        self.widgets.take();
        // The owned dock and wallpaper destroy their surfaces on drop.
    }
}
pub fn start(app:&gtk::Application,panel:&gtk::ApplicationWindow,live:Rc<RefCell<Appearance>>){
    let native=crate::task_backend::TaskBackend::detect().is_some();
    let mut layers=DesktopLayers::default();layers.update(app,&live.borrow(),native);
    let state=Rc::new(RefCell::new(Some(layers)));
    let weak_app=app.downgrade();
    let tick_state=state.clone();
    let source=glib::timeout_add_local(Duration::from_millis(250),move||{
        let Some(app)=weak_app.upgrade()else{return glib::ControlFlow::Continue;};
        if let Some(layers)=tick_state.borrow_mut().as_mut(){layers.update(&app,&live.borrow(),native);}
        glib::ControlFlow::Continue
    });
    let source=Rc::new(RefCell::new(Some(source)));
    panel.connect_destroy(move|_|{
        if let Some(source)=source.borrow_mut().take(){source.remove();}
        state.borrow_mut().take();
    });
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn dock_rebuild_key_excludes_unrelated_appearance(){
        let mut prefs=Appearance::default();let original=DockPreferences::from(&prefs);
        prefs.accent="#123456".into();prefs.button_radius=20;prefs.wallpaper_path="/tmp/wall.png".into();
        assert_eq!(DockPreferences::from(&prefs),original);
        prefs.dock_pins.push("foot.desktop".into());assert_ne!(DockPreferences::from(&prefs),original);
        let pinned=DockPreferences::from(&prefs);prefs.dock_auto_hide=true;assert_ne!(DockPreferences::from(&prefs),pinned);
    }
}
