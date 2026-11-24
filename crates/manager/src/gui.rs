//! GTK frontend for the shared manager backend. Only GTK objects live on the UI thread.
use anyhow::{bail, Result};
use gtk::{gio, glib, prelude::*};
use std::{sync::mpsc, thread, time::Duration};
use crate::{acquire, catalog, packages, selection, trust};
mod setup;
mod deployment;
mod updates;
mod maintenance;
mod rices;
mod snapshots;
mod live_preview;

#[derive(Clone, Copy, Debug)]
enum Page { Overview, Products, Deployments, Recovery, Storage, Security }

const PAGES: &[(Page, &str, &str, &str)] = &[
    (Page::Overview, "overview", "Overview", "Your host system and current Atelier deployment."),
    (Page::Products, "products", "Products", "Native components and supported external desktops. Detection does not establish runtime compatibility."),
    (Page::Deployments, "deployments", "Deployments", "Staged bundles, active selection and rollback target. Inventory checks structure and size only."),
    (Page::Recovery, "recovery", "Recovery", "Verify selected deployments and inspect historical system package transactions. No automatic repair."),
    (Page::Storage, "storage", "Storage", "Deployments, protection reasons, cached releases and logical sizes. Removal requires explicit approval."),
    (Page::Security, "security", "Publisher trust", "Locally pinned and revoked publisher keys. No key is trusted automatically."),
];

struct Row { title: String, status: String, detail: String }
impl Row {
    fn new(title: impl Into<String>, status: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { title: title.into(), status: status.into(), detail: detail.into() }
    }
    fn error(title: &str, error: anyhow::Error) -> Self { Self::new(title, "Unavailable", format!("{error:#}")) }
}

fn rows(page: Page) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    match page {
        Page::Overview => {
            let report = crate::report();
            rows.push(Row::new("Operating system", report.distribution.id.as_deref().unwrap_or("Unknown"),
                report.distribution_error.unwrap_or_else(|| format!("Architecture: {}", report.architecture))));
            rows.push(Row::new("Desktop session", report.desktop.as_deref().unwrap_or("Unknown"),
                report.session_type.as_deref().unwrap_or("Session type not reported")));
            match selection::snapshot() {
                Ok(inventory) => {
                    rows.push(Row::new("Selected deployment", inventory.current.as_deref().unwrap_or("None"),
                        format!("{} known bundles; rollback: {}", inventory.deployments.len(), inventory.previous.as_deref().unwrap_or("none"))));
                }
                Err(e) => rows.push(Row::error("Deployments", e)),
            }
            rows.push(Row::new("Manager version", env!("CARGO_PKG_VERSION"), "Experimental native desktop; physical hardware acceptance is outstanding."));
        }
        Page::Products => {
            let report = crate::report();
            for product in catalog::PRODUCTS {
                let detected = product.binaries.iter().filter(|name| report.products.iter().chain(report.compositors.iter())
                    .any(|item| item.name == **name && item.path.is_some())).count();
                let status = if product.binaries.is_empty() { "Bundle".into() }
                    else { format!("{detected}/{} executables detected", product.binaries.len()) };
                rows.push(Row::new(product.name, status, format!("{}\nSource: {} · ID: {}", product.note, product.source, product.id)));
            }
        }
        Page::Deployments => {
            let inventory = selection::snapshot()?;
            for entry in inventory.deployments {
                let details = match entry.manifest {
                    Some(m) => {
                        let origin = m.release.as_ref().map(|r| format!("Release {} · sequence {}", r.version, r.sequence))
                            .unwrap_or_else(|| "No signed release provenance in this record".into());
                        format!("{}\n{}\n{}", entry.resolved_products.join(", "), origin, entry.status)
                    }
                    None => entry.error.unwrap_or_else(|| "Invalid deployment".into()),
                };
                rows.push(Row::new(entry.id, entry.role, details));
            }
        }
        Page::Recovery => {
            match selection::recovery() {
                Ok(report) => {
                    for (slot, title) in [("current", "Current deployment"), ("previous", "Rollback deployment")] {
                        let entry = &report[slot];
                        let detail = format!("{}\n{}", entry["id"].as_str().unwrap_or("None"), entry["error"].as_str().unwrap_or(""));
                        rows.push(Row::new(title, entry["status"].as_str().unwrap_or("Unknown"), detail));
                    }
                }
                Err(error) => rows.push(Row::error("Deployment recovery", error)),
            }
            match packages::history() {
                Ok(entries) => {
                    if entries.is_empty() { rows.push(Row::new("Package transactions", "No records", "No historical package checkpoints found.")); }
                    for entry in entries {
                        rows.push(Row::new(entry.id, entry.status, format!("{}\nBefore inventory: {} · after inventory: {}\n{}",
                            entry.targets.join(", "), entry.before_inventory, entry.after_inventory, entry.error.unwrap_or_default())));
                    }
                }
                Err(error) => rows.push(Row::error("Package transactions", error)),
            }
        }
        Page::Storage => {
            match selection::cleanup_snapshot() {
                Ok(report) => {
                    if let Some(entries) = report["deployments"].as_array() {
                        for entry in entries {
                            let id = entry["id"].as_str().unwrap_or("Unknown");
                            let size = entry["bytes"].as_u64().map(|n| format!("{:.2} MiB logical artifact size", n as f64 / 1_048_576.0)).unwrap_or_else(|| "Size unknown".into());
                            rows.push(Row::new(id, entry["protection"].as_str().unwrap_or("Unknown"), format!("{size}\nRuntime: {}", entry["runtime"].as_str().unwrap_or("unknown"))));
                        }
                    }
                }
                Err(e) => rows.push(Row::error("Deployment storage", e)),
            }
            for entry in acquire::cache_snapshot()? {
                let detail = entry.bytes.map(|n| format!("{:.2} MiB logical size", n as f64 / 1_048_576.0))
                    .or(entry.error).unwrap_or_else(|| "Size unknown".into());
                rows.push(Row::new(entry.id, entry.status, detail));
            }
        }
        Page::Security => {
            for (id, key) in trust::snapshot()?.keys {
                rows.push(Row::new("Publisher key", if key.revoked { "Revoked" } else { "Pinned" }, id));
            }
        }
    }
    Ok(rows)
}

