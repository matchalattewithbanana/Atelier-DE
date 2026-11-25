//! Custom component metadata registry, with an immutable reviewed operation.
use gtk::prelude::*;
use std::{cell::RefCell,rc::Rc};
use crate::customization_registry::{self as registry,Action,Review};
use super::{background,label};

pub(super) fn build()->gtk::Box{
    let outer=gtk::Box::new(gtk::Orientation::Vertical,12);
    outer.set_margin_top(24);outer.set_margin_bottom(24);outer.set_margin_start(24);outer.set_margin_end(24);
    outer.append(&label("Custom components","title-1"));
    outer.append(&label("Register references to your own cursor, login, locker or appearance components. Manifest metadata is never executed. A separate trusted preparation and activation workflow is required.","dim-label"));
    let form=gtk::Box::new(gtk::Orientation::Vertical,8);outer.append(&form);
    let operation=gtk::ComboBoxText::new();
    for (id,title) in [("register","Register manifest"),("remove","Unregister reference"),("restore","Restore previous registry")]{operation.append(Some(id),title);}
    operation.set_active_id(Some("register"));form.append(&operation);
    let value=gtk::Entry::new();value.set_placeholder_text(Some("Absolute manifest path or custom:ID"));form.append(&value);
    let review_button=gtk::Button::with_label("Review registry change");form.append(&review_button);
    let detail=label("","body");detail.set_selectable(true);
    outer.append(&gtk::ScrolledWindow::builder().child(&detail).vexpand(true).min_content_height(130).build());
    let confirm=gtk::Entry::new();confirm.set_sensitive(false);outer.append(&confirm);
    let apply=gtk::Button::with_label("Apply reviewed metadata change");apply.set_sensitive(false);outer.append(&apply);
    let status=label("No changes made.","dim-label");status.set_selectable(true);outer.append(&status);
    let state=Rc::new(RefCell::new(None::<Review>));
    let invalidate={
        let (state,detail,confirm,apply)=(state.clone(),detail.downgrade(),confirm.downgrade(),apply.downgrade());
        Rc::new(move||{
            state.borrow_mut().take();
            if let Some(d)=detail.upgrade(){d.set_text("");}
            if let Some(c)=confirm.upgrade(){c.set_text("");c.set_sensitive(false);c.set_placeholder_text(Some("Review first"));}
            if let Some(a)=apply.upgrade(){a.set_sensitive(false);}
        })
    };
    {let invalidate=invalidate.clone();operation.connect_changed(move|_|invalidate());}
    {let invalidate=invalidate.clone();value.connect_changed(move|_|invalidate());}
    {
        let (form,operation,value,status,detail,confirm,apply)=(form.downgrade(),operation.downgrade(),value.downgrade(),status.downgrade(),detail.downgrade(),confirm.downgrade(),apply.downgrade());
        let state=state.clone();
        review_button.connect_clicked(move|_|{
            let (Some(f),Some(o),Some(v),Some(s))=(form.upgrade(),operation.upgrade(),value.upgrade(),status.upgrade())else{return;};
            let action=match o.active_id().as_deref(){Some("register")=>Action::Register,Some("remove")=>Action::Remove,Some("restore")=>Action::Restore,_=>return};
            let value=v.text().trim().to_owned();invalidate();f.set_sensitive(false);s.set_text("Reviewing metadata…");
            let (form,status,detail,confirm,apply,state)=(form.clone(),status.clone(),detail.clone(),confirm.clone(),apply.clone(),state.clone());
            background(move||registry::review(action,&value),move|result|{
                if let (Some(s),Some(d),Some(c),Some(a))=(status.upgrade(),detail.upgrade(),confirm.upgrade(),apply.upgrade()){
                    match result{
                        Ok(review)=>{d.set_text(review.detail());c.set_placeholder_text(Some(&format!("Type {}",review.phrase())));c.set_sensitive(true);a.set_sensitive(true);*state.borrow_mut()=Some(review);s.set_text("Review ready. Type the exact confirmation.");}
                        Err(e)=>s.set_text(&format!("Review failed: {e:#}")),
                    }
                }
                if let Some(f)=form.upgrade(){f.set_sensitive(true);}
            });
        });
    }
    {
        let (form,confirm,status,detail)=(form.downgrade(),confirm.downgrade(),status.downgrade(),detail.downgrade());
        apply.connect_clicked(move|button|{
            let (Some(f),Some(c),Some(s))=(form.upgrade(),confirm.upgrade(),status.upgrade())else{return;};
            let typed=c.text().to_string();
            if !state.borrow().as_ref().is_some_and(|r|r.phrase()==typed){s.set_text("Type the exact reviewed confirmation.");return;}
            let Some(review)=state.borrow_mut().take()else{return;};
            f.set_sensitive(false);button.set_sensitive(false);c.set_sensitive(false);c.set_text("");
            s.set_text("Rechecking registry and saving metadata…");
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
