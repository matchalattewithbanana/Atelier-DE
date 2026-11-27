//! Independent native appearance draft and previous-version recovery.
use atelier_config::{appearance_path,Appearance,AppearanceReview,PanelDensity};
use gtk::prelude::*;
use std::{cell::RefCell,rc::Rc};
use super::description;

#[derive(Clone)]
struct Fields{
    cursor_theme:gtk::Entry,cursor_size:gtk::SpinButton,
    accent:gtk::Entry,height:gtk::SpinButton,spacing:gtk::SpinButton,radius:gtk::SpinButton,icons:gtk::SpinButton,
    toggles:Vec<gtk::CheckButton>,density:gtk::ComboBoxText,grid:gtk::CheckButton,
    dock:gtk::CheckButton,auto_hide:gtk::CheckButton,hide_fullscreen:gtk::CheckButton,dock_icons:gtk::SpinButton,pins:gtk::TextView,
}
impl Fields{
    fn populate(&self,p:&Appearance){
        self.accent.set_text(&p.accent);
        self.density.set_active_id(Some(match p.panel_density{PanelDensity::Automatic=>"auto",PanelDensity::Comfortable=>"comfortable",PanelDensity::Compact=>"compact"}));
        self.grid.set_active(p.launcher_grid_default);
        self.cursor_theme.set_text(&p.cursor.theme);self.cursor_size.set_value(p.cursor.size as f64);
        self.dock.set_active(p.dock_enabled);self.auto_hide.set_active(p.dock_auto_hide);self.hide_fullscreen.set_active(p.dock_hide_fullscreen);self.dock_icons.set_value(p.dock_icon_size as f64);self.pins.buffer().set_text(&p.dock_pins.join("\n"));
        for (field,value) in [(&self.height,p.panel_height),(&self.spacing,p.panel_spacing),(&self.radius,p.button_radius),(&self.icons,p.launcher_icon_size)]{field.set_value(value as f64);}
        for (field,value) in self.toggles.iter().zip([p.show_date,p.show_battery,p.show_media,p.show_notifications,p.show_audio,p.show_terminal]){field.set_active(value);}
    }
    fn collect(&self,baseline:&Appearance)->Appearance{
        let buffer=self.pins.buffer();let pins=buffer.text(&buffer.start_iter(),&buffer.end_iter(),true);
        Appearance{cursor:atelier_config::CursorPreferences{theme:self.cursor_theme.text().trim().to_owned(),size:self.cursor_size.value_as_int() as u32},dock_enabled:self.dock.is_active(),dock_auto_hide:self.auto_hide.is_active(),dock_hide_fullscreen:self.hide_fullscreen.is_active(),dock_icon_size:self.dock_icons.value_as_int() as u32,
            dock_pins:pins.lines().map(str::trim).filter(|s|!s.is_empty()).map(str::to_owned).collect(),accent:self.accent.text().to_string(),panel_height:self.height.value_as_int() as u32,panel_spacing:self.spacing.value_as_int() as u32,
            button_radius:self.radius.value_as_int() as u32,launcher_icon_size:self.icons.value_as_int() as u32,
            panel_density:match self.density.active_id().as_deref(){Some("compact")=>PanelDensity::Compact,Some("comfortable")=>PanelDensity::Comfortable,_=>PanelDensity::Automatic},launcher_grid_default:self.grid.is_active(),
            show_date:self.toggles[0].is_active(),show_battery:self.toggles[1].is_active(),show_media:self.toggles[2].is_active(),
            show_notifications:self.toggles[3].is_active(),show_audio:self.toggles[4].is_active(),show_terminal:self.toggles[5].is_active(),..baseline.clone()}
    }
}
fn spin(parent:&gtk::Box,title:&str,min:f64,max:f64)->gtk::SpinButton{
    let row=gtk::Box::new(gtk::Orientation::Horizontal,12);
    let label=description(title);label.set_hexpand(true);row.append(&label);
    let spin=gtk::SpinButton::with_range(min,max,1.0);row.append(&spin);parent.append(&row);spin
}
pub fn build(parent:&gtk::Box){
    let path=match appearance_path(){Ok(p)=>p,Err(e)=>{parent.append(&description(&format!("{e:#}")));return;}};
    parent.append(&super::heading("Shell preview"));
    parent.append(&description("Draft illustration · No live changes. Sample geometry and colors, not a screenshot. Saved theme palette is used when present; otherwise a sample dark palette."));
    let preview=gtk::DrawingArea::new();preview.set_content_height(270);preview.set_hexpand(true);parent.append(&preview);
    let sample=gtk::ComboBoxText::new();
    for (id,name) in [("wide","1440 px · 4 workspaces"),("narrow","800 px · 4 workspaces"),("external","1200 px · 9 workspaces")]{sample.append(Some(id),name);}
    sample.set_active_id(Some("wide"));parent.append(&sample);
    let simulate_reveal=gtk::CheckButton::with_label("Simulate dock reveal");parent.append(&simulate_reveal);
    let simulate_fullscreen=gtk::CheckButton::with_label("Simulate native fullscreen for dock");parent.append(&simulate_fullscreen);
    parent.append(&description("Dock simulation only: auto-hide shows the bottom reveal strip until revealed. Native fullscreen may hide both. The panel and launcher illustration remains visible for comparison."));
    parent.append(&super::heading("Colors and pointer"));
    let accent=gtk::Entry::new();accent.set_placeholder_text(Some("#RRGGBB or blank for GTK default"));parent.append(&description("Accent for highlighted native shell buttons"));parent.append(&accent);
    parent.append(&super::heading("Pointer theme"));
    parent.append(&description("Select an installed Bibata or GoogleDot variant, enter another installed XCursor theme, or leave blank to inherit the environment. Restart the native session after saving. Presets do not install files."));
    let cursor_theme=gtk::Entry::new();cursor_theme.set_placeholder_text(Some("System default (inherit)"));
    let presets=gtk::ComboBoxText::new();
    presets.append(Some(""),"System default (inherit)");
    for theme in atelier_config::CURSOR_PRESETS{presets.append(Some(theme),theme);}
    {let entry=cursor_theme.downgrade();presets.connect_changed(move|picker|{
        if let (Some(entry),Some(id))=(entry.upgrade(),picker.active_id()){entry.set_text(&id);}
    });}
    parent.append(&presets);parent.append(&cursor_theme);
    let cursor_size=spin(parent,"Pointer size (when a theme is selected)",8.0,128.0);
    parent.append(&gtk::LinkButton::with_label("https://github.com/ful1e5/Bibata_Cursor/releases","Get Bibata cursors"));
    parent.append(&gtk::LinkButton::with_label("https://github.com/ful1e5/Google_Cursor/releases","Get GoogleDot cursors"));
    parent.append(&super::heading("Panel and launcher"));
    parent.append(&description("Automatic adapts to logical monitor width; Comfortable keeps labels and Compact uses icons. Enabled controls remain accessible by scrolling if needed."));
    let density=gtk::ComboBoxText::new();for (id,name) in [("auto","Automatic density"),("comfortable","Comfortable · icons and labels"),("compact","Compact · icons")]{density.append(Some(id),name);}parent.append(&density);
    let height=spin(parent,"Panel minimum height",32.0,80.0);
    let spacing=spin(parent,"Panel item spacing",0.0,24.0);
    let radius=spin(parent,"Button radius",0.0,24.0);
    let icons=spin(parent,"Launcher icon size",16.0,64.0);
    let grid=gtk::CheckButton::with_label("Open launcher in grid view by default");parent.append(&grid);
    let toggles:Vec<_>=["Show date","Show battery","Show media button","Show notifications button","Show audio button","Show terminal button"].iter().map(|title|{let field=gtk::CheckButton::with_label(title);parent.append(&field);field}).collect();
    parent.append(&super::heading("Application dock"));
    parent.append(&description("Optional bottom dock in panel mode. Native task support requires a compatible compositor. External desktops use launch-only mode."));
    let dock=gtk::CheckButton::with_label("Enable bottom application dock");parent.append(&dock);
    let auto_hide=gtk::CheckButton::with_label("Auto-hide dock; reveal at bottom center");parent.append(&auto_hide);
    let hide_fullscreen=gtk::CheckButton::with_label("Hide dock during native fullscreen");parent.append(&hide_fullscreen);
    let dock_icons=spin(parent,"Dock icon size",16.0,64.0);
    parent.append(&description("Choose an installed application to append, or edit the exact case-sensitive desktop IDs below. Order is left to right."));
    let picker=gtk::ComboBoxText::new();picker.set_hexpand(true);
    let mut apps:Vec<_>=gtk::gio::AppInfo::all().into_iter().filter(|app|app.should_show()).collect();
    apps.sort_by_key(|app|app.display_name().to_lowercase());
    let mut seen=std::collections::HashSet::new();
    for app in apps{
        if let Some(id)=app.id(){
            if id.ends_with(".desktop") && seen.insert(id.to_string()){
                picker.append(Some(&id),&format!("{} ({id})",app.display_name()));
            }
        }
    }
    let add=gtk::Button::with_label("Add pin");
    let row=gtk::Box::new(gtk::Orientation::Horizontal,8);row.append(&picker);row.append(&add);parent.append(&row);
    let pins=gtk::TextView::new();pins.set_monospace(true);pins.set_wrap_mode(gtk::WrapMode::WordChar);
    parent.append(&gtk::ScrolledWindow::builder().child(&pins).min_content_height(100).build());
    {let picker=picker.downgrade();let pins=pins.downgrade();add.connect_clicked(move |_|{
        let (Some(picker),Some(pins))=(picker.upgrade(),pins.upgrade())else{return;};
        let Some(id)=picker.active_id()else{return;};let buffer=pins.buffer();
        let text=buffer.text(&buffer.start_iter(),&buffer.end_iter(),true);
        if !text.lines().any(|line|line.trim()==id.as_str()){
            let text=text.trim_end();buffer.set_text(&if text.is_empty(){id.to_string()}else{format!("{text}\n{id}")});
        }
    });}
    let fields=Fields{cursor_theme,cursor_size,accent,height,spacing,radius,icons,toggles,density,grid,dock,auto_hide,hide_fullscreen,dock_icons,pins};
    let status=description("Load saved appearance to begin.");status.set_selectable(true);
    let reload=gtk::Button::with_label("Load saved appearance");
    let defaults=gtk::Button::with_label("Preview defaults");
    let previous=gtk::Button::with_label("Load previous appearance into draft");
    let save=gtk::Button::with_label("Save native appearance");save.add_css_class("suggested-action");
    for button in [&reload,&defaults,&previous,&save]{parent.append(button);}parent.append(&status);
    let state=Rc::new(RefCell::new(None::<AppearanceReview>));
    {
        let (fields,state,sample,simulate_reveal,simulate_fullscreen)=(fields.clone(),state.clone(),sample.clone(),simulate_reveal.clone(),simulate_fullscreen.clone());
        preview.set_draw_func(move|_,cr,w,h|{
            let state=state.borrow();let Some(review)=state.as_ref()else{return;};
            let draft=fields.collect(&review.value);
            let (width,tags)=match sample.active_id().as_deref(){Some("narrow")=>(800,4),Some("external")=>(1200,9),_=>(1440,4)};
            super::shell_preview::draw(cr,w,h,&draft,width,tags,super::shell_preview::DockSample{revealed:simulate_reveal.is_active(),fullscreen:simulate_fullscreen.is_active()});
        });
    }
    {let weak=preview.downgrade();fields.accent.connect_changed(move|_|{if let Some(p)=weak.upgrade(){p.queue_draw();}});}
    for field in [&fields.height,&fields.spacing,&fields.radius,&fields.icons,&fields.dock_icons]{let weak=preview.downgrade();field.connect_value_changed(move|_|{if let Some(p)=weak.upgrade(){p.queue_draw();}});}
    for field in fields.toggles.iter().chain([&fields.dock,&fields.grid,&fields.auto_hide,&fields.hide_fullscreen,&simulate_reveal,&simulate_fullscreen]){let weak=preview.downgrade();field.connect_toggled(move|_|{if let Some(p)=weak.upgrade(){p.queue_draw();}});}
    for field in [&fields.density,&sample]{let weak=preview.downgrade();field.connect_changed(move|_|{if let Some(p)=weak.upgrade(){p.queue_draw();}});}
    {let weak=preview.downgrade();fields.pins.buffer().connect_changed(move|_|{if let Some(p)=weak.upgrade(){p.queue_draw();}});}
    let enable={let (defaults,previous,save)=(defaults.downgrade(),previous.downgrade(),save.downgrade());Rc::new(move|enabled|{for button in [&defaults,&previous,&save]{if let Some(b)=button.upgrade(){b.set_sensitive(enabled);}}})};
    enable(false);
    {
        let (fields,state,status,enable,preview)=(fields.clone(),state.clone(),status.clone(),enable.clone(),preview.downgrade());
        reload.connect_clicked(move |_|{match AppearanceReview::load(&path){
            Ok(review)=>{fields.populate(&review.value);*state.borrow_mut()=Some(review);enable(true);status.set_text("Draft loaded. Colors, radius, panel and dock preferences update live; reopen the launcher for icon size and default view.");}
            Err(e)=>{state.borrow_mut().take();enable(false);status.set_text(&format!("Load failed; existing files unchanged: {e:#}"));}
        }
        if let Some(p)=preview.upgrade(){p.queue_draw();}});
    }
    {let (fields,status)=(fields.clone(),status.clone());defaults.connect_clicked(move |_|{fields.populate(&Appearance::default());status.set_text("Defaults in draft only.");});}
    {let (fields,state,status)=(fields.clone(),state.clone(),status.clone());previous.connect_clicked(move |_|{
        let baseline=state.borrow();let Some(r)=baseline.as_ref()else{return;};
        match r.previous(){Ok(value)=>{fields.populate(&value);status.set_text("Previous appearance loaded into draft. Save to restore.");},Err(e)=>status.set_text(&format!("{e:#}"))}
    });}
    save.connect_clicked(move |_|{
        let mut baseline=state.borrow_mut();let Some(r)=baseline.as_mut()else{return;};
        let next=fields.collect(&r.value);
        match r.save(next){
            Ok(true)=>status.set_text("Appearance saved. Updated shell applies panel and dock preferences live. Reopen launcher for icon size and default view. Restart native session for cursor preferences."),
            Ok(false)=>status.set_text("No appearance changes."),
            Err(e)=>status.set_text(&format!("Save failed: {e:#}")),
        }
    });
    reload.emit_clicked();
}
