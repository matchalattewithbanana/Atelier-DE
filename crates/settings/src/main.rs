//! Native settings application. Writes shared user preferences only.
mod network_page;
mod appearance_page;
mod shell_preview;
mod wallpaper_page;
mod widgets_page;
mod windows_page;
mod window_rules_page;
mod motion_page;
mod notifications_page;
mod notification_rules_page;
mod themes_page;
use atelier_config::{config_path, Config, Layout, Theme};
use gtk::{gio, glib, prelude::*};
use std::{cell::RefCell, rc::Rc};
mod audio_page;
mod bluetooth_page;
mod window_preview;

fn main() -> glib::ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let initial_page = match args.as_slice() {
        [] => "appearance".to_owned(),
        [flag, id] if flag == "--page" && ["appearance", "shell-appearance", "wallpaper", "widgets", "windows", "window-rules", "motion", "notifications", "notification-rules", "themes", "applications", "session", "network", "power", "audio", "bluetooth", "about"].contains(&id.as_str()) => id.clone(),
        [flag] if flag == "--help" || flag == "-h" => {
            println!("atelier-settings [--page appearance|shell-appearance|wallpaper|widgets|windows|window-rules|motion|notifications|notification-rules|themes|applications|session|network|power|audio|bluetooth|about]");
            return glib::ExitCode::SUCCESS;
        }
        _ => { eprintln!("Invalid settings page; use --help."); return glib::ExitCode::FAILURE; }
    };
    let app = gtk::Application::builder().application_id("org.atelier.Settings").flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_activate(move |app| build_settings(app, &initial_page));
    app.run_with_args(&["atelier-settings"])
}

fn heading(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0); label.add_css_class("title-2");
    label
}

fn description(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0); label.set_wrap(true); label.add_css_class("dim-label");
    label
}

fn page(stack: &gtk::Stack, id: &str, title: &str, detail: &str) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.set_margin_top(24); content.set_margin_bottom(24);
    content.set_margin_start(24); content.set_margin_end(24);
    content.append(&heading(title)); content.append(&description(detail));
    let scroll = gtk::ScrolledWindow::builder().child(&content).build();
    stack.add_titled(&scroll, Some(id), title);
    content
}

#[derive(Clone)]
struct Fields {
    midnight: gtk::CheckButton,
    daylight: gtk::CheckButton,
    dusk: gtk::CheckButton,
    terminal: gtk::Entry,
    session: gtk::CheckButton,
    layout: gtk::ComboBoxText,
}

impl Fields {
    fn populate(&self, config: &Config) {
        match config.theme {
            Theme::Midnight => self.midnight.set_active(true),
            Theme::Daylight => self.daylight.set_active(true),
            Theme::Dusk => self.dusk.set_active(true),
        }
        self.terminal.set_text(&config.terminal);
        self.session.set_active(config.start_shell);
        self.layout.set_active_id(Some(match config.layout { Layout::Floating => "floating", Layout::Grid => "grid" }));
        if let Some(settings) = gtk::Settings::default() { settings.set_gtk_application_prefer_dark_theme(config.theme.is_dark()); }
    }
    fn collect(&self, baseline: &Config) -> Config {
        let mut updated = baseline.clone();
        updated.theme = if self.daylight.is_active() { Theme::Daylight } else if self.dusk.is_active() { Theme::Dusk } else { Theme::Midnight };
        updated.terminal = self.terminal.text().to_string();
        updated.start_shell = self.session.is_active();
        updated.layout = if self.layout.active_id().as_deref() == Some("grid") { Layout::Grid } else { Layout::Floating };
        updated
    }
}

