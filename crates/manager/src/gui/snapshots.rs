//! Portable per-rice configuration backups. All archive policy belongs to Mango Atelier.
use gtk::prelude::*;
use std::path::PathBuf;
use crate::rice::{self, SnapshotAction};
use super::{background, display_rows, label, Row};

fn chosen(combo: &gtk::ComboBoxText) -> Option<SnapshotAction> {
    Some(match combo.active_id()?.as_str() {
        "list" => SnapshotAction::List, "inspect" => SnapshotAction::Inspect,
        "create" => SnapshotAction::Create, "import" => SnapshotAction::Import,
        "export" => SnapshotAction::Export, "rename" => SnapshotAction::Rename,
        "restore" => SnapshotAction::Restore, "delete" => SnapshotAction::Delete,
        _ => return None,
    })
}
fn explain(action: SnapshotAction) -> &'static str {
    match action {
        SnapshotAction::List => "Browse validated local rice snapshots.",
        SnapshotAction::Inspect => "Inspect file names, sizes and checksums before restoring or sharing.",
        SnapshotAction::Create => "Enter a registered rice ID and snapshot name. Captures its allowed configuration only.",
        SnapshotAction::Import => "Enter an absolute ZIP path. The archive is validated and repackaged; settings are not restored.",
        SnapshotAction::Export => "Enter a snapshot ID and new absolute ZIP path. Existing files are not overwritten. Archives may contain private settings or images.",
        SnapshotAction::Rename => "Enter a snapshot ID and new label.",
        SnapshotAction::Restore => "Inspect the snapshot first. Close ALL Mango Atelier sessions and previews, including other logins. Run from a separate desktop. Restoring modifies scoped settings and creates a recovery snapshot.",
        SnapshotAction::Delete => "Permanently remove only this snapshot. The rice installation and current settings remain unchanged.",
    }
}
fn entry(form: &gtk::Box, placeholder: &str) -> gtk::Entry {
    let field = gtk::Entry::new();
    field.set_placeholder_text(Some(placeholder)); form.append(&field); field
}

