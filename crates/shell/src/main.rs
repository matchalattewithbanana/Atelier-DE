//! Native Wayland client; does not import Smithay or compositor internals.

use atelier_config::{component, config_path, Config};
use gtk::{gio, glib, prelude::*};
use std::process::Command;
mod media_controls;
mod notification_centre;
#[cfg(feature = "layer-panel")]
mod panel;
#[cfg(feature = "layer-panel")]
mod workspace_backend;

fn main() -> glib::ExitCode {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("atelier-shell [--panel | --launcher]\nThe panel is optional; external desktops use launcher mode by default.");
        return glib::ExitCode::SUCCESS;
    }
    if arguments.len() > 1 || arguments.iter().any(|arg| arg != "--panel" && arg != "--launcher") {
        eprintln!("Use --panel or --launcher, not both.");
        return glib::ExitCode::FAILURE;
    }
    let requested_panel = arguments.first().is_some_and(|arg| arg == "--panel");
    let requested_launcher = arguments.first().is_some_and(|arg| arg == "--launcher");
    #[cfg(not(feature = "layer-panel"))]
    if requested_panel {
        eprintln!("Panel support requires the layer-panel build feature.");
        return glib::ExitCode::FAILURE;
    }
    // External rices own their existing bar. Only native sessions auto-start
    // the panel; other products opt in explicitly with --panel.
    let use_panel = requested_panel || (!requested_launcher && std::env::var("WAYLAND_DISPLAY").ok().is_some_and(|display| {
        display.starts_with("wayland-atelier-") ||
            atelier_config::workspace_paths(&display).ok().is_some_and(|(command, _)| command.exists())
    }));
    let app = gtk::Application::builder().application_id("org.atelier.Shell").flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_activate(move |app| {
        #[cfg(feature = "layer-panel")]
        if use_panel && gtk4_layer_shell::is_supported() { panel::build(app); return; }
        let _ = use_panel;
        build_shell(app);
    });
    app.run_with_args(&["atelier-shell"])
}

fn build_shell(app: &gtk::Application) {
    let loaded = config_path().and_then(|path| Config::load(&path));
    let (config, config_warning) = match loaded {
        Ok(config) => (config, String::new()),
        Err(error) => (Config::default(), format!("Configuration unavailable: {error}")),
    };
    if let Some(settings) = gtk::Settings::default() { settings.set_gtk_application_prefer_dark_theme(config.theme.is_dark()); }

    let window = gtk::ApplicationWindow::builder().application(app).title("Atelier · Desktop Shell").default_width(580).default_height(540).build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.set_margin_top(24); content.set_margin_bottom(24); content.set_margin_start(24); content.set_margin_end(24);
    let title = gtk::Label::new(Some("Atelier")); title.add_css_class("title-1"); title.set_xalign(0.0);
    let description = gtk::Label::new(Some("Your workspace")); description.add_css_class("dim-label"); description.set_xalign(0.0);
    content.append(&title); content.append(&description);

    let status = gtk::Label::new(Some(&config_warning)); status.set_wrap(true); status.set_xalign(0.0); status.add_css_class("dim-label");
    let shortcuts = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let terminal = gtk::Button::with_label("Terminal"); terminal.set_hexpand(true);
    let settings = gtk::Button::with_label("Settings"); settings.set_hexpand(true);
    let terminal_path = config.terminal.clone();
    let message = status.clone();
    terminal.connect_clicked(move |_| match Command::new(&terminal_path).spawn() {
        Ok(mut child) => { std::thread::spawn(move || { let _ = child.wait(); }); message.set_text("Terminal launched."); }
        Err(error) => message.set_text(&format!("Could not launch terminal: {error}")),
    });
    let message = status.clone();
    settings.connect_clicked(move |_| match component("atelier-settings").spawn() {
        Ok(mut child) => { std::thread::spawn(move || { let _ = child.wait(); }); message.set_text("Settings opened."); }
        Err(error) => message.set_text(&format!("Could not open Settings: {error}")),
    });
    shortcuts.append(&terminal); shortcuts.append(&settings);
    let media = gtk::Button::with_label("Media"); media.set_hexpand(true);
    let app_ref = app.downgrade();
    media.connect_clicked(move |_| { if let Some(app) = app_ref.upgrade() { media_controls::show(&app); } });
    shortcuts.append(&media);
    let notifications = gtk::Button::with_label("Notifications"); notifications.set_hexpand(true);
    let app_ref = app.downgrade();
    notifications.connect_clicked(move |_| { if let Some(app) = app_ref.upgrade() { notification_centre::show(&app); } });
    shortcuts.append(&notifications); content.append(&shortcuts);

    let search = gtk::Entry::new(); search.set_placeholder_text(Some("Search installed applications")); content.append(&search);
    let list = gtk::ListBox::new(); list.set_selection_mode(gtk::SelectionMode::None); list.add_css_class("boxed-list");
    let mut apps: Vec<_> = gio::AppInfo::all().into_iter().filter(|app| app.should_show()).collect();
    apps.sort_by_key(|app| app.display_name().to_lowercase());
    let mut seen = std::collections::HashSet::new();
    let mut rows = Vec::new();
    for app_info in apps {
        let name = app_info.display_name().to_string();
        let id = app_info.id().map(|id| id.to_string()).unwrap_or_else(|| name.clone());
        if !seen.insert(id.clone()) { continue; }
        let row = gtk::ListBoxRow::new();
        let button = gtk::Button::new(); button.add_css_class("flat");
        let item = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let icon = app_info.icon().map(|icon| gtk::Image::from_gicon(&icon)).unwrap_or_else(|| gtk::Image::from_icon_name("application-x-executable")); icon.set_pixel_size(32);
        let label = gtk::Label::new(Some(&name)); label.set_xalign(0.0); label.set_hexpand(true);
        item.append(&icon); item.append(&label); button.set_child(Some(&item)); row.set_child(Some(&button));
        let message = status.clone();
        button.connect_clicked(move |_| {
            let context = gtk::gdk::Display::default().map(|display| display.app_launch_context());
            match app_info.launch(&[], context.as_ref()) {
                Ok(()) => message.set_text("Application launched."),
                Err(error) => message.set_text(&format!("Could not launch application: {error}")),
            }
        });
        list.append(&row); rows.push((row, format!("{name} {id}").to_lowercase()));
    }
    search.connect_changed(move |entry| {
        let query = entry.text().to_lowercase();
        for (row, text) in &rows { row.set_visible(text.contains(query.trim())); }
    });
    let scroll = gtk::ScrolledWindow::builder().vexpand(true).child(&list).build();
    content.append(&scroll); content.append(&status);
    window.set_child(Some(&content)); window.present();
}
