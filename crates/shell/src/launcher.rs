//! Native GTK launcher: shared ranked model, persistent list and grid widgets.
use gtk::{gdk,glib,prelude::*};
use crate::launcher_catalog::{self,Category,Entry};
use std::{cell::RefCell,rc::Rc,sync::mpsc,time::Duration};
use atelier_config::{Appearance,AppearanceReview,appearance_path};

pub struct Browser{pub root:gtk::Box,pub search:gtk::SearchEntry}
fn index(widget:&impl IsA<gtk::Widget>)->Option<usize>{widget.widget_name().strip_prefix("app-row-")?.parse().ok()}
type Ranks=Rc<RefCell<Vec<Option<u32>>>>;
fn matches(ranks:&Ranks,widget:&impl IsA<gtk::Widget>)->bool{
    index(widget).and_then(|i|ranks.borrow().get(i).copied().flatten()).is_some()
}
fn order(ranks:&Ranks,a:&impl IsA<gtk::Widget>,b:&impl IsA<gtk::Widget>)->gtk::Ordering{
    let ranks=ranks.borrow();let a=index(a).unwrap_or(usize::MAX);let b=index(b).unwrap_or(usize::MAX);
    let score=|i:usize|ranks.get(i).copied().flatten();
    match score(b).cmp(&score(a)).then(a.cmp(&b)){
        std::cmp::Ordering::Less=>gtk::Ordering::Smaller,std::cmp::Ordering::Equal=>gtk::Ordering::Equal,std::cmp::Ordering::Greater=>gtk::Ordering::Larger
    }
}
fn title(text:&str)->gtk::Label{
    let label=gtk::Label::new(Some(text));label.set_xalign(0.0);label.set_ellipsize(gtk::pango::EllipsizeMode::End);label.set_max_width_chars(48);label
}
fn app_icon(entry:&Entry,size:u32)->gtk::Image{
    let icon=entry.app.icon().map(|icon|gtk::Image::from_gicon(&icon)).unwrap_or_else(||gtk::Image::from_icon_name("application-x-executable"));
    icon.set_pixel_size(size as i32);icon
}
fn pin_icon()->gtk::Image{
    let icon=gtk::Image::from_icon_name("starred-symbolic");icon.set_tooltip_text(Some("Pinned in dock"));icon
}
fn identify(widget:&impl IsA<gtk::Widget>,entry:&Entry,i:usize){
    widget.set_widget_name(&format!("app-row-{i}"));
    let accessible=if entry.pinned.get(){format!("{} · Pinned · {}",entry.name,entry.description)}else{format!("{} · {}",entry.name,entry.description)};
    widget.set_tooltip_text(Some(&accessible));
    widget.as_ref().update_property(&[gtk::accessible::Property::Label(&accessible)]);
}
struct PinMark{widget:glib::WeakRef<gtk::Widget>,icon:glib::WeakRef<gtk::Image>}
impl PinMark{
    fn new(widget:&impl IsA<gtk::Widget>,icon:&gtk::Image)->Self{
        Self{widget:widget.as_ref().downgrade(),icon:icon.downgrade()}
    }
    fn update(&self,entry:&Entry,i:usize){
        if let Some(widget)=self.widget.upgrade(){identify(&widget,entry,i);}
        if let Some(icon)=self.icon.upgrade(){icon.set_visible(entry.pinned.get());}
    }
}
fn list_row(entry:&Entry,i:usize,size:u32)->(gtk::ListBoxRow,PinMark){
    let row=gtk::ListBoxRow::new();identify(&row,entry,i);
    let item=gtk::Box::new(gtk::Orientation::Horizontal,12);item.add_css_class("atelier-launcher-app");
    item.append(&app_icon(entry,size));
    let text=gtk::Box::new(gtk::Orientation::Vertical,3);text.set_hexpand(true);
    let name=title(&entry.name);name.add_css_class("atelier-launcher-app-name");text.append(&name);
    if !entry.description.is_empty(){let desc=title(&entry.description);desc.add_css_class("dim-label");text.append(&desc);}
    item.append(&text);if entry.pinned{item.append(&pin_icon());}
    row.set_child(Some(&item));row
}
fn grid_tile(entry:&Entry,i:usize,size:u32)->(gtk::FlowBoxChild,PinMark){
    let child=gtk::FlowBoxChild::new();identify(&child,entry,i);
    let tile=gtk::Box::new(gtk::Orientation::Vertical,8);tile.add_css_class("atelier-launcher-tile");
    tile.set_size_request(96,96);tile.set_halign(gtk::Align::Fill);
    let art=gtk::Overlay::new();art.set_halign(gtk::Align::Center);
    let icon=app_icon(entry,size);art.set_child(Some(&icon));
    if entry.pinned{
        let pin=pin_icon();pin.set_halign(gtk::Align::End);pin.set_valign(gtk::Align::Start);art.add_overlay(&pin);
    }
    tile.append(&art);
    let name=title(&entry.name);name.set_xalign(0.5);name.set_max_width_chars(14);name.set_wrap(true);name.set_lines(2);name.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    tile.append(&name);child.set_child(Some(&tile));let mark=PinMark::new(&child,&pin);(child,mark)
}
fn first_list(list:&gtk::ListBox,ranks:&Ranks)->Option<gtk::ListBoxRow>{
    let mut i=0;while let Some(row)=list.row_at_index(i){if matches(ranks,&row){return Some(row);}i+=1;}None
}
fn first_grid(grid:&gtk::FlowBox,ranks:&Ranks)->Option<gtk::FlowBoxChild>{
    let mut i=0;while let Some(child)=grid.child_at_index(i){if matches(ranks,&child){return Some(child);}i+=1;}None
}
pub fn browser(window:&gtk::ApplicationWindow,appearance:&atelier_config::Appearance,status:&gtk::Label)->Browser{
    let root=gtk::Box::new(gtk::Orientation::Vertical,8);root.set_vexpand(true);
    let search=gtk::SearchEntry::new();search.set_placeholder_text(Some("Search apps, descriptions and keywords"));search.set_size_request(-1,48);search.add_css_class("atelier-launcher-search");root.append(&search);
    let header=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let categories=gtk::ComboBoxText::new();for (_,name) in Category::OPTIONS{categories.append_text(name);}
    categories.set_active(Some(0));categories.set_hexpand(true);header.append(&categories);
    let count=gtk::Label::new(None);count.add_css_class("dim-label");header.append(&count);
    let switch=gtk::ToggleButton::with_label("Grid");switch.set_tooltip_text(Some("Switch between list and application grid"));header.append(&switch);root.append(&header);
    let edit=gtk::ToggleButton::with_label("Edit dock pins");
    edit.set_tooltip_text(Some("Switch application activation to pin editing; does not launch apps"));root.append(&edit);
    let hint=gtk::Label::new(Some("Pin editing: select an app to add or remove its dock pin. Turn off editing to launch apps."));
    hint.set_wrap(true);hint.set_xalign(0.0);hint.set_visible(false);root.append(&hint);
    {let hint=hint.downgrade();edit.connect_toggled(move|button|{
        if let Some(hint)=hint.upgrade(){hint.set_visible(button.is_active());}
        button.set_label(if button.is_active(){"Done editing dock pins"}else{"Edit dock pins"});
    });}
    let list=gtk::ListBox::new();list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(true);list.add_css_class("atelier-launcher-results");
    let grid=gtk::FlowBox::new();grid.set_selection_mode(gtk::SelectionMode::Single);
    grid.set_activate_on_single_click(true);grid.set_homogeneous(true);
    grid.set_min_children_per_line(1);grid.set_max_children_per_line(6);
    grid.set_column_spacing(6);grid.set_row_spacing(6);grid.set_valign(gtk::Align::Start);grid.add_css_class("atelier-launcher-grid");
    let entries=Rc::new(launcher_catalog::catalog(&appearance.dock_pins));
    let mut marks=Vec::new();
    for (i,entry) in entries.iter().enumerate(){
        let (row,list_mark)=list_row(entry,i,appearance.launcher_icon_size);
        let (tile,grid_mark)=grid_tile(entry,i,appearance.launcher_icon_size);
        list.append(&row);grid.insert(&tile,-1);marks.push((list_mark,grid_mark));
    }
    let marks=Rc::new(marks);
    let pins=Rc::new(RefCell::new(appearance.dock_pins.clone()));
    let ranks=Rc::new(RefCell::new(vec![Some(0u32);entries.len()]));
    {let ranks=ranks.clone();list.set_filter_func(move|row|matches(&ranks,row));}
    {let ranks=ranks.clone();grid.set_filter_func(move|child|matches(&ranks,child));}
    {let ranks=ranks.clone();list.set_sort_func(move|a,b|order(&ranks,a,b));}
    {let ranks=ranks.clone();grid.set_sort_func(move|a,b|order(&ranks,a,b));}
    let empty=gtk::Label::new(Some("No matching applications"));empty.set_margin_top(32);empty.set_margin_bottom(32);
    let stack=gtk::Stack::new();stack.set_vexpand(true);stack.set_hhomogeneous(false);stack.set_vhomogeneous(false);
    let list_scroll=gtk::ScrolledWindow::builder().vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).child(&list).build();
    let grid_scroll=gtk::ScrolledWindow::builder().vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).child(&grid).build();
    stack.add_named(&list_scroll,Some("list"));stack.add_named(&grid_scroll,Some("grid"));stack.add_named(&empty,Some("empty"));root.append(&stack);
    let refresh:Rc<dyn Fn()> = {
        let (search,categories,list,grid,count,switch,stack)=(search.downgrade(),categories.downgrade(),list.downgrade(),grid.downgrade(),count.downgrade(),switch.downgrade(),stack.downgrade());
        let entries=entries.clone();let ranks=ranks.clone();
        Rc::new(move||{
            let (Some(search),Some(categories),Some(list),Some(grid),Some(count),Some(switch),Some(stack))=(search.upgrade(),categories.upgrade(),list.upgrade(),grid.upgrade(),count.upgrade(),switch.upgrade(),stack.upgrade())else{return;};
            let category=categories.active().and_then(|i|Category::OPTIONS.get(i as usize)).map_or(Category::All,|(c,_)|*c);
            let query=search.text();
            let new:Vec<_>=entries.iter().map(|entry|entry.rank(category,&query)).collect();
            let visible=new.iter().filter(|r|r.is_some()).count();*ranks.borrow_mut()=new;
            list.invalidate_filter();list.invalidate_sort();grid.invalidate_filter();grid.invalidate_sort();
            count.set_text(&format!("{visible} apps"));
            stack.set_visible_child_name(if visible==0{"empty"}else if switch.is_active(){"grid"}else{"list"});
            list.unselect_all();grid.unselect_all();
            if !query.trim().is_empty(){
                if let Some(row)=first_list(&list,&ranks){list.select_row(Some(&row));}
                if let Some(child)=first_grid(&grid,&ranks){grid.select_child(&child);}
            }
        })
    };
    {let refresh=refresh.clone();search.connect_changed(move |_|refresh());}
    {let refresh=refresh.clone();categories.connect_changed(move |_|refresh());}
    {let refresh=refresh.clone();switch.connect_toggled(move|button|{button.set_label(if button.is_active(){"List"}else{"Grid"});refresh();});}
    let launch:Rc<dyn Fn(usize)>={
        let (window,status,root,edit)=(window.downgrade(),status.downgrade(),root.downgrade(),edit.downgrade());
        let ranks=ranks.clone();let refresh=refresh.clone();
        Rc::new(move|i|{
            if !ranks.borrow().get(i).is_some_and(Option::is_some){return;}
            let Some(entry)=entries.get(i)else{return;};
            let (Some(root),Some(edit),Some(status))=(root.upgrade(),edit.upgrade(),status.upgrade())else{return;};
            if !root.is_sensitive(){return;}
            if !edit.is_active(){
                match crate::applications::launch(&entry.app){
                    Ok(())=>{if let Some(window)=window.upgrade(){window.close();}}
                    Err(e)=>status.set_text(&format!("Could not launch application: {e}")),
                }
                return;
            }
            let Some(id)=entry.app.id().map(|s|s.to_string())else{status.set_text("This application has no desktop entry ID and cannot be pinned.");return;};
            let expected=pins.borrow().clone();let desired=!entry.pinned.get();
            root.set_sensitive(false);status.set_text("Saving dock pins…");
            let (send,receive)=mpsc::sync_channel(1);
            std::thread::spawn(move||{let _=send.send(save_pin(&id,desired,&expected));});
            let (root,status)=(root.downgrade(),status.downgrade());
            let (entries,marks,pins,refresh)=(entries.clone(),marks.clone(),pins.clone(),refresh.clone());
            glib::timeout_add_local(Duration::from_millis(50),move||{
                let (Some(root),Some(status))=(root.upgrade(),status.upgrade())else{return glib::ControlFlow::Break;};
                match receive.try_recv(){
                    Ok((confirmed,message))=>{
                        if let Some(value)=confirmed{
                            *pins.borrow_mut()=value.dock_pins;
                            let current=pins.borrow();
                            for (i,entry) in entries.iter().enumerate(){
                                entry.pinned.set(entry.app.id().is_some_and(|id|current.iter().any(|pin|pin==id.as_str())));
                                marks[i].0.update(entry,i);marks[i].1.update(entry,i);
                            }
                            drop(current);refresh();
                        }
                        status.set_text(&message);root.set_sensitive(true);glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty)=>glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected)=>{
                        root.set_sensitive(true);status.set_text("Pin save worker stopped; reopen the launcher to verify settings.");glib::ControlFlow::Break
                    }
                }
            });
        })
    };
    {let launch=launch.clone();list.connect_row_activated(move|_,row|{if let Some(i)=index(row){launch(i);}});}
    {let launch=launch.clone();grid.connect_child_activated(move|_,child|{if let Some(i)=index(child){launch(i);}});}
    {let (list,grid,switch,edit)=(list.downgrade(),grid.downgrade(),switch.downgrade(),edit.downgrade());let refresh=refresh.clone();let launch=launch.clone();
        search.connect_activate(move|entry|{
            refresh();if entry.text().trim().is_empty()||edit.upgrade().is_none_or(|e|e.is_active()){return;}
            let Some(switch)=switch.upgrade()else{return;};
            let selected=if switch.is_active(){grid.upgrade().and_then(|g|g.selected_children().first().and_then(|child|index(child)))}
                else{list.upgrade().and_then(|l|l.selected_row().as_ref().and_then(|row|index(row)))};
            if let Some(i)=selected{launch(i);}
        });
    }
    let keys=gtk::EventControllerKey::new();
    {let (list,grid,switch)=(list.downgrade(),grid.downgrade(),switch.downgrade());let ranks=ranks.clone();keys.connect_key_pressed(move|_,key,_,modifiers|{
        if key!=gdk::Key::Down||!modifiers.is_empty(){return glib::Propagation::Proceed;}
        if let Some(switch)=switch.upgrade(){
            if switch.is_active(){
                if let Some(grid)=grid.upgrade(){if let Some(child)=first_grid(&grid,&ranks){grid.select_child(&child);child.grab_focus();return glib::Propagation::Stop;}}
            }else if let Some(list)=list.upgrade(){if let Some(row)=first_list(&list,&ranks){list.select_row(Some(&row));row.grab_focus();return glib::Propagation::Stop;}}
        }
        glib::Propagation::Proceed
    });}
    search.add_controller(keys);
    let escape=gtk::EventControllerKey::new();let weak=window.downgrade();
    escape.connect_key_pressed(move|_,key,_,_|{
        if key==gdk::Key::Escape{if let Some(window)=weak.upgrade(){window.close();}return glib::Propagation::Stop;}
        glib::Propagation::Proceed
    });window.add_controller(escape);
    refresh();Browser{root,search}
}

