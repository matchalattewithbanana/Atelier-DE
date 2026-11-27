//! Local motion settings; portable themes cannot override accessibility.
use atelier_config::{appearance_path,AppearanceReview,Motion};
use gtk::{glib,prelude::*};
use std::{cell::{Cell,RefCell},rc::Rc,time::{Duration,Instant}};
use super::description;

/// Preview time is derived from the same accessibility policy as the native
/// compositor. No repeating animation, and no animation for reduced motion.
fn alpha(duration:Duration,elapsed:Duration)->f64{
    if duration.is_zero(){1.0}else{(elapsed.as_secs_f64()/duration.as_secs_f64()).clamp(0.0,1.0)}
}
fn draw_sample(cr:&gtk::cairo::Context,w:i32,h:i32,opacity:f64){
    if w<=0||h<=0{return;}
    let scale=(f64::from(w)/440.0).min(f64::from(h)/220.0);
    let _=cr.save();cr.translate((f64::from(w)-440.0*scale)/2.0,(f64::from(h)-220.0*scale)/2.0);cr.scale(scale,scale);
    cr.set_source_rgb(0.15,0.19,0.27);cr.rectangle(0.0,0.0,440.0,220.0);let _=cr.fill();
    cr.set_source_rgb(0.26,0.35,0.48);cr.rectangle(22.0,30.0,396.0,170.0);let _=cr.fill();
    cr.push_group();
    cr.set_source_rgb(0.10,0.13,0.19);cr.rectangle(95.0,42.0,250.0,144.0);let _=cr.fill();
    cr.set_source_rgb(0.25,0.31,0.43);cr.rectangle(95.0,42.0,250.0,30.0);let _=cr.fill();
    cr.set_source_rgb(0.93,0.95,0.99);cr.set_font_size(14.0);cr.move_to(108.0,62.0);let _=cr.show_text("Sample application");
    cr.set_source_rgb(0.46,0.55,0.71);
    for (i,width) in [180.0,122.0,156.0].into_iter().enumerate(){cr.rectangle(109.0,91.0+i as f64*20.0,width,6.0);let _=cr.fill();}
    let _=cr.pop_group_to_source();let _=cr.paint_with_alpha(opacity);
    let _=cr.restore();
}
pub fn build(parent:&gtk::Box){
    let path=match appearance_path(){Ok(p)=>p,Err(e)=>{parent.append(&description(&format!("{e:#}")));return;}};
    parent.append(&description("Opening fades apply only to ordinary newly mapped native windows without active focused backdrop blur. Input and layout remain unchanged. Reduced motion immediately settles any active fades when preferences are applied."));
    let enabled=gtk::CheckButton::with_label("Animate window opening");parent.append(&enabled);
    parent.append(&description("Opening duration (milliseconds)"));
    let duration=gtk::SpinButton::with_range(80.0,400.0,10.0);duration.set_value(180.0);parent.append(&duration);
    let reduced=gtk::CheckButton::with_label("Reduce motion (overrides opening animation)");parent.append(&reduced);
    parent.append(&description("Sample opening fade · Plays only when requested; changes are never applied to the desktop by this preview."));
    let canvas=gtk::DrawingArea::new();canvas.set_content_height(220);canvas.set_hexpand(true);
    canvas.update_property(&[gtk::accessible::Property::Label("Illustrative window opening fade")]);parent.append(&canvas);
    let replay=gtk::Button::with_label("Play sample opening");parent.append(&replay);
    let effective=description("");parent.append(&effective);
    let opacity=Rc::new(Cell::new(1.0));
    {let opacity=opacity.clone();canvas.set_draw_func(move|_,cr,w,h|draw_sample(cr,w,h,opacity.get()));}
    let collect={let (enabled,duration,reduced)=(enabled.clone(),duration.clone(),reduced.clone());move||Motion{
        opening_fade_ms:if enabled.is_active(){duration.value_as_int() as u32}else{0},reduced_motion:reduced.is_active()
    }};
    // A generation invalidates every earlier playback when settings change
    // or Replay is clicked again. At most one generation can paint.
    let generation=Rc::new(Cell::new(0u64));
    let settle={
        let (generation,opacity,canvas,effective,duration,enabled,reduced)=(generation.clone(),opacity.clone(),canvas.downgrade(),effective.downgrade(),duration.downgrade(),enabled.downgrade(),reduced.downgrade());
        move||{
            generation.set(generation.get().wrapping_add(1));opacity.set(1.0);
            if let Some(c)=canvas.upgrade(){c.queue_draw();}
            let (Some(d),Some(e),Some(r),Some(label))=(duration.upgrade(),enabled.upgrade(),reduced.upgrade(),effective.upgrade())else{return;};
            d.set_sensitive(e.is_active()&&!r.is_active());
            let motion=Motion{opening_fade_ms:if e.is_active(){d.value_as_int() as u32}else{0},reduced_motion:r.is_active()};
            let ms=motion.opening_duration().as_millis();
            label.set_text(&if r.is_active(){"Effective opening duration: instant (reduced motion)".into()}else if ms==0{"Effective opening duration: instant (animation disabled)".into()}else{format!("Effective opening duration: {ms} ms")});
        }
    };
    {let f=settle.clone();enabled.connect_toggled(move|_|f());}
    {let f=settle.clone();reduced.connect_toggled(move|_|f());}
    {let f=settle.clone();duration.connect_value_changed(move|_|f());}
    {
        let (generation,opacity,canvas,collect)=(generation.clone(),opacity.clone(),canvas.downgrade(),collect.clone());
        replay.connect_clicked(move|_|{
            let Some(c)=canvas.upgrade()else{return;};
            let duration=collect().opening_duration();
            let token=generation.get().wrapping_add(1);generation.set(token);
            opacity.set(alpha(duration,Duration::ZERO));c.queue_draw();
            if duration.is_zero(){return;}
            let (generation,opacity,canvas)=(generation.clone(),opacity.clone(),canvas.clone());
            let start=Instant::now();
            glib::timeout_add_local(Duration::from_millis(16),move||{
                if generation.get()!=token{return glib::ControlFlow::Break;}
                let Some(c)=canvas.upgrade()else{return glib::ControlFlow::Break;};
                let a=alpha(duration,start.elapsed());opacity.set(a);c.queue_draw();
                if a>=1.0{glib::ControlFlow::Break}else{glib::ControlFlow::Continue}
            });
        });
    }
    let reload=gtk::Button::with_label("Reload saved motion settings");parent.append(&reload);
    let previous=gtk::Button::with_label("Load previous motion settings into draft");parent.append(&previous);
    let reset=gtk::Button::with_label("Reset motion settings in draft");parent.append(&reset);
    let save=gtk::Button::with_label("Save motion preferences");save.add_css_class("suggested-action");save.set_sensitive(false);parent.append(&save);
    let status=description("");status.set_selectable(true);parent.append(&status);
    let state=Rc::new(RefCell::new(None::<AppearanceReview>));
    let populate={let (enabled,duration,reduced)=(enabled.clone(),duration.clone(),reduced.clone());move|value:Motion|{
        enabled.set_active(value.opening_fade_ms>0);
        duration.set_value(if value.opening_fade_ms==0{180.0}else{f64::from(value.opening_fade_ms)});
        reduced.set_active(value.reduced_motion);
    }};
    settle();
    {let (state,populate,status,save)=(state.clone(),populate.clone(),status.clone(),save.downgrade());
    reload.connect_clicked(move|_|match AppearanceReview::load(&path){
        Ok(review)=>{populate(review.value.motion);*state.borrow_mut()=Some(review);if let Some(s)=save.upgrade(){s.set_sensitive(true);}status.set_text("Saved motion preferences loaded.");}
        Err(e)=>{state.borrow_mut().take();if let Some(s)=save.upgrade(){s.set_sensitive(false);}status.set_text(&format!("Load failed: {e:#}"));}
    });}
    {let (state,populate,status)=(state.clone(),populate.clone(),status.clone());
    previous.connect_clicked(move|_|{
        let state=state.borrow();let Some(review)=state.as_ref()else{return;};
        match review.previous(){Ok(value)=>{populate(value.motion);status.set_text("Previous motion preferences loaded into draft.");}Err(e)=>status.set_text(&format!("{e:#}"))}
    });}
    {let (populate,status)=(populate.clone(),status.clone());reset.connect_clicked(move|_|{populate(Motion::default());status.set_text("Defaults in draft only.");});}
    save.connect_clicked(move|_|{
        let mut state=state.borrow_mut();let Some(review)=state.as_mut()else{return;};
        let mut next=review.value.clone();
        next.motion=collect();
        match review.save(next){
            Ok(true)=>status.set_text("Saved. The running native compositor applies motion preferences after active input grabs finish."),
            Ok(false)=>status.set_text("No changes."),
            Err(e)=>status.set_text(&format!("Save failed: {e:#}. Reload before retrying.")),
        }
    });
    reload.emit_clicked();
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn preview_is_bounded_and_settles(){
        let duration=Duration::from_millis(200);
        assert_eq!(alpha(duration,Duration::ZERO),0.0);
        assert_eq!(alpha(duration,Duration::from_millis(100)),0.5);
        assert_eq!(alpha(duration,Duration::from_secs(2)),1.0);
        assert_eq!(alpha(Duration::ZERO,Duration::ZERO),1.0);
    }
    #[test]
    fn accessibility_policy_wins_over_enabled_animation(){
        let motion=Motion{opening_fade_ms:400,reduced_motion:true};
        assert_eq!(alpha(motion.opening_duration(),Duration::ZERO),1.0);
        assert_eq!(Motion{reduced_motion:false,..motion}.opening_duration(),Duration::from_millis(400));
    }
    #[test]
    fn sample_renders_without_a_display(){
        let surface=gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32,220,110).unwrap();
        let cr=gtk::cairo::Context::new(&surface).unwrap();
        for opacity in [0.0,0.5,1.0]{draw_sample(&cr,220,110,opacity);}
        assert!(cr.status().is_ok());
    }
}
