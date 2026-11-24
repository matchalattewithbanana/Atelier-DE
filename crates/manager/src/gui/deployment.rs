//! Confirmed local activation and rollback; no privileged or remote operation.
use gtk::prelude::*;
use std::{cell::RefCell, rc::Rc};
use crate::selection;
use super::{background, label};

pub(super) fn controls(window: &gtk::ApplicationWindow, refresh: &gtk::Button, recovery: bool) -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 8);
    container.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let status = label("Review a selection change before applying it.", "dim-label");
    status.set_selectable(true);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let target = gtk::Entry::new();
    target.set_placeholder_text(Some("Deployment ID"));
    target.set_hexpand(true);
    let historical = gtk::CheckButton::with_label("Historical selection");
    let activate = gtk::Button::with_label("Review activation");
    let rollback = gtk::Button::with_label("Review rollback");
    let verify = gtk::Button::with_label("Verify");
    if !recovery {
        line.append(&target);
        line.append(&verify);
        line.append(&activate);
        container.append(&historical);
    }
    line.append(&rollback);
    container.append(&line);
    container.append(&status);
    let buttons = vec![activate.clone(), rollback.clone(), verify.clone()];
    let weak_buttons: Vec<_> = buttons.iter().map(|b| b.downgrade()).collect();
    let set_busy = move |busy: bool| {
        for button in &weak_buttons { if let Some(button) = button.upgrade() { button.set_sensitive(!busy); } }
    };
    let set_busy = Rc::new(set_busy);
    let request = {
        let window = window.downgrade();
        let refresh = refresh.downgrade();
        let status = status.downgrade();
        let set_busy = set_busy.clone();
        Rc::new(move |id: Option<String>, history: bool| {
            let Some(message) = status.upgrade() else { return; };
            set_busy(true);
            message.set_text("Verifying target and reviewing selection…");
            let (window, refresh, status, set_busy) = (window.clone(), refresh.clone(), status.clone(), set_busy.clone());
            background(move || selection::review_activation(id.as_deref(), history), move |result| {
                let (Some(window), Some(message)) = (window.upgrade(), status.upgrade()) else { return; };
                let reviewed = match result {
                    Ok(reviewed) => reviewed,
                    Err(error) => { message.set_text(&format!("Review failed: {error:#}")); set_busy(false); return; }
                };
                let dialog = gtk::Dialog::builder().title("Confirm deployment selection")
                    .transient_for(&window).modal(true).default_width(560).build();
                dialog.add_button("Cancel", gtk::ResponseType::Cancel);
                dialog.add_button("Select deployment", gtk::ResponseType::Accept);
                dialog.set_default_response(gtk::ResponseType::Cancel);
                let summary = label(reviewed.detail(), "body");
                summary.set_selectable(true);
                summary.set_margin_top(18); summary.set_margin_bottom(18);
                summary.set_margin_start(18); summary.set_margin_end(18);
                dialog.content_area().append(&summary);
                let reviewed = Rc::new(RefCell::new(Some(reviewed)));
                let (status, refresh, set_busy) = (status.clone(), refresh.clone(), set_busy.clone());
                message.set_text("Awaiting explicit confirmation.");
                dialog.connect_response(move |dialog, response| {
                    let plan = reviewed.borrow_mut().take();
                    dialog.close();
                    if response != gtk::ResponseType::Accept {
                        if let Some(message) = status.upgrade() { message.set_text("Cancelled. Selection unchanged."); }
                        set_busy(false); return;
                    }
                    let Some(plan) = plan else { return; };
                    if let Some(message) = status.upgrade() { message.set_text("Rechecking reviewed target and applying selection…"); }
                    let (status, refresh, set_busy) = (status.clone(), refresh.clone(), set_busy.clone());
                    background(move || plan.apply(), move |result| {
                        if let Some(message) = status.upgrade() {
                            match result {
                                Ok(id) => {
                                    message.set_text(&format!("Selected {id}. Running sessions were not restarted."));
                                    if let Some(refresh) = refresh.upgrade() { refresh.emit_clicked(); }
                                }
                                Err(error) => message.set_text(&format!("Selection failed: {error:#}")),
                            }
                        }
                        set_busy(false);
                    });
                });
                dialog.present();
            });
        })
    };
    {
        let request = request.clone();
        let (target, historical) = (target.clone(), historical.clone());
        activate.connect_clicked(move |_| request(Some(target.text().trim().to_owned()), historical.is_active()));
    }
    rollback.connect_clicked(move |_| request(None, false));
    {
        let status = status.downgrade();
        let set_busy = set_busy.clone();
        verify.connect_clicked(move |_| {
            let id = target.text().trim().to_owned();
            if let Some(message) = status.upgrade() { message.set_text("Hashing deployment artifacts…"); }
            set_busy(true);
            let (status, set_busy) = (status.clone(), set_busy.clone());
            background(move || selection::verify_identity(&id), move |result| {
                if let Some(message) = status.upgrade() {
                    match result {
                        Ok(()) => message.set_text("All recorded artifact hashes verified. This is an integrity check, not publisher authentication."),
                        Err(error) => message.set_text(&format!("Verification failed: {error:#}")),
                    }
                }
                set_busy(false);
            });
        });
    }
    container
}
