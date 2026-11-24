//! Optional GTK layer-shell panel; the searchable launcher stays a normal window.
use atelier_config::{component, config_path, workspace_paths, Config};
use gtk::{glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::{fs, os::unix::net::UnixDatagram, process::Command, time::Duration};

pub fn build(app: &gtk::Application) {
    let config = config_path().and_then(|p| Config::load(&p)).unwrap_or_default();
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(config.theme.is_dark());
    }
    let window = gtk::ApplicationWindow::builder().application(app).title("Atelier panel").decorated(false).default_height(40).build();
    window.init_layer_shell();
    window.set_namespace("atelier-panel");
    window.set_layer(Layer::Top);
    for edge in [Edge::Left, Edge::Right, Edge::Top] { window.set_anchor(edge, true); }
    window.set_keyboard_mode(KeyboardMode::None);
    window.auto_exclusive_zone_enable();
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.set_margin_start(12); bar.set_margin_end(12); bar.set_margin_top(4); bar.set_margin_bottom(4);
    bar.add_css_class("atelier-panel");

    let launcher = gtk::Button::with_label("Atelier");
    launcher.add_css_class("suggested-action");
    let app_ref = app.downgrade();
    launcher.connect_clicked(move |_| {
        if let Some(app) = app_ref.upgrade() { crate::build_shell(&app); }
    });
    bar.append(&launcher);

    let paths = std::env::var("WAYLAND_DISPLAY").ok().and_then(|display| workspace_paths(&display).ok());
    let mut workspaces = Vec::new();
    for index in 1..=4u8 {
        let button = gtk::Button::with_label(&index.to_string());
        button.set_tooltip_text(Some(&format!("Workspace {index}")));
        button.add_css_class("flat");
        button.set_sensitive(paths.as_ref().is_some_and(|(command, _)| command.exists()));
        let destination = paths.as_ref().map(|(command, _)| command.clone());
        button.connect_clicked(move |_| {
            if let Some(path) = &destination {
                if let Ok(socket) = UnixDatagram::unbound() { let _ = socket.send_to(&[index], path); }
            }
        });
        bar.append(&button);
        workspaces.push(button);
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0); spacer.set_hexpand(true); bar.append(&spacer);
    let clock = gtk::Label::new(None);
    let battery = gtk::Label::new(None);
    battery.add_css_class("dim-label");
    bar.append(&clock); bar.append(&battery);
    let terminal = gtk::Button::with_label("Terminal");
    let terminal_path = config.terminal.clone();
    terminal.connect_clicked(move |_| spawn(Command::new(&terminal_path)));
    bar.append(&terminal);
    let settings = gtk::Button::with_label("Settings");
    settings.connect_clicked(move |_| spawn(component("atelier-settings")));
    bar.append(&settings);

    let tick = move || {
        if let Ok(now) = glib::DateTime::now_local() {
            if let Ok(text) = now.format("%a %d %b · %H:%M") { clock.set_text(&text); }
        }
        let power = battery_percent();
        battery.set_visible(power.is_some());
        if let Some(power) = power { battery.set_text(&format!("Battery {power}%")); }
        let active = paths.as_ref().and_then(|(_, state)| fs::read(state).ok()).and_then(|value| value.first().copied());
        for (index, button) in workspaces.iter().enumerate() {
            button.set_sensitive(paths.as_ref().is_some_and(|(command, _)| command.exists()));
            if active == Some((index + 1) as u8) { button.add_css_class("suggested-action"); }
            else { button.remove_css_class("suggested-action"); }
        }
    };
    tick();
    let weak = window.downgrade();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        if weak.upgrade().is_none() { return glib::ControlFlow::Break; }
        tick();
        glib::ControlFlow::Continue
    });

    let provider = gtk::CssProvider::new();
    provider.load_from_data(".atelier-panel { background: @theme_bg_color; padding: 2px; } .atelier-panel button { border-radius: 10px; padding: 3px 12px; }");
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    window.set_child(Some(&bar)); window.present();
}

fn spawn(mut command: Command) {
    match command.spawn() {
        Ok(mut child) => { std::thread::spawn(move || { let _ = child.wait(); }); }
        Err(error) => eprintln!("Atelier panel launch failed: {error}"),
    }
}

fn battery_percent() -> Option<u32> {
    let entries = fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in entries.flatten() {
        if fs::read_to_string(entry.path().join("type")).ok()?.trim() != "Battery" { continue; }
        if let Ok(value) = fs::read_to_string(entry.path().join("capacity")) {
            if let Ok(percent) = value.trim().parse::<u32>() { if percent <= 100 { return Some(percent); } }
        }
    }
    None
}