/// Work off the GTK thread: use the latest reviewed profile but reject a
/// changed pin list. Unrelated appearance edits are preserved by the normal
/// reviewed save and its stale-byte check. Never enable the dock implicitly.
fn save_pin(id:&str,desired:bool,expected:&[String])->(Option<Appearance>,String){
    let path=match appearance_path(){Ok(path)=>path,Err(e)=>return (None,format!("Cannot locate appearance settings: {e}"))};
    let mut review=match AppearanceReview::load(&path){Ok(review)=>review,Err(e)=>return (None,format!("Cannot read appearance settings: {e}"))};
    if review.value.dock_pins!=expected{
        return (Some(review.value),"Dock pins changed elsewhere. Updated the launcher; select the application again to confirm.".into());
    }
    match review.set_dock_pin(id,desired){
        Ok(_)=>{
            let message=format!("{}{}",
                if desired{"Application pinned."}else{"Application unpinned."},
                if review.value.dock_enabled{""}else{" Dock is disabled; enable it in Settings to see pins."});
            (Some(review.value),message)
        }
        Err(e)=>{
            let confirmed=AppearanceReview::load(&path).ok().map(|r|r.value);
            (confirmed,format!("Pin edit not confirmed: {e}. Verify the displayed pins before retrying."))
        }
    }
}
