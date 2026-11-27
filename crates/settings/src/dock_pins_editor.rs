//! Ordered dock pin editor. The existing text buffer is the sole draft
//! source; graphical actions and the advanced editor share it.
use gtk::prelude::*;
use std::collections::HashMap;
use super::description;

const MAX_PINS:usize=32;

pub(super) fn ids(buffer:&gtk::TextBuffer)->Vec<String>{
    parse(&buffer.text(&buffer.start_iter(),&buffer.end_iter(),true))
}
fn parse(text:&str)->Vec<String>{
    text.lines().map(str::trim).filter(|s|!s.is_empty()).map(str::to_owned).collect()
}

#[derive(Clone,Copy)]
enum Edit{Up,Down,Remove}
fn edit(pins:&mut Vec<String>,index:usize,action:Edit)->Option<usize>{
    if index>=pins.len(){return None;}
    match action{
        Edit::Up if index>0=>{pins.swap(index,index-1);Some(index-1)}
        Edit::Down if index+1<pins.len()=>{pins.swap(index,index+1);Some(index+1)}
        Edit::Remove=>{pins.remove(index);Some(index.min(pins.len().saturating_sub(1)))}
        _=>None
    }
}
fn append(pins:&mut Vec<String>,id:&str)->Result<(),&'static str>{
    if pins.iter().any(|p|p==id){return Err("Application is already pinned.");}
    if pins.len()>=MAX_PINS{return Err("The dock supports up to 32 pins.");}
    let candidate=atelier_config::Appearance{dock_pins:vec![id.to_owned()],..Default::default()};
    if candidate.validate().is_err(){return Err("Invalid desktop application ID.");}
    pins.push(id.to_owned());Ok(())
}
fn selected(list:&gtk::ListBox)->Option<usize>{
    list.selected_row().and_then(|row|usize::try_from(row.index()).ok())
}
fn select(list:&gtk::ListBox,index:usize){
    if let Some(row)=list.row_at_index(index as i32){list.select_row(Some(&row));}
}

