//! Native window spacing editor, sharing reviewed appearance persistence.
use atelier_config::{appearance_path,AppearanceReview,WindowSpacing,WindowBorder};
use gtk::prelude::*;
use std::{cell::RefCell,rc::Rc};
use super::description;
pub fn build(parent:&gtk::Box){
    let path=match appearance_path(){Ok(path)=>path,Err(e)=>{parent.append(&description(&format!("{e:#}")));return;}};
    parent.append(&description("Grid mode only: spacing is measured in logical pixels and reduced automatically on tiny outputs. Maximize uses the available workspace; fullscreen uses the entire output. Borders apply to floating and grid windows; maximized and fullscreen windows have no border."));
    parent.append(&description("Inner gap (between tiles)"));let inner=gtk::SpinButton::with_range(0.0,48.0,1.0);parent.append(&inner);
    parent.append(&description("Outer gap (around tiled workspace)"));let outer=gtk::SpinButton::with_range(0.0,64.0,1.0);parent.append(&outer);
    parent.append(&description("Border width (0 disables borders)"));let width=gtk::SpinButton::with_range(0.0,8.0,1.0);parent.append(&width);
    parent.append(&description("Focused border color (#RRGGBB)"));let focused=gtk::Entry::new();parent.append(&focused);
    parent.append(&description("Unfocused border color (#RRGGBB)"));let unfocused=gtk::Entry::new();parent.append(&unfocused);
    let reload=gtk::Button::with_label("Reload saved window appearance");parent.append(&reload);
    let previous=gtk::Button::with_label("Load previous window appearance into draft");parent.append(&previous);
    let reset=gtk::Button::with_label("Reset window appearance in draft");parent.append(&reset);
    let save=gtk::Button::with_label("Save window appearance");save.add_css_class("suggested-action");save.set_sensitive(false);parent.append(&save);
    let status=description("");status.set_selectable(true);parent.append(&status);
    let state=Rc::new(RefCell::new(None::<AppearanceReview>));
    let populate={let (inner,outer,width,focused,unfocused)=(inner.clone(),outer.clone(),width.clone(),focused.clone(),unfocused.clone());move|spacing:WindowSpacing,border:&WindowBorder|{
        inner.set_value(spacing.inner_gap as f64);outer.set_value(spacing.outer_gap as f64);width.set_value(border.width as f64);focused.set_text(&border.focused);unfocused.set_text(&border.unfocused);
    }};
    {let (state,populate,status,save)=(state.clone(),populate.clone(),status.clone(),save.downgrade());
    reload.connect_clicked(move|_|match AppearanceReview::load(&path){
        Ok(review)=>{populate(review.value.window_spacing,&review.value.window_border);*state.borrow_mut()=Some(review);if let Some(s)=save.upgrade(){s.set_sensitive(true);}status.set_text("Saved spacing loaded.");}
        Err(e)=>{state.borrow_mut().take();if let Some(s)=save.upgrade(){s.set_sensitive(false);}status.set_text(&format!("Load failed: {e:#}"));}
    });}
    {let (populate,status)=(populate.clone(),status.clone());reset.connect_clicked(move|_|{populate(WindowSpacing::default(),&WindowBorder::default());status.set_text("Defaults in draft only.");});}
    {let (state,populate,status)=(state.clone(),populate.clone(),status.clone());previous.connect_clicked(move|_|{
        let state=state.borrow();let Some(review)=state.as_ref()else{status.set_text("Reload first.");return;};
        match review.previous(){Ok(value)=>{populate(value.window_spacing,&value.window_border);status.set_text("Previous spacing loaded into draft.");},Err(e)=>status.set_text(&format!("{e:#}"))}
    });}
    save.connect_clicked(move|_|{
        let mut state=state.borrow_mut();let Some(review)=state.as_mut()else{return;};
        let mut next=review.value.clone();next.window_spacing=WindowSpacing{inner_gap:inner.value_as_int() as u32,outer_gap:outer.value_as_int() as u32};
        next.window_border=WindowBorder{width:width.value_as_int() as u32,focused:focused.text().to_string(),unfocused:unfocused.text().to_string()};
        match review.save(next){Ok(true)=>status.set_text("Saved. Running native compositor applies window appearance after active input grabs finish."),Ok(false)=>status.set_text("No window appearance changes."),Err(e)=>status.set_text(&format!("Save failed: {e:#}. Reload before retrying."))}
    });
    reload.emit_clicked();
}
