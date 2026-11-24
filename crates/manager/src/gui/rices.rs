//! Rice Manager is an optional delegated capability, not a native compositor dependency.
use gtk::prelude::*;
use std::path::PathBuf;
use crate::rice::{self, Action};
use super::{background, display_rows, label, Row};

pub(super) fn build() -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    outer.set_margin_top(24); outer.set_margin_bottom(24); outer.set_margin_start(24); outer.set_margin_end(24);
    outer.append(&label("Rice Manager", "title-1"));
    outer.append(&label("Optional Mango Atelier integration. Use only a trusted local checkout; all delegated operations execute its code as your user. Native Atelier deployments are independent.", "dim-label"));
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    outer.append(&form);
    let path = gtk::Entry::new();
    path.set_placeholder_text(Some("Absolute path to Mango Atelier checkout"));
    form.append(&path);
    let trust = gtk::CheckButton::with_label("I trust this checkout and approve running its Rice Manager.");
    form.append(&trust);
    { let trust = trust.downgrade(); path.connect_changed(move |_| { if let Some(trust) = trust.upgrade() { trust.set_active(false); } }); }
    let target = gtk::ComboBoxText::new();
    for (id, name) in [("mango", "MangoWM"), ("hyprland", "Hyprland"), ("niri", "Niri")] { target.append(Some(id), name); }
    form.append(&label("Target compositor (offline configuration)", "body"));
    form.append(&target);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let catalog = gtk::Button::with_label("Local catalog");
    let discover = gtk::Button::with_label("Discover online");
    let snapshots = gtk::Button::with_label("Snapshots");
    actions.append(&catalog); actions.append(&discover); actions.append(&snapshots);
    form.append(&actions);
    let id = gtk::Entry::new();
    id.set_placeholder_text(Some("Rice ID from local catalog"));
    form.append(&id);
    let phrase = gtk::Entry::new();
    phrase.set_placeholder_text(Some("Typed confirmation"));
    form.append(&label("Prepare: type prepare sekaipop for TARGET. Select: type select ID for TARGET. Preparation may fetch the pinned source; selection affects next login, not the running shell. Custom rices execute user-trusted code at login.", "dim-label"));
    form.append(&phrase);
    let operations = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let prepare = gtk::Button::with_label("Prepare SekaiPop");
    let select = gtk::Button::with_label("Select for next login");
    operations.append(&prepare); operations.append(&select); form.append(&operations);
    let status = label("No checkout executed.", "dim-label"); status.set_selectable(true);
    outer.append(&status);
    let list = gtk::ListBox::new(); list.add_css_class("boxed-list"); list.set_selection_mode(gtk::SelectionMode::None);
    let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).min_content_height(120).build();
    outer.append(&scroll);
    for (button, action) in [(catalog,Action::Local),(discover,Action::Discover),(snapshots,Action::Snapshots),(prepare,Action::Prepare),(select,Action::Select)] {
        let (form, status, list, path, trust, target, id, phrase) = (form.downgrade(), status.downgrade(), list.downgrade(), path.downgrade(), trust.downgrade(), target.downgrade(), id.downgrade(), phrase.downgrade());
        button.connect_clicked(move |_| {
            let (Some(form_widget), Some(message), Some(list_widget), Some(path), Some(trust), Some(target), Some(id), Some(phrase)) =
                (form.upgrade(), status.upgrade(), list.upgrade(), path.upgrade(), trust.upgrade(), target.upgrade(), id.upgrade(), phrase.upgrade()) else { return; };
            if !trust.is_active() { message.set_text("Confirm trust in the local checkout first."); return; }
            let Some(target) = target.active_id().map(|s| s.to_string()) else { message.set_text("Select a target compositor."); return; };
            let id = id.text().trim().to_owned();
            let expected = match action {
                Action::Prepare => Some(format!("prepare sekaipop for {target}")),
                Action::Select => Some(format!("select {id} for {target}")),
                _ => None,
            };
            if let Some(expected) = expected {
                if phrase.text().as_str() != expected { message.set_text(&format!("Type exactly: {expected}")); return; }
            }
            phrase.set_text("");
            let id = if matches!(action, Action::Prepare) { "sekaipop".into() } else { id };
            let path = PathBuf::from(path.text().trim());
            form_widget.set_sensitive(false);
            display_rows(&list_widget, vec![]);
            message.set_text("Running the explicitly requested Mango Atelier operation…");
            let (form, status, list) = (form.clone(), status.clone(), list.clone());
            background(move || rice::execute(&path, &target, action, &id), move |result| {
                let (Some(form), Some(status), Some(list)) = (form.upgrade(), status.upgrade(), list.upgrade()) else { return; };
                match result {
                    Ok(items) => {
                        status.set_text(if items.is_empty() { "No records found." } else { "Delegated operation completed. Inspect results and refresh the catalog after changes." });
                        display_rows(&list, items.into_iter().map(|item| Row::new(item.title,item.status,item.detail)).collect());
                    }
                    Err(e) => { status.set_text("Operation failed. Inspect the delegated error."); display_rows(&list, vec![Row::error("Mango Atelier",e)]); }
                }
                form.set_sensitive(true);
            });
        });
    }
    outer
}
