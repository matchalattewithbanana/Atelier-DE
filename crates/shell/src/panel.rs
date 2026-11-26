//! Optional GTK layer-shell panel; the searchable launcher stays a normal window.
use atelier_config::{component, config_path, Appearance, Config};
use crate::workspace_backend::{installed_helper, WorkspaceBackend};
use gtk::{glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::{cell::RefCell,rc::Rc, process::Command, time::Duration};

pub fn build(app: &gtk::Application, popups: bool, island: bool, live:Rc<RefCell<Appearance>>) {
    let appearance = live.borrow().clone();
    let config = config_path().and_then(|p| Config::load(&p)).unwrap_or_default();
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(config.theme.is_dark());
    }
    let window = gtk::ApplicationWindow::builder().application(app).title("Atelier panel").decorated(false).build();
    window.add_css_class("atelier-panel-layer");
    window.init_layer_shell();
    window.set_namespace(Some("atelier-panel"));
    window.set_layer(Layer::Top);
    for edge in [Edge::Left, Edge::Right, Edge::Top] { window.set_anchor(edge, true); }
    window.set_keyboard_mode(KeyboardMode::None);
    window.auto_exclusive_zone_enable();
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, appearance.panel_spacing as i32);
    bar.set_size_request(-1, appearance.panel_height as i32);
    // Outer viewport owns margins so they do not scroll with the controls.
    bar.add_css_class("atelier-panel");

    let mut shortcuts=Vec::new();
    let launcher_control=crate::panel_layout::Shortcut::new("Atelier","view-grid-symbolic");
    let launcher=launcher_control.button.clone();
    launcher.add_css_class("suggested-action");
    let app_ref = app.downgrade();
    launcher.connect_clicked(move |_| {
        if let Some(app) = app_ref.upgrade() { crate::build_shell(&app); }
    });
    bar.append(&launcher);shortcuts.push(launcher_control);

    let backend = Rc::new(WorkspaceBackend::detect());
    let backend_label = gtk::Label::new(Some(backend.label()));
    backend_label.add_css_class("dim-label");
    bar.append(&backend_label);
    launcher.set_tooltip_text(Some(&format!("Open Atelier launcher · {}",backend.label())));
    let workspaces=crate::workspace_strip::Strip::new(backend.clone());
    bar.append(&workspaces.root);
    if backend.is_external() && installed_helper("atelier").is_some() {
        let rice_control=crate::panel_layout::Shortcut::new("Rices","applications-graphics-symbolic");
        let rices=rice_control.button.clone();
        rices.set_tooltip_text(Some("Browse installed and available Atelier rice products"));
        let app_ref = app.downgrade();
        rices.connect_clicked(move |_| {
            if let Some(app) = app_ref.upgrade() { show_rices(&app); }
        });
        bar.append(&rices);shortcuts.push(rice_control);
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0); spacer.set_hexpand(true); bar.append(&spacer);
    let clock = gtk::Label::new(None);
    let battery = gtk::Label::new(None);
    battery.add_css_class("dim-label");
    let status_group=gtk::Box::new(gtk::Orientation::Horizontal,6);status_group.add_css_class("atelier-panel-group");
    status_group.set_valign(gtk::Align::Center);status_group.append(&clock);status_group.append(&battery);
    clock.set_single_line_mode(true);battery.set_single_line_mode(true);battery.set_max_width_chars(16);battery.set_ellipsize(gtk::pango::EllipsizeMode::End);
    bar.append(&status_group);
    let controls=gtk::Box::new(gtk::Orientation::Horizontal,4);controls.add_css_class("atelier-panel-group");controls.set_valign(gtk::Align::Center);
    let media_control=crate::panel_layout::Shortcut::new("Media","multimedia-player-symbolic");
    let media=media_control.button.clone();
    let app_ref = app.downgrade();
    media.connect_clicked(move |_| { if let Some(app) = app_ref.upgrade() { crate::media_controls::show(&app); } });
    controls.append(&media);shortcuts.push(media_control); media.set_visible(appearance.show_media);
    let notifications_control=crate::panel_layout::Shortcut::new("Notifications","preferences-system-notifications-symbolic");
    let notifications=notifications_control.button.clone();
    let app_ref = app.downgrade();
    notifications.connect_clicked(move |_| { if let Some(app) = app_ref.upgrade() { crate::notification_centre::show(&app); } });
    controls.append(&notifications);shortcuts.push(notifications_control); notifications.set_visible(appearance.show_notifications);
    let audio = quick_button(app,&window);
    controls.append(&audio); audio.set_visible(appearance.show_audio);
    audio.update_property(&[gtk::accessible::Property::Label("Quick controls")]);
    let terminal_control=crate::panel_layout::Shortcut::new("Terminal","utilities-terminal-symbolic");
    let terminal=terminal_control.button.clone();
    let terminal_path = config.terminal.clone();
    terminal.connect_clicked(move |_| spawn(Command::new(&terminal_path)));
    controls.append(&terminal);shortcuts.push(terminal_control); terminal.set_visible(appearance.show_terminal);
    let settings_control=crate::panel_layout::Shortcut::new("Settings","preferences-system-symbolic");
    let settings=settings_control.button.clone();
    settings.connect_clicked(move |_| spawn(component("atelier-settings")));
    controls.append(&settings);shortcuts.push(settings_control);
    bar.append(&controls);
    let layout=crate::panel_layout::Layout::new(&bar,shortcuts,&backend_label,&audio,backend.count());
    let panel_weak=window.downgrade();

    if appearance.show_battery { update_power(&battery); } else { battery.set_visible(false); }
    let live_power=live.clone();
    let battery_weak = battery.downgrade();
    glib::timeout_add_local(Duration::from_secs(5), move || {
        let Some(label) = battery_weak.upgrade() else { return glib::ControlFlow::Break; };
        if live_power.borrow().show_battery { update_power(&label); }else{label.set_visible(false);}
        glib::ControlFlow::Continue
    });
    let layers_live=live.clone();
    let notification_live=live.clone();
    let mut last_panel=appearance.clone();
    let bar_weak=bar.downgrade();
    let mut tick = move || {
        let current=live.borrow();
        if *current!=last_panel{
            if let Some(bar)=bar_weak.upgrade(){bar.set_spacing(current.panel_spacing as i32);bar.set_size_request(-1,current.panel_height as i32);}
            media.set_visible(current.show_media);notifications.set_visible(current.show_notifications);audio.set_visible(current.show_audio);terminal.set_visible(current.show_terminal);
            if current.show_battery!=last_panel.show_battery{if current.show_battery{update_power(&battery);}else{battery.set_visible(false);}}
            last_panel=current.clone();
        }
        let compact=panel_weak.upgrade().is_none_or(|panel|layout.update(&panel,current.panel_spacing));
        if let Ok(now) = glib::DateTime::now_local() {
            if let Ok(text) = now.format(if current.show_date&&!compact { "%a %d %b · %H:%M" } else { "%H:%M" }) { clock.set_text(&text); }
            if let Ok(text)=now.format(if current.show_date{"%a %d %b · %H:%M"}else{"%H:%M"}){clock.set_tooltip_text(Some(&text));}
        }
        let snapshot = backend.snapshot();
        workspaces.update(snapshot.as_ref(),current.motion.reduced_motion);
    };
    tick();
    let weak = window.downgrade();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        if weak.upgrade().is_none() { return glib::ControlFlow::Break; }
        tick();
        glib::ControlFlow::Continue
    });

    window.connect_map(|_| println!("Atelier layer panel mapped"));
    // A constrained viewport allows narrow outputs to reach every enabled
    // item, even with nine workspaces, large fonts or custom spacing.
    let viewport=gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never).propagate_natural_width(false)
        .propagate_natural_height(true).min_content_width(0).child(&bar).build();
    viewport.set_margin_start(12);viewport.set_margin_end(12);viewport.set_margin_top(4);viewport.set_margin_bottom(4);
    window.set_child(Some(&viewport)); window.present();
    crate::desktop_layers::start(app,&window,layers_live);
    if popups { crate::notification_toast::start(app, &window, island, notification_live); }
}

