//! Reviewed custom manifest registry; no install or launch.
use gtk::prelude::*;
use std::{cell::RefCell,rc::Rc,path::PathBuf};
use crate::rice::{self,RegistryAction,RegistryReview};
use super::{background,label};

pub(super) fn build()->gtk::Box{
    let outer=gtk::Box::new(gtk::Orientation::Vertical,12);
    outer.set_margin_top(24);outer.set_margin_bottom(24);outer.set_margin_start(24);outer.set_margin_end(24);
    outer.append(&label("Custom rices","title-1"));
    outer.append(&label("Register a trusted local Quickshell or command manifest. Registration never installs or launches it; later activation executes user-trusted code. Close all Atelier sessions. Export custom snapshots before unregistering.","dim-label"));
    let form=gtk::Box::new(gtk::Orientation::Vertical,8);outer.append(&form);
    let path=gtk::Entry::new();path.set_placeholder_text(Some("Absolute trusted Mango Atelier checkout"));form.append(&path);
    let trust=gtk::CheckButton::with_label("I trust this checkout and have closed all Atelier sessions.");form.append(&trust);
    let operation=gtk::ComboBoxText::new();operation.append(Some("register"),"Register local manifest");operation.append(Some("remove"),"Unregister custom rice");operation.set_active_id(Some("register"));form.append(&operation);
    let value=gtk::Entry::new();value.set_placeholder_text(Some("Absolute manifest JSON path or custom rice ID"));form.append(&value);
    let review=gtk::Button::with_label("Review manifest");form.append(&review);
    let detail=label("","body");detail.set_selectable(true);
    let scroll=gtk::ScrolledWindow::builder().child(&detail).vexpand(true).min_content_height(130).build();
    outer.append(&scroll);
    let confirmation=gtk::Entry::new();confirmation.set_sensitive(false);confirmation.set_placeholder_text(Some("Review first"));outer.append(&confirmation);
    let apply=gtk::Button::with_label("Apply reviewed registry change");apply.set_sensitive(false);outer.append(&apply);
    let status=label("No changes made.","dim-label");status.set_selectable(true);outer.append(&status);
    let state:Rc<RefCell<Option<RegistryReview>>>=Rc::new(RefCell::new(None));
    let invalidate={
        let (state,detail,confirmation,apply)=(state.clone(),detail.downgrade(),confirmation.downgrade(),apply.downgrade());
        Rc::new(move||{
            state.borrow_mut().take();
            if let Some(d)=detail.upgrade(){d.set_text("");}
            if let Some(c)=confirmation.upgrade(){c.set_text("");c.set_sensitive(false);c.set_placeholder_text(Some("Review first"));}
            if let Some(a)=apply.upgrade(){a.set_sensitive(false);}
        })
    };
    {let (trust,invalidate)=(trust.downgrade(),invalidate.clone());path.connect_changed(move|_|{if let Some(t)=trust.upgrade(){t.set_active(false);}invalidate();});}
    {let invalidate=invalidate.clone();trust.connect_toggled(move|_|invalidate());}
    {let invalidate=invalidate.clone();operation.connect_changed(move|_|invalidate());}
    {let invalidate=invalidate.clone();value.connect_changed(move|_|invalidate());}
    {
        let (form,path,trust,operation,value,status,detail,confirmation,apply)=(form.downgrade(),path.downgrade(),trust.downgrade(),operation.downgrade(),value.downgrade(),status.downgrade(),detail.downgrade(),confirmation.downgrade(),apply.downgrade());
        let state=state.clone();let invalidate=invalidate.clone();
        review.connect_clicked(move|_|{
            let (Some(f),Some(p),Some(t),Some(o),Some(v),Some(s))=(form.upgrade(),path.upgrade(),trust.upgrade(),operation.upgrade(),value.upgrade(),status.upgrade())else{return;};
            if !t.is_active(){s.set_text("Confirm checkout trust and offline state.");return;}
            let action=match o.active_id().as_deref(){Some("register")=>RegistryAction::Register,Some("remove")=>RegistryAction::Remove,_=>return};
            let p=PathBuf::from(p.text().trim());let v=v.text().trim().to_owned();
            invalidate();f.set_sensitive(false);s.set_text("Reviewing manifest bytes without launching the rice…");
            let (form,status,detail,confirmation,apply,state)=(form.clone(),status.clone(),detail.clone(),confirmation.clone(),apply.clone(),state.clone());
            background(move||{let r=rice::review_registry(&p,action,&v)?;let text=r.detail()?;Ok((r,text))},move|result|{
                if let (Some(s),Some(d),Some(c),Some(a))=(status.upgrade(),detail.upgrade(),confirmation.upgrade(),apply.upgrade()){
                    match result{
                        Ok((review,text))=>{d.set_text(&text);c.set_placeholder_text(Some(&format!("Type {}",review.phrase())));c.set_sensitive(true);a.set_sensitive(true);*state.borrow_mut()=Some(review);s.set_text("Review ready. Type the exact confirmation.");}
                        Err(e)=>s.set_text(&format!("Review failed: {e:#}")),
                    }
                }
                if let Some(f)=form.upgrade(){f.set_sensitive(true);}
            });
        });
    }
    {
        let (form,confirmation,status,detail)=(form.downgrade(),confirmation.downgrade(),status.downgrade(),detail.downgrade());
        apply.connect_clicked(move|button|{
            let (Some(f),Some(c),Some(s))=(form.upgrade(),confirmation.upgrade(),status.upgrade())else{return;};
            let typed=c.text().to_string();
            if !state.borrow().as_ref().is_some_and(|r|r.phrase()==typed){s.set_text("Type the exact reviewed confirmation.");return;}
            let Some(review)=state.borrow_mut().take()else{return;};
            f.set_sensitive(false);button.set_sensitive(false);c.set_sensitive(false);c.set_text("");
            s.set_text("Rechecking reviewed manifest and registry protection…");
            let (form,status,detail)=(form.clone(),status.clone(),detail.clone());
            background(move||review.apply(&typed),move|result|{
                if let Some(d)=detail.upgrade(){d.set_text("");}
                if let Some(s)=status.upgrade(){match result{Ok(text)=>s.set_text(&text),Err(e)=>s.set_text(&format!("Operation failed: {e:#}. Review again before retrying."))}}
                if let Some(f)=form.upgrade(){f.set_sensitive(true);}
            });
        });
    }
    outer
}
