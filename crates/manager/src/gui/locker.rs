//! Screen locker diagnostics. No locker process or authentication is started.
use gtk::prelude::*;
use super::{background,label,display_rows,Row};
pub(super) fn build()->gtk::Box{
    let outer=gtk::Box::new(gtk::Orientation::Vertical,12);
    outer.set_margin_top(24);outer.set_margin_bottom(24);outer.set_margin_start(24);outer.set_margin_end(24);
    outer.append(&label("Screen lockers","title-1"));
    outer.append(&label("Inspect Swaylock, Hyprlock and GTKLock prerequisites on the current Wayland connection. Native Atelier locking is not enabled. Nested locking cannot be assumed to secure the host.","dim-label"));
    let probe=gtk::Button::with_label("Check current compositor and locker packages");outer.append(&probe);
    let status=label("No checks performed.","body");status.set_selectable(true);outer.append(&status);
    let list=gtk::ListBox::new();list.add_css_class("boxed-list");list.set_selection_mode(gtk::SelectionMode::None);
    outer.append(&gtk::ScrolledWindow::builder().child(&list).vexpand(true).min_content_height(140).build());
    outer.append(&label("Install optional packages through the existing Installation review. A separate secure lock implementation and physical acceptance test are required before enabling native locker profiles. No login or lock settings are changed.","dim-label"));
    let (status,list)=(status.downgrade(),list.downgrade());
    probe.connect_clicked(move|button|{
        let (Some(status),Some(list))=(status.upgrade(),list.upgrade())else{return;};
        button.set_sensitive(false);status.set_text("Inspecting the current display registry…");
        let (button,status,list)=(button.downgrade(),status.downgrade(),list.downgrade());
        background(crate::locker::check,move|result|{
            let (Some(button),Some(status),Some(list))=(button.upgrade(),status.upgrade(),list.upgrade())else{return;};
            match result{
                Ok(report)=>{
                    status.set_text(&format!("Display: {}\nSession-lock: {:?} · layer-shell: {:?} · outputs: {:?}\n{}",report.display.as_deref().unwrap_or("(none)"),report.protocols.session_lock,report.protocols.layer_shell,report.protocols.outputs,report.protocols.detail));
                    display_rows(&list,report.lockers.into_iter().map(|l|Row::new(l.id,l.assessment,format!("Executable: {:?}\nPAM service file: {:?}\n{}",l.executable,l.pam_service,report.note))).collect());
                }
                Err(e)=>{status.set_text("Locker diagnostics unavailable.");display_rows(&list,vec![Row::error("Locker check",e)]);}
            }
            button.set_sensitive(true);
        });
    });
    outer
}