/// Use the layer surface's own native popup relationship, not absolute
/// screen coordinates. Older layer-shell versions lack on-demand focus.
fn quick_button(app:&gtk::Application,panel:&gtk::ApplicationWindow)->gtk::Widget{
    if gtk4_layer_shell::protocol_version()<4{
        let button=gtk::Button::with_label("Quick controls");
        button.set_tooltip_text(Some("Wi-Fi, Bluetooth, audio and media"));
        let app=app.downgrade();
        button.connect_clicked(move |_|{if let Some(app)=app.upgrade(){crate::quick_controls::show(&app);}});
        return button.upcast();
    }
    let button=gtk::MenuButton::new();button.set_label("Quick controls");
    button.set_tooltip_text(Some("Wi-Fi, Bluetooth, audio and media"));
    let (app,panel)=(app.downgrade(),panel.downgrade());
    button.set_create_popup_func(move|button|{
        let (Some(app),Some(panel))=(app.upgrade(),panel.upgrade())else{return;};
        let popup=gtk::Popover::new();popup.set_autohide(true);
        popup.set_has_arrow(false);popup.set_position(gtk::PositionType::Bottom);
        popup.add_css_class("atelier-quick-popover");
        let scroll=gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(320).max_content_height(520).propagate_natural_height(true)
            .child(&crate::quick_controls::content(&app)).build();
        popup.set_child(Some(&scroll));
        let weak_panel=panel.downgrade();let weak_scroll=scroll.downgrade();
        popup.connect_map(move |_|{
            if let Some(panel)=weak_panel.upgrade(){
                // A popup never takes exclusive keyboard focus.
                panel.set_keyboard_mode(KeyboardMode::OnDemand);
                if let (Some(surface),Some(scroll))=(panel.surface(),weak_scroll.upgrade()){
                    if let Some(monitor)=surface.display().monitor_at_surface(&surface){
                        scroll.set_max_content_height((monitor.geometry().height()-120).clamp(100,520));
                    }
                }
            }
        });
        let weak_panel=panel.downgrade();
        popup.connect_unmap(move |_|{if let Some(panel)=weak_panel.upgrade(){panel.set_keyboard_mode(KeyboardMode::None);}});
        button.set_popover(Some(&popup));
    });
    button.upcast()
}

