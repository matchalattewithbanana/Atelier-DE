//! Unified read-only customization browse/preview page.
use gtk::prelude::*;
use super::{background,display_rows,label,Row};

pub(super) fn build()->gtk::Box{
    let outer=gtk::Box::new(gtk::Orientation::Vertical,12);
    outer.set_margin_top(24);outer.set_margin_bottom(24);outer.set_margin_start(24);outer.set_margin_end(24);
    outer.append(&label("Customization catalog","title-1"));
    outer.append(&label("Browse optional cursor packs, login screens, lockers, icon themes and existing appearance workflows. Detection is not validation. No service changes or community code execution.","dim-label"));
    let category=gtk::ComboBoxText::new();
    category.append(Some("all"),"All categories");
    for id in crate::customization::CATEGORIES{category.append(Some(id),id);}
    category.set_active_id(Some("all"));
    outer.append(&category);
    let refresh=gtk::Button::with_label("Browse / refresh catalog");
    outer.append(&refresh);
    let status=label("Choose a category to inspect sources, preparation and activation boundaries.","dim-label");
    outer.append(&status);
    let list=gtk::ListBox::new();list.add_css_class("boxed-list");list.set_selection_mode(gtk::SelectionMode::None);
    outer.append(&gtk::ScrolledWindow::builder().child(&list).vexpand(true).min_content_height(120).build());
    let category=category.downgrade();let status=status.downgrade();let list=list.downgrade();
    refresh.connect_clicked(move|button|{
        let (Some(category),Some(status),Some(list))=(category.upgrade(),status.upgrade(),list.upgrade())else{return;};
        let selected=category.active_id().map(|id|id.to_string()).filter(|id|id!="all");
        button.set_sensitive(false);status.set_text("Inspecting local customization catalog…");
        let (button,status,list)=(button.downgrade(),status.downgrade(),list.downgrade());
        background(move||crate::customization::filtered(selected.as_deref()),move|result|{
            let (Some(button),Some(status),Some(list))=(button.upgrade(),status.upgrade(),list.upgrade())else{return;};
            match result{
                Ok(items)=>{
                    status.set_text("Read-only review. Use Installation for optional packages; use Settings or the dedicated appearance pages for selection.");
                    display_rows(&list,items.into_iter().map(|i|Row::new(i.name,i.readiness,format!("{} · {} · {}\nSource: {}\nPrepare: {}\nSelect: {}\n{}",i.id,i.category,i.scope,i.source,i.install,i.activation,i.detail))).collect());
                }
                Err(error)=>{status.set_text("Catalog unavailable.");display_rows(&list,vec![Row::error("Customization catalog",error)]);}
            }
            button.set_sensitive(true);
        });
    });
    outer
}
