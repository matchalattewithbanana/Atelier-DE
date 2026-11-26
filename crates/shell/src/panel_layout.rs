//! Adaptive panel presentation. The enabled controls are never dropped
//! to fit the output; scrolling is the last-resort access path.
use gtk::prelude::*;
use std::cell::Cell;

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum Density{Comfortable,Compact}
fn density(width:i32,workspaces:usize,spacing:u32)->Density{
    // Logical output pixels. External nine-tag strips and unusually wide
    // user spacing need more room. Unknown geometry favors compact mode.
    let threshold=1100+workspaces.saturating_sub(4).min(5) as i32*36+
        spacing.saturating_sub(8).min(16) as i32*8;
    if width>=threshold{Density::Comfortable}else{Density::Compact}
}
pub struct Shortcut{
    pub button:gtk::Button,
    label:gtk::Label,
}
impl Shortcut{
    pub fn new(name:&str,icon:&str)->Self{
        let button=gtk::Button::new();button.add_css_class("atelier-panel-shortcut");
        button.set_tooltip_text(Some(name));
        button.update_property(&[gtk::accessible::Property::Label(name)]);
        let child=gtk::Box::new(gtk::Orientation::Horizontal,6);child.set_halign(gtk::Align::Center);
        let image=gtk::Image::from_icon_name(icon);image.set_pixel_size(16);child.append(&image);
        let label=gtk::Label::new(Some(name));child.append(&label);
        button.set_child(Some(&child));Self{button,label}
    }
    fn compact(&self,value:bool){self.label.set_visible(!value);}
}
pub struct Layout{
    bar:gtk::Box,
    shortcuts:Vec<Shortcut>,
    backend_label:gtk::Label,
    quick:gtk::Widget,
    density:Cell<Option<Density>>,
    workspace_count:usize,
}
impl Layout{
    pub fn new(bar:&gtk::Box,shortcuts:Vec<Shortcut>,backend_label:&gtk::Label,quick:&gtk::Widget,workspace_count:usize)->Self{
        Self{bar:bar.clone(),shortcuts,backend_label:backend_label.clone(),quick:quick.clone(),density:Cell::new(None),workspace_count}
    }
    /// Use monitor geometry rather than the potentially oversized child's
    /// requested width. No reparenting, popup recreation or focus changes.
    pub fn update(&self,panel:&gtk::ApplicationWindow,spacing:u32)->bool{
        let width=panel.surface().and_then(|s|s.display().monitor_at_surface(&s)).map_or(0,|m|m.geometry().width());
        let next=density(width,self.workspace_count,spacing);
        let compact=next==Density::Compact;
        if self.density.replace(Some(next))!=Some(next){
            for shortcut in &self.shortcuts{shortcut.compact(compact);}
            self.backend_label.set_visible(!compact);
            if compact{self.bar.add_css_class("atelier-panel-compact");}else{self.bar.remove_css_class("atelier-panel-compact");}
            if let Some(menu)=self.quick.downcast_ref::<gtk::MenuButton>(){
                if compact{menu.set_icon_name("preferences-system-symbolic");}else{menu.set_label("Quick controls");}
            }else if let Some(button)=self.quick.downcast_ref::<gtk::Button>(){
                if compact{button.set_icon_name("preferences-system-symbolic");}else{button.set_label("Quick controls");}
            }
        }
        compact
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn logical_width_and_workspace_count_select_density(){
        assert_eq!(density(0,4,8),Density::Compact);
        assert_eq!(density(800,4,8),Density::Compact);
        assert_eq!(density(1100,4,8),Density::Comfortable);
        assert_eq!(density(1200,9,8),Density::Compact);
        assert_eq!(density(1440,9,8),Density::Comfortable);
        assert_eq!(density(1200,4,24),Density::Compact);
    }
}
