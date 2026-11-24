//! Explicitly reviewed publisher and cache maintenance using the CLI backends.
use gtk::prelude::*;
use std::{cell::RefCell, rc::Rc, path::PathBuf};
use crate::{acquire, trust, selection};
use super::{background, label};

#[derive(Clone, Copy)]
enum Action { Pin, Revoke, RemoveCache, RemoveDeployment }
enum Review { Trust(trust::TrustReview), Cache(acquire::CacheRemoval), Deployment(selection::OfflineRemoval) }
impl Review {
    fn detail(&self) -> &str {
        match self { Self::Trust(r) => r.detail(), Self::Cache(r) => r.detail(), Self::Deployment(r) => r.detail() }
    }
    fn phrase(&self) -> &str {
        match self { Self::Trust(r) => r.phrase(), Self::Cache(_) => "remove cache", Self::Deployment(r) => r.phrase() }
    }
    fn apply(self) -> anyhow::Result<String> {
        match self { Self::Trust(r) => r.apply(), Self::Cache(r) => r.apply(), Self::Deployment(r) => r.apply() }
    }
}
fn review(action: Action, input: String) -> anyhow::Result<Review> {
    match action {
        Action::Pin => {
            let path = PathBuf::from(input);
            if !path.is_absolute() { anyhow::bail!("Use the absolute path to an independently verified public key file"); }
            Ok(Review::Trust(trust::review_pin(&path)?))
        }
        Action::Revoke => Ok(Review::Trust(trust::review_revoke(&input)?)),
        Action::RemoveCache => Ok(Review::Cache(acquire::review_remove_cache(&input)?)),
        Action::RemoveDeployment => Ok(Review::Deployment(selection::review_remove_offline(&input)?)),
    }
}

pub(super) fn controls(window: &gtk::ApplicationWindow, refresh: &gtk::Button, security: bool) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    outer.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    outer.append(&form);
    let status = label("Review and type the exact confirmation before making changes.", "dim-label");
    status.set_selectable(true);
    outer.append(&status);
    let actions = if security {
        vec![(Action::Pin, "Absolute path to Ed25519 public key", "Review pin"), (Action::Revoke, "Pinned key ID", "Review revocation")]
    } else { vec![(Action::RemoveCache, "Cache ID (release-...)", "Review cache removal"), (Action::RemoveDeployment, "Deployment ID (offline only)", "Review deployment removal")] };
    for (action, placeholder, caption) in actions {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(placeholder)); entry.set_hexpand(true);
        let button = gtk::Button::with_label(caption);
        line.append(&entry); line.append(&button); form.append(&line);
        let (form, status, window, refresh) = (form.downgrade(), status.downgrade(), window.downgrade(), refresh.downgrade());
        button.connect_clicked(move |_| {
            let (Some(form_widget), Some(message)) = (form.upgrade(), status.upgrade()) else { return; };
            let input = entry.text().trim().to_owned();
            if input.is_empty() { message.set_text("Enter a key path or ID first."); return; }
            form_widget.set_sensitive(false);
            message.set_text("Reviewing requested operation…");
            let (form, status, window, refresh) = (form.clone(), status.clone(), window.clone(), refresh.clone());
            background(move || review(action, input), move |result| {
                let (Some(form_widget), Some(message), Some(window)) = (form.upgrade(), status.upgrade(), window.upgrade()) else { return; };
                let plan = match result {
                    Ok(plan) => plan,
                    Err(error) => { message.set_text(&format!("Review failed: {error:#}")); form_widget.set_sensitive(true); return; }
                };
                let phrase = plan.phrase().to_owned();
                let dialog = gtk::Dialog::builder().title("Confirm maintenance operation")
                    .transient_for(&window).modal(true).default_width(580).build();
                dialog.add_button("Cancel", gtk::ResponseType::Cancel);
                let accept = dialog.add_button("Apply", gtk::ResponseType::Accept);
                accept.set_sensitive(false);
                dialog.set_default_response(gtk::ResponseType::Cancel);
                let area = dialog.content_area();
                area.set_spacing(12); area.set_margin_top(18); area.set_margin_bottom(18); area.set_margin_start(18); area.set_margin_end(18);
                let summary = label(plan.detail(), "body"); summary.set_selectable(true);
                area.append(&summary);
                area.append(&label(&format!("Type exactly: {phrase}"), "heading"));
                let confirmation = gtk::Entry::new(); area.append(&confirmation);
                let expected = phrase.clone();
                let accept = accept.downgrade();
                confirmation.connect_changed(move |entry| { if let Some(button) = accept.upgrade() { button.set_sensitive(entry.text().as_str() == expected); } });
                let plan = Rc::new(RefCell::new(Some(plan)));
                message.set_text("Awaiting explicit confirmation.");
                let (form, status, refresh) = (form.clone(), status.clone(), refresh.clone());
                dialog.connect_response(move |dialog, response| {
                    let review = plan.borrow_mut().take();
                    let approved = response == gtk::ResponseType::Accept && confirmation.text().as_str() == phrase;
                    dialog.close();
                    if !approved {
                        if let Some(message) = status.upgrade() { message.set_text("Cancelled. Nothing changed."); }
                        if let Some(form) = form.upgrade() { form.set_sensitive(true); }
                        return;
                    }
                    let Some(review) = review else { return; };
                    if let Some(message) = status.upgrade() { message.set_text("Applying reviewed operation…"); }
                    let (form, status, refresh) = (form.clone(), status.clone(), refresh.clone());
                    background(move || review.apply(), move |result| {
                        if let Some(message) = status.upgrade() {
                            match result {
                                Ok(text) => { message.set_text(&text); if let Some(refresh) = refresh.upgrade() { refresh.emit_clicked(); } }
                                Err(error) => message.set_text(&format!("Operation failed: {error:#}")),
                            }
                        }
                        if let Some(form) = form.upgrade() { form.set_sensitive(true); }
                    });
                });
                dialog.present();
            });
        });
    }
    outer
}