/// Build the complete editor and return its shared buffer widget. A missing
/// app remains a real pin, so reloads cannot silently discard it.
pub(super) fn build(parent:&gtk::Box)->gtk::TextView{
    parent.append(&description("Add installed applications and arrange them from left to right. Changes remain in this draft until saved."));
    let picker=gtk::ComboBoxText::new();picker.set_hexpand(true);
    let mut apps:Vec<_>=gtk::gio::AppInfo::all().into_iter().filter(|app|app.should_show()).collect();
    apps.sort_by_key(|app|(app.display_name().to_lowercase(),app.id().map(|id|id.to_string())));
    let mut names=HashMap::<String,String>::new();
    for app in apps{
        if let Some(id)=app.id(){
            if id.ends_with(".desktop")&&!names.contains_key(id.as_str()){
                let name=app.display_name().to_string();
                picker.append(Some(&id),&format!("{name} ({id})"));
                names.insert(id.to_string(),name);
            }
        }
    }
    let add=gtk::Button::with_label("Add pin");
    let row=gtk::Box::new(gtk::Orientation::Horizontal,8);row.append(&picker);row.append(&add);parent.append(&row);
    if !names.is_empty(){picker.set_active(Some(0));}
    add.set_sensitive(picker.active_id().is_some());
    {let add=add.downgrade();picker.connect_changed(move|p|{if let Some(a)=add.upgrade(){a.set_sensitive(p.active_id().is_some());}});}
    let list=gtk::ListBox::new();list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("boxed-list");
    let empty=description("No applications pinned.");list.set_placeholder(Some(&empty));
    let scroll=gtk::ScrolledWindow::builder().child(&list).min_content_height(120).max_content_height(280).propagate_natural_height(true).build();
    parent.append(&scroll);
    let up=gtk::Button::with_label("Move up");let down=gtk::Button::with_label("Move down");let remove=gtk::Button::with_label("Remove");
    let actions=gtk::Box::new(gtk::Orientation::Horizontal,8);
    for button in [&up,&down,&remove]{button.set_sensitive(false);actions.append(button);}
    parent.append(&actions);
    let status=description("");status.set_selectable(true);parent.append(&status);
    let pins=gtk::TextView::new();pins.set_monospace(true);pins.set_wrap_mode(gtk::WrapMode::WordChar);
    let advanced=gtk::Expander::new(Some("Advanced · Edit exact desktop IDs"));
    advanced.set_child(Some(&gtk::ScrolledWindow::builder().child(&pins).min_content_height(100).build()));parent.append(&advanced);
    let buffer=pins.buffer();
    {
        let (buffer,up,down,remove)=(buffer.downgrade(),up.downgrade(),down.downgrade(),remove.downgrade());
        list.connect_row_selected(move|_,row|{
            let Some(b)=buffer.upgrade()else{return;};
            let count=ids(&b).len().min(MAX_PINS);
            let index=row.and_then(|r|usize::try_from(r.index()).ok()).filter(|i|*i<count);
            if let Some(b)=up.upgrade(){b.set_sensitive(index.is_some_and(|i|i>0));}
            if let Some(b)=down.upgrade(){b.set_sensitive(index.is_some_and(|i|i+1<count));}
            if let Some(b)=remove.upgrade(){b.set_sensitive(index.is_some());}
        });
    }
    {
        let (list,status)=(list.downgrade(),status.downgrade());
        buffer.connect_changed(move|buffer|{
            let (Some(list),Some(status))=(list.upgrade(),status.upgrade())else{return;};
            let old=selected(&list).unwrap_or(0);
            while let Some(row)=list.row_at_index(0){list.remove(&row);}
            let pins=ids(buffer);
            // Only a bounded number of rows is rendered, even for invalid
            // manual drafts. All text is retained for repair and validation.
            for (index,id) in pins.iter().take(MAX_PINS).enumerate(){
                let item=gtk::Box::new(gtk::Orientation::Vertical,3);
                item.set_margin_top(6);item.set_margin_bottom(6);item.set_margin_start(10);item.set_margin_end(10);
                let title=gtk::Label::new(Some(&format!("{}. {}",index+1,names.get(id).map(String::as_str).unwrap_or("Not in application picker"))));
                title.set_xalign(0.0);title.set_wrap(true);item.append(&title);
                item.append(&description(id));list.append(&item);
            }
            status.set_text(&if pins.len()>MAX_PINS{format!("{} pins · Only the first 32 are shown. Repair the advanced draft before saving.",pins.len())}else{format!("{} / 32 pins",pins.len())});
            select(&list,old.min(pins.len().min(MAX_PINS).saturating_sub(1)));
        });
    }
    {
        let (picker,buffer,list,status)=(picker.downgrade(),buffer.downgrade(),list.downgrade(),status.downgrade());
        add.connect_clicked(move|_|{
            let (Some(picker),Some(buffer),Some(list),Some(status))=(picker.upgrade(),buffer.upgrade(),list.upgrade(),status.upgrade())else{return;};
            let Some(id)=picker.active_id()else{return;};let mut pins=ids(&buffer);
            match append(&mut pins,&id){
                Ok(())=>{buffer.set_text(&pins.join("\n"));select(&list,pins.len()-1);}
                Err(message)=>status.set_text(message),
            }
        });
    }
    for (button,action) in [(&up,Edit::Up),(&down,Edit::Down),(&remove,Edit::Remove)]{
        let (buffer,list)=(buffer.downgrade(),list.downgrade());
        button.connect_clicked(move|_|{
            let (Some(buffer),Some(list))=(buffer.upgrade(),list.upgrade())else{return;};
            let Some(index)=selected(&list)else{return;};
            let mut pins=ids(&buffer);
            if let Some(next)=edit(&mut pins,index,action){buffer.set_text(&pins.join("\n"));select(&list,next);}
        });
    }
    status.set_text("0 / 32 pins");
    pins
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn reorder_preserves_unknown_ids_and_boundaries(){
        let mut pins=parse("a.desktop\n missing.desktop\nb.desktop\n");
        assert_eq!(edit(&mut pins,0,Edit::Up),None);
        assert_eq!(edit(&mut pins,2,Edit::Down),None);
        assert_eq!(edit(&mut pins,1,Edit::Up),Some(0));
        assert_eq!(pins,["missing.desktop","a.desktop","b.desktop"]);
        assert_eq!(edit(&mut pins,0,Edit::Down),Some(1));
        assert_eq!(edit(&mut pins,1,Edit::Remove),Some(1));
        assert_eq!(pins,["a.desktop","b.desktop"]);
        assert_eq!(edit(&mut pins,5,Edit::Remove),None);
    }
    #[test]
    fn append_checks_duplicate_invalid_and_capacity(){
        let mut pins=vec!["a.desktop".into()];
        assert!(append(&mut pins,"a.desktop").is_err());
        assert!(append(&mut pins,"../bad.desktop").is_err());
        assert!(append(&mut pins,"A.desktop").is_ok());
        assert_eq!(pins,["a.desktop","A.desktop"]);
        let mut full=(0..32).map(|i|format!("app{i}.desktop")).collect();
        assert!(append(&mut full,"extra.desktop").is_err());
        assert_eq!(full.len(),32);
    }
}