pub(super) fn build() -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    outer.set_margin_top(24); outer.set_margin_bottom(24); outer.set_margin_start(24); outer.set_margin_end(24);
    outer.append(&label("Rice backups", "title-1"));
    outer.append(&label("Portable per-rice settings archives. These are separate from native deployments and system package recovery.", "dim-label"));
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    outer.append(&form);
    let path = entry(&form, "Absolute path to trusted Mango Atelier checkout");
    let trust = gtk::CheckButton::with_label("I trust this checkout and approve executing its backup manager.");
    form.append(&trust);
    { let trust = trust.downgrade(); path.connect_changed(move |_| { if let Some(trust) = trust.upgrade() { trust.set_active(false); } }); }
    let action = gtk::ComboBoxText::new();
    for (id, name) in [("list","List snapshots"),("inspect","Inspect manifest"),("create","Create snapshot"),("import","Import archive"),("export","Export archive"),("rename","Rename snapshot"),("restore","Restore offline"),("delete","Delete snapshot")] {
        action.append(Some(id),name);
    }
    action.set_active_id(Some("list")); form.append(&action);
    let guidance = label(explain(SnapshotAction::List), "dim-label");
    form.append(&guidance);
    let id = entry(&form, "Snapshot ID or rice ID for creation");
    let value = entry(&form, "Snapshot name or absolute ZIP path");
    let confirmation = entry(&form, "Typed confirmation for changes");
    let required = label("Listing does not modify settings.", "dim-label");
    required.set_selectable(true); form.append(&required);
    let run = gtk::Button::with_label("Run requested operation");
    form.append(&run);
    let status = label("No checkout executed.", "dim-label"); status.set_selectable(true);
    outer.append(&status);
    let list = gtk::ListBox::new(); list.add_css_class("boxed-list"); list.set_selection_mode(gtk::SelectionMode::None);
    outer.append(&gtk::ScrolledWindow::builder().child(&list).vexpand(true).min_content_height(100).build());

    // A change to any reviewed input clears typed approval.
    let sync = {
        let (action, id, value, confirmation, required, guidance) = (action.downgrade(), id.downgrade(), value.downgrade(), confirmation.downgrade(), required.downgrade(), guidance.downgrade());
        std::rc::Rc::new(move || {
            let (Some(action), Some(id), Some(value), Some(confirmation), Some(required), Some(guidance)) =
                (action.upgrade(), id.upgrade(), value.upgrade(), confirmation.upgrade(), required.upgrade(), guidance.upgrade()) else { return; };
            let Some(a) = chosen(&action) else { return; };
            confirmation.set_text("");
            id.set_visible(!matches!(a, SnapshotAction::List | SnapshotAction::Import));
            value.set_visible(matches!(a, SnapshotAction::Create | SnapshotAction::Import | SnapshotAction::Export | SnapshotAction::Rename));
            confirmation.set_visible(!matches!(a, SnapshotAction::List | SnapshotAction::Inspect));
            guidance.set_text(explain(a));
            match a.phrase(id.text().trim(), value.text().trim()) {
                Ok(Some(p)) => required.set_text(&format!("Type exactly: {p}")),
                Ok(None) => required.set_text("Inspection only."),
                Err(e) => required.set_text(&format!("{e:#}")),
            }
        })
    };
    { let sync = sync.clone(); action.connect_changed(move |_| sync()); }
    { let sync = sync.clone(); id.connect_changed(move |_| sync()); }
    { let sync = sync.clone(); value.connect_changed(move |_| sync()); }
    { let sync = sync.clone(); path.connect_changed(move |_| sync()); }
    { let sync = sync.clone(); trust.connect_toggled(move |_| sync()); }
    sync();

    let (form, status, list, path, trust, action, id, value, confirmation) =
        (form.downgrade(), status.downgrade(), list.downgrade(), path.downgrade(), trust.downgrade(), action.downgrade(), id.downgrade(), value.downgrade(), confirmation.downgrade());
    run.connect_clicked(move |_| {
        let (Some(form_widget), Some(message), Some(list_widget), Some(path), Some(trust), Some(action), Some(id), Some(value), Some(confirmation)) =
            (form.upgrade(), status.upgrade(), list.upgrade(), path.upgrade(), trust.upgrade(), action.upgrade(), id.upgrade(), value.upgrade(), confirmation.upgrade()) else { return; };
        if !trust.is_active() { message.set_text("Confirm trust in the checkout first."); return; }
        let Some(action) = chosen(&action) else { return; };
        let id = id.text().trim().to_owned();
        let value = value.text().trim().to_owned();
        let typed = confirmation.text().to_string();
        match action.phrase(&id,&value) {
            Ok(Some(expected)) if expected != typed => { message.set_text(&format!("Type exactly: {expected}")); return; }
            Err(e) => { message.set_text(&format!("{e:#}")); return; }
            _ => {}
        }
        let path = PathBuf::from(path.text().trim());
        confirmation.set_text("");
        form_widget.set_sensitive(false);
        display_rows(&list_widget, vec![]);
        message.set_text("Running delegated snapshot operation…");
        let (form,status,list) = (form.clone(),status.clone(),list.clone());
        background(move || rice::snapshots(&path,action,&id,&value,&typed), move |result| {
            let (Some(form),Some(status),Some(list)) = (form.upgrade(),status.upgrade(),list.upgrade()) else { return; };
            match result {
                Ok(items) => {
                    status.set_text(if items.is_empty() { "No validated snapshots found." } else { "Operation completed. Review the result; list snapshots again to refresh." });
                    display_rows(&list,items.into_iter().map(|item| Row::new(item.title,item.status,item.detail)).collect());
                }
                Err(e) => { status.set_text("Operation failed. Inspect error and recovery state before retrying."); display_rows(&list,vec![Row::error("Snapshot operation",e)]); }
            }
            form.set_sensitive(true);
        });
    });
    outer
}
