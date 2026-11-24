//! Live previews are supervised by Mango Atelier; this window does not own the timer.
use gtk::prelude::*;
use std::{cell::RefCell, rc::Rc};
use crate::live_rice::{self, Operation, Status};
use super::{background,label};

pub(super) fn build() -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical,12);
    outer.set_margin_top(24); outer.set_margin_bottom(24); outer.set_margin_start(24); outer.set_margin_end(24);
    outer.append(&label("Live rice preview","title-1"));
    outer.append(&label("Requires a running, updated Mango Atelier supervisor in this exact Wayland session. Prepare a compatible rice first. The independent supervisor owns a 30-second preview and automatically attempts recovery on expiry or shell failure. No offline fallback.","dim-label"));
    let form = gtk::Box::new(gtk::Orientation::Vertical,10); outer.append(&form);
    let refresh = gtk::Button::with_label("Refresh live status"); form.append(&refresh);
    let id = gtk::Entry::new(); id.set_placeholder_text(Some("Prepared local rice ID")); form.append(&id);
    let approve = gtk::Entry::new(); approve.set_placeholder_text(Some("Type preview RICE_ID")); form.append(&approve);
    let begin = gtk::Button::with_label("Try for 30 seconds"); begin.set_sensitive(false); form.append(&begin);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal,8);
    let apply = gtk::Button::with_label("Keep preview");
    let revert = gtk::Button::with_label("Revert preview");
    apply.set_sensitive(false); revert.set_sensitive(false);
    controls.append(&apply); controls.append(&revert); form.append(&controls);
    let status = label("Refresh to connect. No live changes made.","dim-label"); status.set_selectable(true); outer.append(&status);
    let info = label("","body"); info.set_selectable(true); outer.append(&info);
    let state: Rc<RefCell<Option<Status>>> = Rc::new(RefCell::new(None));
    { let approve = approve.downgrade(); id.connect_changed(move |_| { if let Some(approve)=approve.upgrade(){approve.set_text("");} }); }
    for (button, action) in [(refresh.clone(),0),(begin.clone(),1),(apply.clone(),2),(revert.clone(),3)] {
        let (form,status,info,id,approve,begin,apply,revert)=(form.downgrade(),status.downgrade(),info.downgrade(),id.downgrade(),approve.downgrade(),begin.downgrade(),apply.downgrade(),revert.downgrade());
        let state=state.clone();
        button.connect_clicked(move |_| {
            let (Some(form_widget),Some(message),Some(id_widget),Some(approval))=(form.upgrade(),status.upgrade(),id.upgrade(),approve.upgrade()) else{return;};
            let operation = match action {
                0 => Operation::Status,
                1 => {
                    let id=id_widget.text().trim().to_owned();
                    if approval.text().as_str()!=format!("preview {id}") {message.set_text("Type preview followed by the exact rice ID.");return;}
                    if !state.borrow().as_ref().is_some_and(|s| s.active.is_some() && s.preview.is_none()) {message.set_text("Refresh live status before starting.");return;}
                    Operation::Begin(id)
                }
                _ => {
                    let token=state.borrow().as_ref().and_then(|s|s.preview.as_ref()).map(|p|p.token.clone());
                    let Some(token)=token else{message.set_text("Refresh live status first.");return;};
                    if action==2 {Operation::Apply(token)} else {Operation::Revert(token)}
                }
            };
            approval.set_text(""); state.borrow_mut().take();
            form_widget.set_sensitive(false); message.set_text("Contacting the live supervisor…");
            let (form,status,info,begin,apply,revert,state)=(form.clone(),status.clone(),info.clone(),begin.clone(),apply.clone(),revert.clone(),state.clone());
            background(move||live_rice::execute(operation),move|result|{
                let (Some(form),Some(status),Some(info),Some(begin),Some(apply),Some(revert))=(form.upgrade(),status.upgrade(),info.upgrade(),begin.upgrade(),apply.upgrade(),revert.upgrade()) else{return;};
                begin.set_sensitive(false); apply.set_sensitive(false); revert.set_sensitive(false);
                match result {
                    Ok(result)=>{
                        if let Some(p)=&result.preview{
                            info.set_text(&format!("Trying {} · previous {}\n{} seconds remaining when checked. Refresh for current status. The independent preview panel can also apply or revert.",p.rice,p.previous,p.seconds));
                            apply.set_sensitive(p.seconds>0); revert.set_sensitive(p.seconds>0);
                        }else{
                            info.set_text(&format!("Active rice: {}\nNo active preview.",result.active.as_deref().unwrap_or("none")));
                            begin.set_sensitive(result.active.is_some());
                        }
                        status.set_text("Supervisor responded. Live status is a snapshot, not a running countdown.");
                        *state.borrow_mut()=Some(result);
                    }
                    Err(e)=>{info.set_text("");status.set_text(&format!("Request failed: {e:#}\nRefresh status before retrying. The supervisor still owns recovery."));}
                }
                form.set_sensitive(true);
            });
        });
    }
    outer
}