pub(super) fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.add_css_class(class);
    label
}

fn display_rows(list: &gtk::ListBox, rows: Vec<Row>) {
    while let Some(child) = list.first_child() { list.remove(&child); }
    for row in rows {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        card.set_margin_top(12); card.set_margin_bottom(12);
        card.set_margin_start(16); card.set_margin_end(16);
        card.append(&label(&row.title, "heading"));
        let status = label(&row.status, "dim-label");
        status.set_selectable(true);
        card.append(&status);
        let detail = label(&row.detail, "body");
        detail.set_selectable(true);
        card.append(&detail);
        list.append(&card);
    }
}

pub(super) fn background<T: Send + 'static>(work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    mut finish: impl FnMut(anyhow::Result<T>) + 'static) {
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || { let _ = tx.send(work()); });
    glib::timeout_add_local(Duration::from_millis(50), move || {
        match rx.try_recv() {
            Ok(result) => finish(result),
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => finish(Err(anyhow::anyhow!("setup worker stopped unexpectedly"))),
        }
        glib::ControlFlow::Break
    });
}

fn refresh(page: Page, list: &gtk::ListBox, status: &gtk::Label, button: &gtk::Button) {
    button.set_sensitive(false);
    status.set_text("Checking local state…");
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || { let _ = sender.send(rows(page)); });
    let (list, status, button) = (list.downgrade(), status.downgrade(), button.downgrade());
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let (Some(list), Some(status), Some(button)) = (list.upgrade(), status.upgrade(), button.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        match receiver.try_recv() {
            Ok(Ok(rows)) => {
                status.set_text(if rows.is_empty() { "No records found." } else { "Local snapshot refreshed." });
                display_rows(&list, rows);
            }
            Ok(Err(error)) => {
                display_rows(&list, vec![Row::error("Data unavailable", error)]);
                status.set_text("Refresh failed.");
            }
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                display_rows(&list, vec![Row::new("Data unavailable", "Refresh failed", "Background inspection stopped unexpectedly.")]);
                status.set_text("Refresh failed.");
            }
        }
        button.set_sensitive(true);
        glib::ControlFlow::Break
    });
}

