//! Native settings application. Writes the shared versioned configuration only.

use atelier_config::{config_path, Config, Theme};
use gtk::{gio, glib, prelude::*};

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder().application_id("org.atelier.Settings").flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_activate(build_settings);
    app.run()
}

fn build_settings(app: &gtk::Application) {
    let window = gtk::ApplicationWindow::builder().application(app).title("Atelier · Settings").default_width(510).default_height(420).build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.set_margin_top(24); content.set_margin_bottom(24); content.set_margin_start(24); content.set_margin_end(24);
    let title = gtk::Label::new(Some("System settings")); title.set_xalign(0.0); title.add_css_class("title-1"); content.append(&title);
    let (path, config) = match config_path().and_then(|path| Config::load(&path).map(|config| (path, config))) {
        Ok(result) => result,
        Err(error) => {
            let error = gtk::Label::new(Some(&format!("Settings could not be loaded. Existing configuration was not changed.\n{error}")));
            error.set_wrap(true); content.append(&error); window.set_child(Some(&content)); window.present(); return;
        }
    };
    if let Some(settings) = gtk::Settings::default() { settings.set_gtk_application_prefer_dark_theme(config.theme.is_dark()); }
    let appearance = gtk::Label::new(Some("Appearance")); appearance.set_xalign(0.0); appearance.add_css_class("heading"); content.append(&appearance);
    let themes = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let midnight = gtk::CheckButton::with_label(Theme::Midnight.name());
    let daylight = gtk::CheckButton::with_label(Theme::Daylight.name()); daylight.set_group(Some(&midnight));
    let dusk = gtk::CheckButton::with_label(Theme::Dusk.name()); dusk.set_group(Some(&midnight));
    match config.theme { Theme::Midnight => midnight.set_active(true), Theme::Daylight => daylight.set_active(true), Theme::Dusk => dusk.set_active(true) }
    themes.append(&midnight); themes.append(&daylight); themes.append(&dusk); content.append(&themes);
    for (button, theme) in [(&midnight, Theme::Midnight), (&daylight, Theme::Daylight), (&dusk, Theme::Dusk)] {
        button.connect_toggled(move |button| {
            if button.is_active() { if let Some(settings) = gtk::Settings::default() { settings.set_gtk_application_prefer_dark_theme(theme.is_dark()); } }
        });
    }
    let applications = gtk::Label::new(Some("Default applications")); applications.set_xalign(0.0); applications.add_css_class("heading"); content.append(&applications);
    let terminal_label = gtk::Label::new(Some("Terminal executable (name or absolute path)")); terminal_label.set_xalign(0.0); content.append(&terminal_label);
    let terminal = gtk::Entry::new(); terminal.set_text(&config.terminal); content.append(&terminal);
    let session = gtk::CheckButton::with_label("Launch desktop shell when the session starts"); session.set_active(config.start_shell); content.append(&session);
    let status = gtk::Label::new(Some("Preferences apply when components restart. Appearance previews in this window.")); status.set_wrap(true); status.set_xalign(0.0); status.add_css_class("dim-label");
    let save = gtk::Button::with_label("Save preferences"); save.add_css_class("suggested-action");
    let reset = gtk::Button::with_label("Restore defaults");
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 10); actions.append(&reset); actions.append(&save); content.append(&actions);
    let reset_fields = (midnight.clone(), terminal.clone(), session.clone(), status.clone());
    reset.connect_clicked(move |_| {
        let defaults = Config::default(); reset_fields.0.set_active(true); reset_fields.1.set_text(&defaults.terminal); reset_fields.2.set_active(defaults.start_shell);
        reset_fields.3.set_text("Defaults previewed. Save to persist the changes.");
    });
    let message = status.clone();
    save.connect_clicked(move |_| {
        let theme = if daylight.is_active() { Theme::Daylight } else if dusk.is_active() { Theme::Dusk } else { Theme::Midnight };
        let updated = Config { theme, terminal: terminal.text().to_string(), start_shell: session.is_active(), ..Config::default() };
        match updated.save(&path) {
            Ok(()) => message.set_text("Saved. Restart the shell/session to apply preferences."),
            Err(error) => message.set_text(&format!("Save failed: {error}")),
        }
    });
    content.append(&status); window.set_child(Some(&content)); window.present();
}
