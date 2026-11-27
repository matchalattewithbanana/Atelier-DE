//! Widget settings own only the widget group; never overwrite other appearance fields.
use atelier_config::{appearance_path,AppearanceReview,DesktopWidgets,WidgetPosition};
use gtk::prelude::*;
use std::{cell::RefCell,rc::Rc};
use super::description;

#[derive(Clone)]
struct Fields {
    enable:gtk::CheckButton,position:gtk::ComboBoxText,width:gtk::SpinButton,
    cards:Vec<gtk::CheckButton>,
}
impl Fields {
    fn populate(&self,value:&DesktopWidgets){
        self.enable.set_active(value.enabled);
        self.position.set_active_id(Some(match value.position{WidgetPosition::TopLeft=>"left",WidgetPosition::TopRight=>"right"}));
        self.width.set_value(value.width as f64);
        for (button,enabled) in self.cards.iter().zip([value.clock,value.calendar,value.media,value.system]){button.set_active(enabled);}
    }
    fn collect(&self)->DesktopWidgets{
        DesktopWidgets{enabled:self.enable.is_active(),position:if self.position.active_id().as_deref()==Some("right"){WidgetPosition::TopRight}else{WidgetPosition::TopLeft},
            width:self.width.value_as_int() as u32,clock:self.cards[0].is_active(),calendar:self.cards[1].is_active(),media:self.cards[2].is_active(),system:self.cards[3].is_active()}
    }
}
pub fn build(parent:&gtk::Box){
    let path=match appearance_path(){Ok(path)=>path,Err(e)=>{parent.append(&description(&format!("{e:#}")));return;}};
    parent.append(&description("Widgets appear behind windows on the native desktop. Media uses the existing playerctl service. No external desktop changes."));
    parent.append(&description("Draft layout · Sample clock, calendar, media and system data. No live widgets or provider queries."));
    let preview=gtk::DrawingArea::new();preview.set_content_height(270);preview.set_hexpand(true);parent.append(&preview);
    let enable=gtk::CheckButton::with_label("Enable desktop widgets");parent.append(&enable);
    let position=gtk::ComboBoxText::new();position.append(Some("left"),"Top left");position.append(Some("right"),"Top right");parent.append(&position);
    parent.append(&description("Group width (logical pixels)"));let width=gtk::SpinButton::with_range(220.0,360.0,10.0);parent.append(&width);
    let cards=["Clock and date","Calendar","Now playing and media controls","Memory, load average and power"].iter().map(|name|{let button=gtk::CheckButton::with_label(name);parent.append(&button);button}).collect();
    let fields=Fields{enable,position,width,cards};
    let reload=gtk::Button::with_label("Reload saved widgets");parent.append(&reload);
    let previous=gtk::Button::with_label("Load previous widget preferences into draft");parent.append(&previous);
    let reset=gtk::Button::with_label("Reset draft to defaults");parent.append(&reset);
    let save=gtk::Button::with_label("Save widget preferences");save.add_css_class("suggested-action");parent.append(&save);
    let status=description("");status.set_selectable(true);parent.append(&status);
    let baseline=Rc::new(RefCell::new(None::<AppearanceReview>));
    save.set_sensitive(false);
    {
        let (baseline,fields)=(baseline.clone(),fields.clone());
        preview.set_draw_func(move|_,cr,w,h|{
            let baseline=baseline.borrow();let Some(review)=baseline.as_ref()else{return;};
            let mut draft=review.value.clone();draft.widgets=fields.collect();
            super::shell_preview::widgets(cr,w,h,&draft);
        });
    }
    for field in fields.cards.iter().chain([&fields.enable]){let p=preview.downgrade();field.connect_toggled(move|_|{if let Some(p)=p.upgrade(){p.queue_draw();}});}
    {let p=preview.downgrade();fields.position.connect_changed(move|_|{if let Some(p)=p.upgrade(){p.queue_draw();}});}
    {let p=preview.downgrade();fields.width.connect_value_changed(move|_|{if let Some(p)=p.upgrade(){p.queue_draw();}});}
    {let (baseline,fields,status,save,preview)=(baseline.clone(),fields.clone(),status.clone(),save.downgrade(),preview.downgrade());
    reload.connect_clicked(move|_|{match AppearanceReview::load(&path){
        Ok(review)=>{fields.populate(&review.value.widgets);*baseline.borrow_mut()=Some(review);if let Some(s)=save.upgrade(){s.set_sensitive(true);}status.set_text("Saved widgets loaded.");}
        Err(e)=>{baseline.borrow_mut().take();if let Some(s)=save.upgrade(){s.set_sensitive(false);}status.set_text(&format!("Load failed: {e:#}"));}
    }
    if let Some(p)=preview.upgrade(){p.queue_draw();}});}
    {let (fields,status)=(fields.clone(),status.clone());reset.connect_clicked(move|_|{fields.populate(&DesktopWidgets::default());status.set_text("Defaults in draft only.");});}
    {let (baseline,fields,status)=(baseline.clone(),fields.clone(),status.clone());previous.connect_clicked(move|_|{
        let state=baseline.borrow();let Some(review)=state.as_ref()else{status.set_text("Reload first.");return;};
        match review.previous(){Ok(value)=>{fields.populate(&value.widgets);status.set_text("Previous widgets in draft only.");},Err(e)=>status.set_text(&format!("{e:#}"))}
    });}
    save.connect_clicked(move|_|{
        let mut state=baseline.borrow_mut();let Some(review)=state.as_mut()else{return;};
        let mut next=review.value.clone();next.widgets=fields.collect();
        match review.save(next){Ok(true)=>status.set_text("Widgets saved. Updated native shell applies widget preferences live."),Ok(false)=>status.set_text("No widget changes."),Err(e)=>status.set_text(&format!("Save failed: {e:#}. Reload before retrying."))}
    });
    reload.emit_clicked();
}
