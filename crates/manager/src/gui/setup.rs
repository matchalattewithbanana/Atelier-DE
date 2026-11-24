//! Graphical setup review. Mutation is restricted to explicitly approved local staging.
use gtk::{glib, prelude::*};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::mpsc, thread, time::Duration};
use crate::{catalog, deploy, wizard};
use super::label;

struct Fields {
    products: Vec<(&'static str, gtk::CheckButton)>,
    providers: Vec<(&'static str, gtk::CheckButton)>,
    rices: Vec<(&'static str, gtk::Entry)>,
    shell: gtk::Entry,
    session: gtk::ComboBoxText,
    source: gtk::Entry,
}

impl Fields {
    fn choices(&self) -> anyhow::Result<wizard::Choices> {
        let ids: Vec<String> = self.products.iter().filter(|(_, b)| b.is_active()).map(|(id, _)| (*id).into()).collect();
        let resolved = catalog::resolve(&ids)?;
        let mango = ids.iter().any(|id| id == "mango-atelier");
        let rices = self.rices.iter().filter(|(id, _)| mango && ids.iter().any(|selected| selected.as_str() == *id))
            .map(|(id, entry)| wizard::RiceChoice { compositor: (*id).into(), rice: entry.text().trim().to_owned() }).collect();
        let has_session = resolved.iter().any(|p| p.id == "session");
        let has_any_session = has_session || ids.iter().any(|id| wizard::EXTERNAL.contains(&id.as_str()));
        Ok(wizard::Choices {
            mode: "custom".into(), ids, rices,
            native_shell: has_session.then(|| self.shell.text().trim().to_owned()),
            optional_providers: self.providers.iter().filter(|(_, b)| b.is_active()).map(|(id, _)| (*id).into()).collect(),
            first_session: has_any_session.then(|| self.session.active_id().map(|s| s.to_string()).unwrap_or_else(|| "keep".into())),
        })
    }
}

struct Review { text: String, stage: Option<deploy::StagePlan> }
fn review(choices: wizard::Choices, source: Option<PathBuf>) -> anyhow::Result<Review> {
    let preview = wizard::build_preview(choices, &crate::report())?;
    let mut text = Vec::new();
    preview.write_review(&mut text)?;
    let mut text = String::from_utf8(text)?;
    let stage = if let Some(source) = source {
        match deploy::review_local(&source, preview.requested_products()) {
            Ok((plan, detail)) => {
                text.push_str("\nLocal staging (separate confirmation)\n");
                text.push_str(&detail);
                text.push_str("Only native binaries are staged. Package dependencies, providers, rice, session and shell preferences are not applied.\n");
                Some(plan)
            }
            Err(error) => {
                text.push_str(&format!("\nLocal staging unavailable: {error:#}\n"));
                None
            }
        }
    } else { None };
    text.push_str("\nSystem package changes and Mango rice application require their separate CLI workflows. No sudo or network actions are triggered here.");
    Ok(Review { text, stage })
}

fn section(container: &gtk::Box, title: &str, description: &str) {
    container.append(&label(title, "title-3"));
    container.append(&label(description, "dim-label"));
}
fn entry(container: &gtk::Box, name: &str, value: &str) -> gtk::Entry {
    let field = gtk::Entry::new();
    field.set_text(value);
    let caption = label(name, "body");
    caption.set_mnemonic_widget(Some(&field));
    container.append(&caption);
    container.append(&field);
    field
}

// Only data is moved between threads. Weak references keep closed windows
// from being retained by outstanding inspections or copy operations.
fn background<T: Send + 'static>(work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
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

pub(super) fn build() -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    outer.set_margin_top(24); outer.set_margin_bottom(24);
    outer.set_margin_start(24); outer.set_margin_end(24);
    outer.append(&label("Installation", "title-1"));
    outer.append(&label("Select components, review the dependency plan, and optionally stage a trusted local native build.", "dim-label"));
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    outer.append(&stack);
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let presets = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    section(&form, "1. Products and compositors", "Presets change selections only. External compositors are optional.");
    form.append(&presets);
    let mut products = Vec::new();
    for product in catalog::PRODUCTS.iter().filter(|p| p.id != "desktop") {
        let button = gtk::CheckButton::with_label(product.name);
        button.set_active(matches!(product.id, "shell" | "settings"));
        button.set_tooltip_text(Some(product.note));
        form.append(&button);
        products.push((product.id, button));
    }
    for (title, mode) in [("Native desktop", "native"), ("Minimal", "minimal"), ("Clear", "clear")] {
        let button = gtk::Button::with_label(title);
        let items = products.clone();
        button.connect_clicked(move |_| {
            for (id, field) in &items {
                let active = match mode {
                    "native" => catalog::PRODUCTS.iter().any(|p| p.id == *id && p.source == "atelier-de"),
                    "minimal" => matches!(*id, "shell" | "settings"),
                    _ => false,
                };
                field.set_active(active);
            }
        });
        presets.append(&button);
    }
    section(&form, "2. Appearance and session", "Rice IDs are used only with Mango Atelier and the corresponding selected compositor. Custom rices require Mango validation.");
    let rices = wizard::EXTERNAL.iter().map(|id| (*id, entry(&form, &format!("Rice for {id} (keep, atelier, serpantinum, custom; sekaipop except Niri)"), "keep"))).collect();
    let shell = entry(&form, "Native shell (default, none, profile:NAME)", "default");
    let session = gtk::ComboBoxText::new();
    for (id, name) in [("keep","Keep current"),("atelier","Atelier"),("mangowm","MangoWM"),("hyprland","Hyprland"),("niri","Niri")] {
        session.append(Some(id), name);
    }
    session.set_active_id(Some("keep"));
    form.append(&label("First session preference (must be selected)", "body"));
    form.append(&session);
    section(&form, "3. Optional providers", "Preview dependencies; no system services are enabled.");
    let providers = wizard::PROVIDERS.iter().map(|id| {
        let button = gtk::CheckButton::with_label(id);
        form.append(&button);
        (*id, button)
    }).collect();
    section(&form, "4. Local native build (optional)", "Only for a native-only selection. Leave empty for a plan without staging. Use an independently trusted build directory.");
    let source = entry(&form, "Absolute path to build directory", "");
    let fields = Rc::new(Fields { products, providers, rices, shell, session, source });
    let form_scroll = gtk::ScrolledWindow::builder().child(&form).vexpand(true).build();
    stack.add_named(&form_scroll, Some("choices"));
    let review_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let review_text = label("", "body");
    review_text.set_selectable(true);
    review_box.append(&review_text);
    let approve = gtk::CheckButton::with_label("I trust this local build and approve copying the reviewed bytes into a new deployment.");
    approve.set_visible(false);
    review_box.append(&approve);
    let review_scroll = gtk::ScrolledWindow::builder().child(&review_box).vexpand(true).build();
    stack.add_named(&review_scroll, Some("review"));
    stack.set_visible_child_name("choices");

    let status = label("No changes have been made.", "dim-label");
    outer.append(&status);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let edit = gtk::Button::with_label("Back to choices");
    edit.set_sensitive(false);
    let preview = gtk::Button::with_label("Review setup");
    preview.add_css_class("suggested-action");
    let stage = gtk::Button::with_label("Stage reviewed build");
    stage.set_sensitive(false);
    actions.append(&edit); actions.append(&preview); actions.append(&stage);
    outer.append(&actions);
    let plan: Rc<RefCell<Option<deploy::StagePlan>>> = Rc::new(RefCell::new(None));

    let weak_stage = stage.downgrade();
    approve.connect_toggled(move |check| {
        if let Some(stage) = weak_stage.upgrade() { stage.set_sensitive(check.is_active()); }
    });
    {
        let (stack, status, preview, stage, approve, plan) = (stack.clone(), status.clone(), preview.clone(), stage.clone(), approve.clone(), plan.clone());
        edit.connect_clicked(move |edit| {
            plan.borrow_mut().take();
            approve.set_active(false); approve.set_visible(false);
            stage.set_sensitive(false);
            stack.set_visible_child_name("choices");
            status.set_text("Edit selections and review again.");
            preview.set_sensitive(true); edit.set_sensitive(false);
        });
    }
    {
        let (stack, status, text, stage, approve, edit, plan) = (stack.clone(), status.clone(), review_text.clone(), stage.clone(), approve.clone(), edit.clone(), plan.clone());
        preview.connect_clicked(move |preview| {
            let choices = match fields.choices() {
                Ok(choices) => choices,
                Err(error) => { status.set_text(&format!("{error:#}")); return; }
            };
            let source = fields.source.text().trim().to_owned();
            let source = (!source.is_empty()).then(|| PathBuf::from(source));
            preview.set_sensitive(false); edit.set_sensitive(false);
            approve.set_active(false); stage.set_sensitive(false);
            status.set_text("Checking dependency plan and local artifacts…");
            text.set_text("");
            stack.set_visible_child_name("review");
            let (status, text, approve, edit) = (status.downgrade(), text.downgrade(), approve.downgrade(), edit.downgrade());
            let plan = plan.clone();
            background(move || review(choices, source), move |result| {
                let (Some(status), Some(text), Some(approve), Some(edit)) = (status.upgrade(), text.upgrade(), approve.upgrade(), edit.upgrade()) else { return; };
                match result {
                    Ok(review) => {
                        text.set_text(&review.text);
                        approve.set_visible(review.stage.is_some());
                        *plan.borrow_mut() = review.stage;
                        status.set_text("Review complete. Nothing installed.");
                    }
                    Err(error) => {
                        text.set_text(&format!("{error:#}"));
                        status.set_text("Review failed. Return to choices.");
                    }
                }
                edit.set_sensitive(true);
            });
        });
    }
    {
        let (status, text, edit, approve, plan) = (status.clone(), review_text.clone(), edit.clone(), approve.clone(), plan.clone());
        stage.connect_clicked(move |stage| {
            if !approve.is_active() { return; }
            let Some(reviewed) = plan.borrow_mut().take() else { return; };
            stage.set_sensitive(false); approve.set_active(false); approve.set_visible(false);
            edit.set_sensitive(false);
            status.set_text("Staging verified local bytes…");
            let (status, text, edit) = (status.downgrade(), text.downgrade(), edit.downgrade());
            background(move || deploy::stage_reviewed(reviewed), move |result| {
                let (Some(status), Some(text), Some(edit)) = (status.upgrade(), text.upgrade(), edit.upgrade()) else { return; };
                match result {
                    Ok(path) => {
                        text.set_text(&format!("New deployment: {}\nNo activation performed. Refresh Deployments to inspect it.", path.display()));
                        status.set_text("Staging complete.");
                    }
                    Err(error) => { text.set_text(&format!("{error:#}")); status.set_text("Staging failed; review again before retrying."); }
                }
                edit.set_sensitive(true);
            });
        });
    }
    outer
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graphical_choices_share_backend_validation() {
        let choices = wizard::Choices {
            mode: "custom".into(), ids: vec!["mango-atelier".into(), "niri".into()],
            rices: vec![wizard::RiceChoice { compositor:"niri".into(), rice:"sekaipop".into() }],
            native_shell: None, optional_providers: vec![], first_session: Some("keep".into()),
        };
        assert!(review(choices, None).is_err());
        let choices = wizard::Choices {
            mode:"custom".into(), ids:vec!["shell".into(),"settings".into()],
            rices:vec![], native_shell:None, optional_providers:vec![], first_session:None,
        };
        let result = review(choices, None).unwrap();
        assert!(result.stage.is_none());
        assert!(result.text.contains("sudo: not used"));
    }
}