fn build_settings(app: &gtk::Application, initial_page: &str) {
    let window = gtk::ApplicationWindow::builder().application(app).title("Atelier · Settings").default_width(760).default_height(510).build();
    let (path, config) = match config_path().and_then(|path| Config::load(&path).map(|config| (path, config))) {
        Ok(result) => result,
        Err(error) => {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
            content.set_margin_top(24); content.set_margin_start(24); content.set_margin_end(24);
            content.append(&heading("Settings unavailable"));
            content.append(&description(&format!("Existing configuration was not changed.\n{error}")));
            window.set_child(Some(&content)); window.present(); return;
        }
    };
    let baseline = Rc::new(RefCell::new(config));
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let layout_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    layout_box.set_vexpand(true);
    let stack = gtk::Stack::new();
    stack.set_hexpand(true); stack.set_vexpand(true);
    let sidebar = gtk::StackSidebar::new();
    sidebar.set_stack(&stack);
    sidebar.set_size_request(165, -1);
    layout_box.append(&sidebar);
    layout_box.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    layout_box.append(&stack);
    root.append(&layout_box);

    let appearance = page(&stack, "appearance", "Appearance", "Choose an Atelier color theme. Preview changes here before saving.");
    let themes = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let midnight = gtk::CheckButton::with_label(Theme::Midnight.name());
    let daylight = gtk::CheckButton::with_label(Theme::Daylight.name()); daylight.set_group(Some(&midnight));
    let dusk = gtk::CheckButton::with_label(Theme::Dusk.name()); dusk.set_group(Some(&midnight));
    for (button, theme) in [(&midnight, Theme::Midnight), (&daylight, Theme::Daylight), (&dusk, Theme::Dusk)] {
        themes.append(button);
        button.connect_toggled(move |button| {
            if button.is_active() { if let Some(settings) = gtk::Settings::default() { settings.set_gtk_application_prefer_dark_theme(theme.is_dark()); } }
        });
    }
    appearance.append(&themes);
    let native_appearance = page(&stack, "shell-appearance", "Shell appearance", "Native panel, dock and launcher preferences with a draft preview. This page has its own save and recovery controls; the window footer saves separate general settings.");
    appearance_page::build(&native_appearance);
    let wallpaper=page(&stack,"wallpaper","Wallpaper","Native desktop background. Saved changes apply to the running updated native shell.");
    wallpaper_page::build(&wallpaper);
    let widgets=page(&stack,"widgets","Desktop widgets","Choose a native desktop widget group. Saved changes apply to the running updated native shell.");
    widgets_page::build(&widgets);
    let windows=page(&stack,"windows","Window appearance","Native grid spacing. Saved changes apply to a running updated compositor.");
    windows_page::build(&windows);
    let rules=page(&stack,"window-rules","Application effects","Override native window effects by exact Wayland application ID. Rules are local and not included in portable themes.");
    window_rules_page::build(&rules);
    let motion=page(&stack,"motion","Motion & accessibility","Control native compositor animations and reduce motion independently of visual themes.");
    motion_page::build(&motion);
    let notifications=page(&stack,"notifications","Notifications","Customize Atelier notification cards. This page saves local presentation preferences separately.");
    notifications_page::build(&notifications);
    let notification_rules=page(&stack,"notification-rules","Notification rules","Configure application preferences and custom notification presets.");
    notification_rules_page::build(&notification_rules);
    let themes=page(&stack,"themes","Native themes","Generate, preview, import and export portable native visual preferences.");
    themes_page::build(&themes);

    let applications = page(&stack, "applications", "Applications", "Set the terminal that Atelier launches. This only changes Atelier's own preference.");
    let terminal_label = gtk::Label::new(Some("Terminal executable (name or absolute path)"));
    terminal_label.set_xalign(0.0);
    let terminal = gtk::Entry::new();
    applications.append(&terminal_label); applications.append(&terminal);

    let session_page = page(&stack, "session", "Session & windows", "These preferences apply to native Atelier sessions. Other desktops are not reconfigured.");
    let session = gtk::CheckButton::with_label("Launch the default shell when the session starts");
    session_page.append(&session);
    let layout = gtk::ComboBoxText::new();
    layout.append(Some("floating"), "Floating windows"); layout.append(Some("grid"), "Grid tiling");
    session_page.append(&description("Window layout"));
    session_page.append(&layout);
    session_page.append(&description("Explicit shell profiles supplied to atelier-session override the default startup choice."));

    let network = page(&stack, "network", "Network", "Manage existing network connections and inspect nearby Wi-Fi.");
    network_page::build(&network);

    let power_page = page(&stack, "power", "Power", "Read-only system battery and external power information.");
    let power_status = description("");
    power_status.set_selectable(true);
    update_power(&power_status);
    power_page.append(&power_status);
    let refresh = gtk::Button::with_label("Refresh power status");
    let target = power_status.clone();
    refresh.connect_clicked(move |_| update_power(&target));
    power_page.append(&refresh);

    let audio = page(&stack, "audio", "Audio", "Choose input and output devices and adjust volume or mute.");
    audio_page::build(&audio);

    let bluetooth = page(&stack, "bluetooth", "Bluetooth", "Manage the Bluetooth adapter and already-paired devices.");
    bluetooth_page::build(&bluetooth);

    let about = page(&stack, "about", "About", "Atelier Settings is an independent application. It does not require a running Atelier compositor.");
    for (name, value) in [
        ("Version", env!("CARGO_PKG_VERSION").to_string()),
        ("Desktop", std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "Unknown".into())),
        ("Session type", std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "Unknown".into())),
        ("Configuration", path.display().to_string()),
    ] {
        let label = gtk::Label::new(Some(&format!("{name}: {value}")));
        label.set_xalign(0.0); label.set_wrap(true); label.set_selectable(true); about.append(&label);
    }

    let fields = Fields { midnight, daylight, dusk, terminal, session, layout };
    fields.populate(&baseline.borrow());
    let footer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    footer.set_margin_start(16); footer.set_margin_end(16); footer.set_margin_top(12); footer.set_margin_bottom(12);
    let status = description("Preferences apply when components restart. Unsaved changes are only a preview.");
    footer.append(&status);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let reset = gtk::Button::with_label("Restore defaults");
    let reload = gtk::Button::with_label("Reload saved");
    let save = gtk::Button::with_label("Save preferences"); save.add_css_class("suggested-action");
    actions.append(&reset); actions.append(&reload);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0); spacer.set_hexpand(true); actions.append(&spacer);
    actions.append(&save); footer.append(&actions);

    let reset_fields = fields.clone();
    let message = status.clone();
    reset.connect_clicked(move |_| {
        reset_fields.populate(&Config::default());
        message.set_text("Defaults previewed. Save to persist the changes.");
    });
    let reload_fields = fields.clone();
    let reload_path = path.clone();
    let reload_baseline = Rc::clone(&baseline);
    let message = status.clone();
    reload.connect_clicked(move |_| match Config::load(&reload_path) {
        Ok(config) => {
            reload_fields.populate(&config);
            *reload_baseline.borrow_mut() = config;
            message.set_text("Reloaded saved preferences. Unsaved changes discarded.");
        }
        Err(error) => message.set_text(&format!("Reload failed: {error}")),
    });
    save.connect_clicked(move |_| {
        let saved = baseline.borrow().clone();
        match Config::load(&path) {
            Ok(current) if current == saved => {}
            Ok(_) => { status.set_text("Settings changed elsewhere. Reload saved preferences before editing again."); return; }
            Err(error) => { status.set_text(&format!("Cannot verify saved preferences: {error}")); return; }
        }
        let updated = fields.collect(&saved);
        match updated.save(&path) {
            Ok(()) => {
                *baseline.borrow_mut() = updated;
                status.set_text("Saved. Restart the shell/session to apply preferences.");
            }
            Err(error) => status.set_text(&format!("Save failed: {error}")),
        }
    });
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&footer);
    stack.set_visible_child_name(initial_page);
    window.set_child(Some(&root)); window.present();
}

fn update_power(label: &gtk::Label) {
    let power = match atelier_services::system_power_snapshot() {
        Ok(power) => power,
        Err(error) => { label.set_text(&format!("Power information unavailable: {error}")); return; }
    };
    let mut lines = vec![match power.external_power {
        Some(true) => "External power: connected",
        Some(false) => "External power: disconnected",
        None => "External power: unknown",
    }.to_owned()];
    if power.batteries.is_empty() { lines.push("No system batteries detected.".into()); }
    for battery in power.batteries {
        let percent = battery.percent.map(|p| format!("{p}%")).unwrap_or_else(|| "Unknown capacity".into());
        lines.push(format!("{} · {} · {}", battery.name, percent, battery.status.as_deref().unwrap_or("Unknown status")));
    }
    label.set_text(&lines.join("\n"));
}