fn build_window(app: &gtk::Application) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder().application(app).title("Atelier · Manager")
        .default_width(900).default_height(620).build();
    let header = gtk::HeaderBar::new();
    let title = gtk::Box::new(gtk::Orientation::Vertical, 1);
    title.append(&label("Atelier Manager", "title-4"));
    title.append(&label("Ecosystem management · Preview", "dim-label"));
    header.set_title_widget(Some(&title));
    window.set_titlebar(Some(&header));
    let layout = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let stack = gtk::Stack::new();
    stack.set_hexpand(true); stack.set_vexpand(true);
    let sidebar = gtk::StackSidebar::new();
    sidebar.set_stack(&stack); sidebar.set_size_request(175, -1);
    layout.append(&sidebar);
    layout.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    layout.append(&stack);
    for &(page, id, name, description) in PAGES {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
        content.set_margin_top(24); content.set_margin_bottom(24);
        content.set_margin_start(24); content.set_margin_end(24);
        content.append(&label(name, "title-1"));
        content.append(&label(description, "dim-label"));
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let status = label("Select Refresh to inspect local state.", "dim-label");
        status.set_hexpand(true);
        let button = gtk::Button::with_label("Refresh");
        toolbar.append(&status); toolbar.append(&button);
        content.append(&toolbar);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).build();
        content.append(&scroll);
        if matches!(page, Page::Deployments | Page::Recovery) {
            content.append(&deployment::controls(&window, &button, matches!(page, Page::Recovery)));
        }
        if matches!(page, Page::Security | Page::Storage) {
            content.append(&maintenance::controls(&window, &button, matches!(page, Page::Security)));
        }
        let (list_copy, status_copy) = (list.clone(), status.clone());
        button.connect_clicked(move |button| refresh(page, &list_copy, &status_copy, button));
        stack.add_titled(&content, Some(id), name);
        // Load once when the page is first selected; later refresh is explicit.
        let loaded = std::cell::Cell::new(false);
        let weak = button.downgrade();
        stack.connect_visible_child_name_notify(move |stack| {
            if stack.visible_child_name().as_deref() == Some(id) && !loaded.replace(true) {
                if let Some(button) = weak.upgrade() { button.emit_clicked(); }
            }
        });
        if id == "overview" { button.emit_clicked(); }
    }
    stack.add_titled(&setup::build(), Some("setup"), "Installation");
    stack.add_titled(&updates::build(), Some("updates"), "Updates");
    stack.add_titled(&rices::build(), Some("rices"), "Rices");
    stack.add_titled(&snapshots::build(), Some("snapshots"), "Rice backups");
    stack.add_titled(&live_preview::build(), Some("live-preview"), "Live rice preview");
    window.set_child(Some(&layout));
    window
}

pub fn run() -> Result<()> {
    if nix::unistd::Uid::effective().is_root() { bail!("run graphical manager as your desktop user, not root"); }
    let app = gtk::Application::builder().application_id("org.atelier.Manager")
        .flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_activate(|app| build_window(app).present());
    if app.run_with_args(&["atelier-manager"]) != glib::ExitCode::SUCCESS { bail!("graphical manager exited unsuccessfully"); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn product_page_contains_the_shared_catalog() {
        let products = rows(Page::Products).unwrap();
        assert_eq!(products.len(), catalog::PRODUCTS.len());
        assert!(products.iter().any(|r| r.title == "Atelier Compositor"));
        assert!(products.iter().any(|r| r.title == "Mango Atelier"));
    }

    #[test]
    #[ignore = "requires a graphical test display"]
    fn gtk_window_smoke() {
        gtk::init().unwrap();
        let app = gtk::Application::builder().application_id("org.atelier.Manager.Test")
            .flags(gio::ApplicationFlags::NON_UNIQUE).build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let window = build_window(&app);
        assert_eq!(window.title().as_deref(), Some("Atelier · Manager"));
        assert!(window.child().is_some());
        window.present();
        let context = glib::MainContext::default();
        for _ in 0..20 { if !context.pending() { break; } context.iteration(false); }
        window.close();
    }
}
