//! Unified read-only customization browse/preview page.
use gtk::prelude::*;
use super::{background,display_rows,label,Row};
use crate::customization_registry::{self as registry,Action,Review};
use std::{cell::RefCell,rc::Rc};

fn review_dialog(parent:&gtk::Window,review:Review,status:&gtk::Label){
    let dialog=gtk::Dialog::builder().title("Review community reference").transient_for(parent)
        .modal(true).default_width(520).build();
    let content=dialog.content_area();
    content.set_spacing(12);
    content.set_margin_top(16);content.set_margin_bottom(16);content.set_margin_start(16);content.set_margin_end(16);
    content.append(&label("Save metadata only. No download, installation, trust grant or compatibility claim.","dim-label"));
    let detail=label(review.detail(),"body");detail.set_selectable(true);
    content.append(&detail);
    let confirm=gtk::Entry::new();confirm.set_placeholder_text(Some(review.phrase()));
    content.append(&label(&format!("Type exactly: {}",review.phrase()),"body"));content.append(&confirm);
    let message=label("","dim-label");content.append(&message);
    dialog.add_button("Cancel",gtk::ResponseType::Cancel);
    dialog.add_button("Save reference",gtk::ResponseType::Accept);
    let review=Rc::new(RefCell::new(Some(review)));
    let status=status.downgrade();
    dialog.connect_response(move|dialog,response|{
        if response!=gtk::ResponseType::Accept{dialog.close();return;}
        let typed=confirm.text().to_string();
        if !review.borrow().as_ref().is_some_and(|r|r.phrase()==typed){message.set_text("Confirmation does not match.");return;}
        let Some(review)=review.borrow_mut().take()else{return;};
        let status=status.clone();
        background(move||review.apply(&typed),move|result|{
            if let Some(status)=status.upgrade(){
                match result{Ok(text)=>status.set_text(&text),Err(error)=>status.set_text(&format!("Reference not saved: {error:#}"))}
            }
        });
        dialog.close();
    });
    dialog.present();
}

