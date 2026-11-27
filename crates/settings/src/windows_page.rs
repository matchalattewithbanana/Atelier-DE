//! Native window spacing editor, sharing reviewed appearance persistence.
use atelier_config::{appearance_path,Appearance,AppearanceReview,WindowSpacing,WindowBorder,WindowShadow};
use gtk::prelude::*;
use std::{cell::RefCell,rc::Rc};
use super::description;
pub fn build(parent:&gtk::Box){
    let path=match appearance_path(){Ok(path)=>path,Err(e)=>{parent.append(&description(&format!("{e:#}")));return;}};
    parent.append(&description("Grid mode only: spacing is measured in logical pixels and reduced automatically on tiny outputs. Maximize uses the available workspace; fullscreen uses the entire output. Borders apply to floating and grid windows; maximized and fullscreen windows have no border."));
    parent.append(&description("Draft illustration · Two sample grid windows. Changes here do not affect the desktop until saved. Loaded window opacity is included; blur, per-app overrides and actual output constraints are not simulated."));
    let preview=gtk::DrawingArea::new();preview.set_content_height(270);preview.set_hexpand(true);parent.append(&preview);
    let swap_focus=gtk::CheckButton::with_label("Focus the right sample window");parent.append(&swap_focus);
    parent.append(&description("Inner gap (between tiles)"));let inner=gtk::SpinButton::with_range(0.0,48.0,1.0);parent.append(&inner);
    parent.append(&description("Outer gap (around tiled workspace)"));let outer=gtk::SpinButton::with_range(0.0,64.0,1.0);parent.append(&outer);
    parent.append(&description("Border width (0 disables borders)"));let width=gtk::SpinButton::with_range(0.0,8.0,1.0);parent.append(&width);
    parent.append(&description("Focused border color (#RRGGBB)"));let focused=gtk::Entry::new();parent.append(&focused);
    parent.append(&description("Unfocused border color (#RRGGBB)"));let unfocused=gtk::Entry::new();parent.append(&unfocused);
    parent.append(&description("Ambient shadow radius (0 disables)"));let shadow_radius=gtk::SpinButton::with_range(0.0,32.0,1.0);parent.append(&shadow_radius);
    parent.append(&description("Shadow opacity (%)"));let shadow_opacity=gtk::SpinButton::with_range(0.0,60.0,1.0);parent.append(&shadow_opacity);
    parent.append(&description("Window corner radius (0 disables)"));let corner_radius=gtk::SpinButton::with_range(0.0,24.0,1.0);parent.append(&corner_radius);
    let reload=gtk::Button::with_label("Reload saved window appearance");parent.append(&reload);
    let previous=gtk::Button::with_label("Load previous window appearance into draft");parent.append(&previous);
    let reset=gtk::Button::with_label("Reset window appearance in draft");parent.append(&reset);
    let save=gtk::Button::with_label("Save window appearance");save.add_css_class("suggested-action");save.set_sensitive(false);parent.append(&save);
    let status=description("");status.set_selectable(true);parent.append(&status);
    let state=Rc::new(RefCell::new(None::<AppearanceReview>));
    let populate={let (inner,outer,width,focused,unfocused,shadow_radius,shadow_opacity,corner_radius)=(inner.clone(),outer.clone(),width.clone(),focused.clone(),unfocused.clone(),shadow_radius.clone(),shadow_opacity.clone(),corner_radius.clone());move|spacing:WindowSpacing,border:&WindowBorder,shadow:WindowShadow,corner:u32|{
        inner.set_value(spacing.inner_gap as f64);outer.set_value(spacing.outer_gap as f64);width.set_value(border.width as f64);focused.set_text(&border.focused);unfocused.set_text(&border.unfocused);shadow_radius.set_value(shadow.radius as f64);shadow_opacity.set_value(shadow.opacity as f64);corner_radius.set_value(corner as f64);
    }};
    // Share collection between the preview and the reviewed save, so neither
    // can silently interpret a draft control differently.
    let collect={let (inner,outer,width,focused,unfocused,shadow_radius,shadow_opacity,corner_radius)=(inner.clone(),outer.clone(),width.clone(),focused.clone(),unfocused.clone(),shadow_radius.clone(),shadow_opacity.clone(),corner_radius.clone());move|baseline:&Appearance|{
        let mut next=baseline.clone();
        next.window_spacing=WindowSpacing{inner_gap:inner.value_as_int() as u32,outer_gap:outer.value_as_int() as u32};
        next.window_border=WindowBorder{width:width.value_as_int() as u32,focused:focused.text().to_string(),unfocused:unfocused.text().to_string()};
        next.window_shadow=WindowShadow{radius:shadow_radius.value_as_int() as u32,opacity:shadow_opacity.value_as_int() as u32};
        next.window_corner_radius=corner_radius.value_as_int() as u32;
        next
    }};
    {
        let (state,collect,swap_focus)=(state.clone(),collect.clone(),swap_focus.clone());
        preview.set_draw_func(move|_,cr,w,h|{
            let state=state.borrow();let Some(review)=state.as_ref()else{return;};
            super::window_preview::draw(cr,w,h,&collect(&review.value),swap_focus.is_active());
        });
    }
    for field in [&inner,&outer,&width,&shadow_radius,&shadow_opacity,&corner_radius]{
        let weak=preview.downgrade();field.connect_value_changed(move|_|{if let Some(p)=weak.upgrade(){p.queue_draw();}});
    }
    for field in [&focused,&unfocused]{
        let weak=preview.downgrade();field.connect_changed(move|_|{if let Some(p)=weak.upgrade(){p.queue_draw();}});
    }
    {let weak=preview.downgrade();swap_focus.connect_toggled(move|_|{if let Some(p)=weak.upgrade(){p.queue_draw();}});}
    {let (state,populate,status,save,preview)=(state.clone(),populate.clone(),status.clone(),save.downgrade(),preview.downgrade());
    reload.connect_clicked(move|_|{match AppearanceReview::load(&path){
        Ok(review)=>{populate(review.value.window_spacing,&review.value.window_border,review.value.window_shadow,review.value.window_corner_radius);*state.borrow_mut()=Some(review);if let Some(s)=save.upgrade(){s.set_sensitive(true);}status.set_text("Saved window appearance loaded.");}
        Err(e)=>{state.borrow_mut().take();if let Some(s)=save.upgrade(){s.set_sensitive(false);}status.set_text(&format!("Load failed: {e:#}"));}
    }
    if let Some(p)=preview.upgrade(){p.queue_draw();}});}
    {let (populate,status)=(populate.clone(),status.clone());reset.connect_clicked(move|_|{populate(WindowSpacing::default(),&WindowBorder::default(),WindowShadow::default(),0);status.set_text("Defaults in draft only.");});}
    {let (state,populate,status)=(state.clone(),populate.clone(),status.clone());previous.connect_clicked(move|_|{
        let state=state.borrow();let Some(review)=state.as_ref()else{status.set_text("Reload first.");return;};
        match review.previous(){Ok(value)=>{populate(value.window_spacing,&value.window_border,value.window_shadow,value.window_corner_radius);status.set_text("Previous window appearance loaded into draft.");},Err(e)=>status.set_text(&format!("{e:#}"))}
    });}
    save.connect_clicked(move|_|{
        let mut state=state.borrow_mut();let Some(review)=state.as_mut()else{return;};
        let next=collect(&review.value);
        match review.save(next){Ok(true)=>status.set_text("Saved. Running native compositor applies window appearance after active input grabs finish."),Ok(false)=>status.set_text("No window appearance changes."),Err(e)=>status.set_text(&format!("Save failed: {e:#}. Reload before retrying."))}
    });
    reload.emit_clicked();
}