fn spawn(mut command: Command) {
    match command.spawn() {
        Ok(mut child) => { std::thread::spawn(move || { let _ = child.wait(); }); }
        Err(error) => eprintln!("Atelier panel launch failed: {error}"),
    }
}

fn update_power(label: &gtk::Label) {
    let Ok(power) = atelier_services::system_power_snapshot() else {
        label.set_text("Power unavailable");
        label.set_tooltip_text(Some("Could not read system power inventory"));
        label.set_visible(true);
        return;
    };
    label.set_visible(!power.batteries.is_empty());
    let summary = power.batteries.iter().map(|battery| {
        battery.percent.map(|p| format!("{p}%")).unwrap_or_else(|| "?".into())
    }).collect::<Vec<_>>().join(" / ");
    label.set_text(&format!("Battery {summary}"));
    let details = power.batteries.iter().map(|battery| {
        format!("{}: {}", battery.name, battery.status.as_deref().unwrap_or("Unknown"))
    }).collect::<Vec<_>>().join("\n");
    let source = match power.external_power { Some(true) => "External power connected", Some(false) => "External power disconnected", None => "External power unknown" };
    label.set_tooltip_text(Some(&format!("{details}\n{source}")));
}

fn show_rices(app: &gtk::Application) {
    let Some(helper) = installed_helper("atelier") else { return; };
    let window = gtk::ApplicationWindow::builder().application(app).title("Atelier · Rice products").default_width(570).default_height(420).build();
    let text = gtk::TextView::new();
    text.set_editable(false);
    text.set_monospace(true);
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    text.buffer().set_text("Loading rice catalog…");
    let scroll = gtk::ScrolledWindow::builder().child(&text).build();
    window.set_child(Some(&scroll));
    window.present();
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = match Command::new(helper).args(["rice", "list"]).output() {
            Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout).into_owned(),
            Ok(output) => format!("Rice manager returned {}:\n{}", output.status, String::from_utf8_lossy(&output.stderr)),
            Err(error) => format!("Rice manager could not start: {error}"),
        };
        let _ = send.send(result);
    });
    let weak = text.downgrade();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let Some(view) = weak.upgrade() else { return glib::ControlFlow::Break; };
        match receive.try_recv() {
            Ok(result) => { view.buffer().set_text(&result); glib::ControlFlow::Break }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => { view.buffer().set_text("Rice manager unavailable."); glib::ControlFlow::Break }
        }
    });
}