fn display_community(list:&gtk::ListBox,status:&gtk::Label,result:crate::customization_discovery::Search){
    display_rows(list,vec![]);
    for item in result.listings{
        let card=gtk::Box::new(gtk::Orientation::Vertical,6);
        card.set_margin_top(12);card.set_margin_bottom(12);card.set_margin_start(16);card.set_margin_end(16);
        card.append(&label(&item.repository,"heading"));
        card.append(&label(item.trust,"dim-label"));
        let detail=label(&format!("{}\n{} stars · license: {}\nUpdated: {}\n{}\nCompatibility: {}",item.source,item.stars,item.license,item.updated_at,item.description,item.compatibility),"body");
        detail.set_selectable(true);card.append(&detail);
        let button=gtk::Button::with_label("Review and save reference");card.append(&button);
        let value=format!("{} {}",result.category,item.repository);
        let status=status.downgrade();
        button.connect_clicked(move|button|{
            let Some(status)=status.upgrade()else{return;};
            let Some(parent)=button.root().and_downcast::<gtk::Window>()else{return;};
            let (parent,status,button)=(parent.downgrade(),status.downgrade(),button.downgrade());
            if let Some(button)=button.upgrade(){button.set_sensitive(false);}
            let value=value.clone();
            background(move||registry::review(Action::Community,&value),move|result|{
                if let Some(button)=button.upgrade(){button.set_sensitive(true);}
                let (Some(parent),Some(status))=(parent.upgrade(),status.upgrade())else{return;};
                match result{Ok(review)=>review_dialog(&parent,review,&status),Err(error)=>status.set_text(&format!("Reference review unavailable: {error:#}"))}
            });
        });
        list.append(&card);
    }
}

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
    let keywords=gtk::Entry::new();keywords.set_placeholder_text(Some("Optional community search keywords"));
    outer.append(&keywords);
    let discover=gtk::Button::with_label("Discover online (GitHub)");
    outer.append(&discover);
    outer.append(&label("Online searches require GitHub CLI access. Results are community references only, not installable packages.","dim-label"));
    let status=label("Choose a category to inspect sources, preparation and activation boundaries.","dim-label");
    outer.append(&status);
    let list=gtk::ListBox::new();list.add_css_class("boxed-list");list.set_selection_mode(gtk::SelectionMode::None);
    outer.append(&gtk::ScrolledWindow::builder().child(&list).vexpand(true).min_content_height(120).build());
    {
        let (category,keywords,status,list,refresh)=(category.downgrade(),keywords.downgrade(),status.downgrade(),list.downgrade(),refresh.downgrade());
        discover.connect_clicked(move|button|{
            let (Some(category),Some(keywords),Some(status),Some(list),Some(refresh))=(category.upgrade(),keywords.upgrade(),status.upgrade(),list.upgrade(),refresh.upgrade())else{return;};
            let Some(selected)=category.active_id().map(|s|s.to_string()).filter(|s|s!="all")else{status.set_text("Select a specific category for online discovery.");return;};
            let words=keywords.text().to_string();
            for widget in [&refresh,button]{widget.set_sensitive(false);}
            category.set_sensitive(false);keywords.set_sensitive(false);
            status.set_text("Searching public GitHub repository metadata…");
            let (button,refresh,category,keywords,status,list)=(button.downgrade(),refresh.downgrade(),category.downgrade(),keywords.downgrade(),status.downgrade(),list.downgrade());
            background(move||crate::customization_discovery::discover(&selected,&words),move|result|{
                let (Some(button),Some(refresh),Some(category),Some(keywords),Some(status),Some(list))=(button.upgrade(),refresh.upgrade(),category.upgrade(),keywords.upgrade(),status.upgrade(),list.upgrade())else{return;};
                match result{
                    Ok(result)=>{
                        status.set_text(&format!("{} reported matches; {} shown. {} No compatibility or trust verified.",result.total_matches,result.listings.len(),if result.incomplete{"Incomplete response."}else{"First page only."}));
                        display_community(&list,&status,result);
                    }
                    Err(error)=>{status.set_text("Online discovery unavailable; local catalog still available.");display_rows(&list,vec![Row::error("GitHub discovery",error)]);}
                }
                button.set_sensitive(true);refresh.set_sensitive(true);category.set_sensitive(true);keywords.set_sensitive(true);
            });
        });
    }
    let discover=discover.downgrade();
    let category=category.downgrade();let status=status.downgrade();let list=list.downgrade();
    refresh.connect_clicked(move|button|{
        let (Some(category),Some(status),Some(list))=(category.upgrade(),status.upgrade(),list.upgrade())else{return;};
        let selected=category.active_id().map(|id|id.to_string()).filter(|id|id!="all");
        button.set_sensitive(false);if let Some(d)=discover.upgrade(){d.set_sensitive(false);}status.set_text("Inspecting local customization catalog…");
        let (button,status,list,discover)=(button.downgrade(),status.downgrade(),list.downgrade(),discover.clone());
        background(move||crate::customization::filtered(selected.as_deref()),move|result|{
            let (Some(button),Some(status),Some(list))=(button.upgrade(),status.upgrade(),list.upgrade())else{return;};
            match result{
                Ok(items)=>{
                    status.set_text("Read-only review. Use Installation for optional packages; use Settings or the dedicated appearance pages for selection.");
                    display_rows(&list,items.into_iter().map(|i|Row::new(i.name,i.readiness,format!("{} · {} · {}\nSource: {}\nDeclared targets: {}\nPrepare: {}\nSelect: {}\n{}",i.id,i.category,i.scope,i.source,if i.declared_targets.is_empty(){"not specified".into()}else{i.declared_targets.join(", ")},i.install,i.activation,i.detail))).collect());
                }
                Err(error)=>{status.set_text("Catalog unavailable.");display_rows(&list,vec![Row::error("Customization catalog",error)]);}
            }
            button.set_sensitive(true);if let Some(d)=discover.upgrade(){d.set_sensitive(true);}
        });
    });
    outer
}
