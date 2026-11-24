//! Optional GTK layer-shell panel; the searchable launcher stays a normal window.
use atelier_config::{component, config_path, Config};
use crate::workspace_backend::{installed_helper, WorkspaceBackend};
use gtk::{glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::{rc::Rc, process::Command, time::Duration};

pub fn build(app: &gtk::Application) {
    let config = config_path().and_then(|p| Config::load(&p)).unwrap_or_default();
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(config.theme.is_dark());
    }
    let window = gtk::ApplicationWindow::builder().application(app).title("Atelier panel").decorated(false).default_height(40).build();
    window.init_layer_shell();
    window.set_namespace(Some("atelier-panel"));
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

    let backend = Rc::new(WorkspaceBackend::detect());
    let backend_label = gtk::Label::new(Some(backend.label()));
    backend_label.add_css_class("dim-label");
    bar.append(&backend_label);
    let mut workspaces = Vec::new();
    for index in 1..=backend.count() as u8 {
        let button = gtk::Button::with_label(&index.to_string());
        button.set_tooltip_text(Some(&format!("Workspace {index}")));
        button.add_css_class("flat");
        button.set_sensitive(false);
        let target = Rc::clone(&backend);
        button.connect_clicked(move |_| target.focus(index));
        bar.append(&button);
        workspaces.push(button);
    }
    if backend.is_external() && installed_helper("atelier").is_some() {
        let rices = gtk::Button::with_label("Rices");
        rices.set_tooltip_text(Some("Browse installed and available Atelier rice products"));
        let app_ref = app.downgrade();
        rices.connect_clicked(move |_| {
            if let Some(app) = app_ref.upgrade() { show_rices(&app); }
        });
        bar.append(&rices);
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0); spacer.set_hexpand(true); bar.append(&spacer);
    let clock = gtk::Label::new(None);
    let battery = gtk::Label::new(None);
    battery.add_css_class("dim-label");
    bar.append(&clock); bar.append(&battery);
    let media = gtk::Button::with_label("Media");
    let app_ref = app.downgrade();
    media.connect_clicked(move |_| { if let Some(app) = app_ref.upgrade() { crate::media_controls::show(&app); } });
    bar.append(&media);
    let terminal = gtk::Button::with_label("Terminal");
    let terminal_path = config.terminal.clone();
    terminal.connect_clicked(move |_| spawn(Command::new(&terminal_path)));
    bar.append(&terminal);
    let settings = gtk::Button::with_label("Settings");
    settings.connect_clicked(move |_| spawn(component("atelier-settings")));
    bar.append(&settings);

    update_power(&battery);
    let battery_weak = battery.downgrade();
    glib::timeout_add_local(Duration::from_secs(5), move || {
        let Some(label) = battery_weak.upgrade() else { return glib::ControlFlow::Break; };
        update_power(&label);
        glib::ControlFlow::Continue
    });
    let tick = move || {
        if let Ok(now) = glib::DateTime::now_local() {
            if let Ok(text) = now.format("%a %d %b · %H:%M") { clock.set_text(&text); }
        }
        let snapshot = backend.snapshot();
        for (index, button) in workspaces.iter().enumerate() {
            button.set_sensitive(snapshot.is_some());
            if snapshot.as_ref().is_some_and(|state| state.active[index]) { button.add_css_class("suggested-action"); }
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
    window.connect_map(|_| println!("Atelier layer panel mapped"));
    window.set_child(Some(&bar)); window.present();
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
