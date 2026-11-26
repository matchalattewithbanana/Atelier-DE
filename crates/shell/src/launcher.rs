//! Native GTK launcher: persistent rows, ranked search and accessible list selection.
use gtk::{gdk,glib,prelude::*};
use crate::launcher_catalog::{self,Category};
use std::{cell::RefCell,rc::Rc};

pub struct Browser{pub root:gtk::Box,pub search:gtk::SearchEntry}
fn index(row:&gtk::ListBoxRow)->Option<usize>{row.widget_name().strip_prefix("app-row-")?.parse().ok()}
fn title(text:&str)->gtk::Label{
    let label=gtk::Label::new(Some(text));label.set_xalign(0.0);label.set_ellipsize(gtk::pango::EllipsizeMode::End);label.set_max_width_chars(48);label
}
pub fn browser(window:&gtk::ApplicationWindow,appearance:&atelier_config::Appearance,status:&gtk::Label)->Browser{
    let root=gtk::Box::new(gtk::Orientation::Vertical,8);root.set_vexpand(true);
    let search=gtk::SearchEntry::new();search.set_placeholder_text(Some("Search apps, descriptions and keywords"));search.set_size_request(-1,48);search.add_css_class("atelier-launcher-search");root.append(&search);
    let header=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let categories=gtk::ComboBoxText::new();
    for (_,name) in Category::OPTIONS{categories.append_text(name);}
    categories.set_active(Some(0));categories.set_hexpand(true);header.append(&categories);
    let count=gtk::Label::new(None);count.add_css_class("dim-label");header.append(&count);root.append(&header);
    let list=gtk::ListBox::new();list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(true);list.add_css_class("atelier-launcher-results");
    let entries=Rc::new(launcher_catalog::catalog(&appearance.dock_pins));
    for (i,entry) in entries.iter().enumerate(){
        let row=gtk::ListBoxRow::new();row.set_widget_name(&format!("app-row-{i}"));
        let item=gtk::Box::new(gtk::Orientation::Horizontal,12);item.add_css_class("atelier-launcher-app");
        let icon=entry.app.icon().map(|icon|gtk::Image::from_gicon(&icon)).unwrap_or_else(||gtk::Image::from_icon_name("application-x-executable"));
        icon.set_pixel_size(appearance.launcher_icon_size as i32);item.append(&icon);
        let text=gtk::Box::new(gtk::Orientation::Vertical,3);text.set_hexpand(true);
        let name=title(&entry.name);name.add_css_class("atelier-launcher-app-name");text.append(&name);
        if !entry.description.is_empty(){let desc=title(&entry.description);desc.add_css_class("dim-label");text.append(&desc);}
        item.append(&text);
        if entry.pinned{let pin=gtk::Image::from_icon_name("starred-symbolic");pin.set_tooltip_text(Some("Pinned in dock"));item.append(&pin);}
        row.set_child(Some(&item));row.set_tooltip_text(Some(&format!("{} · {}",entry.name,entry.description)));
        let accessible=if entry.pinned{format!("{} · Pinned · {}",entry.name,entry.description)}else{format!("{} · {}",entry.name,entry.description)};
        row.update_property(&[gtk::accessible::Property::Label(&accessible)]);
        list.append(&row);
    }
    let ranks=Rc::new(RefCell::new(vec![Some(0u32);entries.len()]));
    {let ranks=ranks.clone();list.set_filter_func(move|row|index(row).and_then(|i|ranks.borrow().get(i).copied().flatten()).is_some());}
    {let ranks=ranks.clone();list.set_sort_func(move|a,b|{
        let ranks=ranks.borrow();let a=index(a).unwrap_or(usize::MAX);let b=index(b).unwrap_or(usize::MAX);
        let score=|i|ranks.get(i).copied().flatten();
        match score(b).cmp(&score(a)).then(a.cmp(&b)){
            std::cmp::Ordering::Less=>gtk::Ordering::Smaller,std::cmp::Ordering::Equal=>gtk::Ordering::Equal,std::cmp::Ordering::Greater=>gtk::Ordering::Larger
        }
    });}
    let empty=gtk::Label::new(Some("No matching applications"));empty.set_margin_top(32);empty.set_margin_bottom(32);list.set_placeholder(Some(&empty));
    let refresh:Rc<dyn Fn()>={
        let (search,categories,list,count)=(search.downgrade(),categories.downgrade(),list.downgrade(),count.downgrade());
        let entries=entries.clone();let ranks=ranks.clone();
        Rc::new(move||{
            let (Some(search),Some(categories),Some(list),Some(count))=(search.upgrade(),categories.upgrade(),list.upgrade(),count.upgrade())else{return;};
            let category=categories.active().and_then(|i|Category::OPTIONS.get(i as usize)).map_or(Category::All,|(c,_)|*c);
            let query=search.text();
            let new:Vec<_>=entries.iter().map(|entry|entry.rank(category,&query)).collect();
            let visible=new.iter().filter(|r|r.is_some()).count();*ranks.borrow_mut()=new;
            list.invalidate_filter();list.invalidate_sort();
            count.set_text(&format!("{visible} apps"));
            list.unselect_all();
            if !query.trim().is_empty(){
                if let Some(row)=list.row_at_index(0).filter(|r|index(r).and_then(|i|ranks.borrow().get(i).copied().flatten()).is_some()){list.select_row(Some(&row));}
            }
        })
    };
    {let refresh=refresh.clone();search.connect_search_changed(move |_|refresh());}
    {let refresh=refresh.clone();categories.connect_changed(move |_|refresh());}
    {let window=window.downgrade();let status=status.downgrade();
        list.connect_row_activated(move|_,row|{
            if let Some(entry)=index(row).and_then(|i|entries.get(i)){
                match crate::applications::launch(&entry.app){
                    Ok(())=>{if let Some(window)=window.upgrade(){window.close();}}
                    Err(e)=>{if let Some(status)=status.upgrade(){status.set_text(&format!("Could not launch application: {e}"));}}
                }
            }
        });
    }
    {let list=list.downgrade();let refresh=refresh.clone();
        search.connect_activate(move|entry|{
            // SearchEntry signals can be delayed. Recompute immediately so
            // Enter never launches a result for a previous query.
            refresh();
            if entry.text().trim().is_empty(){return;}
            if let Some(list)=list.upgrade(){if let Some(row)=list.selected_row(){row.activate();}}
        });
    }
    let keys=gtk::EventControllerKey::new();
    {let list=list.downgrade();let refresh=refresh.clone();keys.connect_key_pressed(move|_,key,_,modifiers|{
        if key==gdk::Key::Down&&modifiers.is_empty(){
            refresh();
            if let Some(list)=list.upgrade(){if let Some(row)=list.selected_row().or_else(||{
                // Filtered rows remain in the list; find the first mapped row.
                let mut i=0;while let Some(row)=list.row_at_index(i){if row.is_child_visible(){return Some(row);}i+=1;}None
            }){list.select_row(Some(&row));row.grab_focus();return glib::Propagation::Stop;}}
        }
        glib::Propagation::Proceed
    });}
    search.add_controller(keys);
    let escape=gtk::EventControllerKey::new();let weak=window.downgrade();
    escape.connect_key_pressed(move|_,key,_,_|{
        if key==gdk::Key::Escape{if let Some(window)=weak.upgrade(){window.close();}return glib::Propagation::Stop;}
        glib::Propagation::Proceed
    });window.add_controller(escape);
    let scroll=gtk::ScrolledWindow::builder().vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).child(&list).build();root.append(&scroll);
    refresh();Browser{root,search}
}
