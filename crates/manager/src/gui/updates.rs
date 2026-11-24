//! Explicit remote browsing, authenticated acquisition, and reviewed staging.
use gtk::prelude::*;
use std::{cell::RefCell, rc::Rc};
use crate::{acquire, discovery, trust};
use super::{background, label};

fn active_keys() -> anyhow::Result<Vec<String>> {
    Ok(trust::snapshot()?.keys.into_iter().filter(|(_, key)| !key.revoked).map(|(id, _)| id).collect())
}
fn selected_key(combo: &gtk::ComboBoxText) -> anyhow::Result<String> {
    combo.active_id().map(|id| id.to_string()).ok_or_else(|| anyhow::anyhow!("Select an independently pinned publisher key. Use the CLI to pin keys first."))
}
fn fill_keys(combo: &gtk::ComboBoxText, keys: Vec<String>) {
    let old = combo.active_id();
    combo.remove_all();
    for key in keys { combo.append(Some(&key), &key); }
    if let Some(old) = old { combo.set_active_id(Some(&old)); }
    // Never choose a publisher implicitly.
}
fn candidate_text(c: &discovery::Candidate) -> String {
    format!("{} · {} · {}", c.tag, if c.prerelease { "Prerelease" } else { "Stable" }, c.metadata_status)
}
pub(super) fn build() -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    outer.set_margin_top(24); outer.set_margin_bottom(24); outer.set_margin_start(24); outer.set_margin_end(24);
    outer.append(&label("Updates", "title-1"));
    outer.append(&label("Browse native releases, download with a pinned publisher, and review installation separately. Nothing is selected or installed automatically.", "dim-label"));
    let form = gtk::Box::new(gtk::Orientation::Vertical, 10);
    outer.append(&form);
    let channels = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let prerelease = gtk::CheckButton::with_label("Include prereleases");
    let browse = gtk::Button::with_label("Browse releases");
    channels.append(&prerelease); channels.append(&browse);
    form.append(&label("1. Remote candidates (not authenticated)", "title-3"));
    form.append(&channels);
    let releases = gtk::ComboBoxText::new();
    form.append(&releases);
    form.append(&label("2. Publisher (select an already pinned key)", "title-3"));
    let keys = gtk::ComboBoxText::new();
    let load_keys = gtk::Button::with_label("Refresh pinned keys");
    form.append(&keys); form.append(&load_keys);
    let fetch = gtk::Button::with_label("Authenticate and download selected release");
    form.append(&fetch);
    form.append(&label("3. Review a cached release", "title-3"));
    let cache = gtk::Entry::new();
    cache.set_placeholder_text(Some("Cache ID (release-...)"));
    form.append(&cache);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let full = gtk::Button::with_label("Review full bundle");
    let selective = gtk::Button::with_label("Review component update");
    actions.append(&full); actions.append(&selective);
    form.append(&actions);
    let status = label("No network request made.", "dim-label");
    status.set_selectable(true);
    outer.append(&status);
    let detail = label("", "body"); detail.set_selectable(true);
    let scroll = gtk::ScrolledWindow::builder().child(&detail).vexpand(true).min_content_height(110).build();
    outer.append(&scroll);
    let confirm = gtk::CheckButton::with_label("I approve acceptance and staging of the exact reviewed release.");
    confirm.set_visible(false);
    outer.append(&confirm);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let cancel = gtk::Button::with_label("Cancel review");
    let apply = gtk::Button::with_label("Stage reviewed release");
    apply.add_css_class("suggested-action");
    cancel.set_sensitive(false); apply.set_sensitive(false);
    footer.append(&cancel); footer.append(&apply); outer.append(&footer);
    let reviewed: Rc<RefCell<Option<acquire::CachedReview>>> = Rc::new(RefCell::new(None));

    // Changing the channel invalidates the displayed candidate list.
    { let releases = releases.clone(); prerelease.connect_toggled(move |_| releases.remove_all()); }
    {
        let (form, status, releases, prerelease) = (form.clone(), status.clone(), releases.clone(), prerelease.clone());
        browse.connect_clicked(move |_| {
            form.set_sensitive(false); releases.remove_all();
            status.set_text("Querying release candidates…");
            let pre = prerelease.is_active();
            let (form, status, releases) = (form.downgrade(), status.downgrade(), releases.downgrade());
            background(move || discovery::browse("native", pre), move |result| {
                let (Some(form), Some(status), Some(releases)) = (form.upgrade(), status.upgrade(), releases.upgrade()) else { return; };
                match result {
                    Ok(candidates) => {
                        for c in &candidates {
                            if c.metadata_status == "signature-not-verified" {
                                releases.append(Some(&c.id.to_string()), &candidate_text(c));
                            } else {
                                releases.append(None, &candidate_text(c));
                            }
                        }
                        status.set_text(&format!("{} candidates; API metadata only. Select a candidate with signature metadata.", candidates.len()));
                    }
                    Err(e) => status.set_text(&format!("Discovery failed: {e:#}")),
                }
                form.set_sensitive(true);
            });
        });
    }
    {
        let (form, status, keys) = (form.clone(), status.clone(), keys.clone());
        load_keys.connect_clicked(move |_| {
            form.set_sensitive(false); status.set_text("Reading publisher trust…");
            let (form, status, keys) = (form.downgrade(), status.downgrade(), keys.downgrade());
            background(active_keys, move |result| {
                let (Some(form), Some(status), Some(keys)) = (form.upgrade(), status.upgrade(), keys.upgrade()) else { return; };
                match result {
                    Ok(ids) => { status.set_text(if ids.is_empty() { "No active pinned keys. Pin a verified key with the CLI first." } else { "Select a pinned key explicitly." }); fill_keys(&keys, ids); }
                    Err(e) => { keys.remove_all(); status.set_text(&format!("Trust unavailable: {e:#}")); }
                }
                form.set_sensitive(true);
            });
        });
    }
    {
        let (form, status, keys, releases, prerelease, cache, detail) = (form.clone(), status.clone(), keys.clone(), releases.clone(), prerelease.clone(), cache.clone(), detail.clone());
        fetch.connect_clicked(move |_| {
            let result = (|| -> anyhow::Result<(u64, String)> {
                let id = releases.active_id().ok_or_else(|| anyhow::anyhow!("Select a candidate with signature metadata"))?.parse()?;
                Ok((id, selected_key(&keys)?))
            })();
            let (id, key) = match result { Ok(v) => v, Err(e) => { status.set_text(&format!("{e:#}")); return; } };
            let pre = prerelease.is_active();
            form.set_sensitive(false); detail.set_text(""); status.set_text("Authenticating and downloading to the isolated cache…");
            let (form, status, cache, detail) = (form.downgrade(), status.downgrade(), cache.downgrade(), detail.downgrade());
            background(move || acquire::fetch_release("native", id, &key, pre), move |result| {
                let (Some(form), Some(status), Some(cache), Some(detail)) = (form.upgrade(), status.upgrade(), cache.upgrade(), detail.upgrade()) else { return; };
                match result {
                    Ok(r) => {
                        if let Some(id) = r.cache.file_name().and_then(|n| n.to_str()) { cache.set_text(id); }
                        detail.set_text(&format!("Release {}\n{}\nCache: {}\nNo acceptance or activation performed.", r.version, r.status, r.cache.display()));
                        status.set_text("Verified cache ready. Review before staging.");
                    }
                    Err(e) => status.set_text(&format!("Acquisition failed: {e:#}")),
                }
                form.set_sensitive(true);
            });
        });
    }
    for (button, update) in [(full, false), (selective, true)] {
        let (form, status, keys, cache, detail, confirm, cancel, reviewed) = (form.clone(), status.clone(), keys.clone(), cache.clone(), detail.clone(), confirm.clone(), cancel.clone(), reviewed.clone());
        button.connect_clicked(move |_| {
            let key = match selected_key(&keys) { Ok(k) => k, Err(e) => { status.set_text(&format!("{e:#}")); return; } };
            let id = cache.text().trim().to_owned();
            form.set_sensitive(false); detail.set_text(""); status.set_text("Verifying cached release and reviewing changes…");
            let (form, status, detail, confirm, cancel) = (form.downgrade(), status.downgrade(), detail.downgrade(), confirm.downgrade(), cancel.downgrade());
            let reviewed = reviewed.clone();
            background(move || acquire::review_cached(&id, &key, update), move |result| {
                let (Some(form), Some(status), Some(detail), Some(confirm), Some(cancel)) = (form.upgrade(), status.upgrade(), detail.upgrade(), confirm.upgrade(), cancel.upgrade()) else { return; };
                match result {
                    Ok(Some(plan)) => {
                        detail.set_text(plan.detail()); *reviewed.borrow_mut() = Some(plan);
                        confirm.set_active(false); confirm.set_visible(true); cancel.set_sensitive(true);
                        status.set_text("Review ready. No deployment staged.");
                    }
                    Ok(None) => { status.set_text("No binary changes; nothing staged or accepted."); form.set_sensitive(true); }
                    Err(e) => { status.set_text(&format!("Review failed: {e:#}")); form.set_sensitive(true); }
                }
            });
        });
    }
    {
        let apply = apply.downgrade();
        confirm.connect_toggled(move |check| { if let Some(apply) = apply.upgrade() { apply.set_sensitive(check.is_active()); } });
    }
    {
        let (form, status, detail, confirm, reviewed) = (form.clone(), status.clone(), detail.clone(), confirm.clone(), reviewed.clone());
        cancel.connect_clicked(move |cancel| {
            reviewed.borrow_mut().take();
            confirm.set_active(false); confirm.set_visible(false); cancel.set_sensitive(false);
            form.set_sensitive(true); detail.set_text(""); status.set_text("Cancelled. No staging or acceptance performed.");
        });
    }
    {
        let (form, status, detail, confirm, cancel) = (form.clone(), status.clone(), detail.clone(), confirm.clone(), cancel.clone());
        apply.connect_clicked(move |apply| {
            if !confirm.is_active() { return; }
            let Some(plan) = reviewed.borrow_mut().take() else { return; };
            apply.set_sensitive(false); confirm.set_active(false); confirm.set_visible(false); cancel.set_sensitive(false);
            status.set_text("Revalidating trust and staging the approved release…");
            let (form, status, detail) = (form.downgrade(), status.downgrade(), detail.downgrade());
            background(move || plan.apply(), move |result| {
                let (Some(form), Some(status), Some(detail)) = (form.upgrade(), status.upgrade(), detail.upgrade()) else { return; };
                match result {
                    Ok(path) => { detail.set_text(&format!("Staged deployment: {}\nRefresh Deployments to inspect and activate separately.", path.display())); status.set_text("Staging complete."); }
                    Err(e) => status.set_text(&format!("Staging failed; review again: {e:#}")),
                }
                form.set_sensitive(true);
            });
        });
    }
    outer
}
