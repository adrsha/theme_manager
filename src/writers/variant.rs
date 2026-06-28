use crate::{Theme, ThemeError, Writer};
use std::path::PathBuf;
use std::process::Command;

pub struct GtkSettings;
pub struct Qt6ct;

// ---------------------------------------------------------------------------
// GTK settings.ini — written for both gtk-3.0 and gtk-4.0
// ---------------------------------------------------------------------------

impl Writer for GtkSettings {
    fn name(&self) -> &'static str { "gtk" }

    // We write two files (gtk-3.0 and gtk-4.0). The trait only supports one
    // output_path, so we use gtk-3.0 as the primary and write gtk-4.0 manually
    // inside generate() by side-effecting — actually no, generate() should be
    // pure. Instead we write both in reload() where we have no return value
    // constraint... but reload() has no Theme access either.
    //
    // Cleanest solution: write both files from generate() by returning the
    // content, then duplicate the write in reload() for gtk-4.0 using the
    // same logic. Actually the simplest correct approach is: this writer
    // targets gtk-3.0, and we register a second identical writer for gtk-4.0.
    // See Gtk4Settings below.
    fn output_path(&self) -> PathBuf {
        crate::home().join(".config/gtk-3.0/settings.ini")
    }

    fn generate(&self, theme: &Theme) -> Result<String, ThemeError> {
        gtk_settings_content(theme)
    }

    fn reload(&self) {
        gsettings_apply(
            "org.gnome.desktop.interface",
            "color-scheme",
        );
    }
}

pub struct Gtk4Settings;

impl Writer for Gtk4Settings {
    fn name(&self) -> &'static str { "gtk4" }

    fn output_path(&self) -> PathBuf {
        crate::home().join(".config/gtk-4.0/settings.ini")
    }

    fn generate(&self, theme: &Theme) -> Result<String, ThemeError> {
        gtk_settings_content(theme)
    }

    // gsettings covers both gtk3 and gtk4 so no second call needed here.
    fn reload(&self) {}
}

fn gtk_settings_content(theme: &Theme) -> Result<String, ThemeError> {
    let prefer_dark = match theme.require("VARIANT")? {
        "dark"  => "1",
        "light" => "0",
        v => return Err(ThemeError::InvalidVariant(v.to_string())),
    };

    Ok(format!(
        "[Settings]\n\
         gtk-application-prefer-dark-theme = {prefer_dark}\n"
    ))
}

fn gsettings_apply(schema: &str, key: &str) {
    // We don't have Theme here so we read the gtk-3.0 settings.ini we just
    // wrote to decide the value — avoids duplicating the variant logic.
    let path = crate::home().join(".config/gtk-3.0/settings.ini");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("theme-manager: [gtk] could not read settings.ini for gsettings: {e}");
            return;
        }
    };

    let prefer_dark = content
        .lines()
        .find_map(|l| l.strip_prefix("gtk-application-prefer-dark-theme = "))
        .unwrap_or("0");

    let color_scheme = if prefer_dark == "1" {
        "prefer-dark"
    } else {
        "prefer-light"
    };

    let result = Command::new("gsettings")
        .args(["set", schema, key, color_scheme])
        .status();

    match result {
        Ok(s) if s.success() => {
            eprintln!("theme-manager: [gtk] gsettings color-scheme → {color_scheme}");
        }
        Ok(s) => eprintln!("theme-manager: [gtk] gsettings exited with {s}"),
        Err(e) => eprintln!("theme-manager: [gtk] gsettings not found: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Qt6ct
// ---------------------------------------------------------------------------

impl Writer for Qt6ct {
    fn name(&self) -> &'static str { "qt6ct" }

    fn output_path(&self) -> PathBuf {
        crate::home().join(".config/qt6ct/qt6ct.conf")
    }

    fn generate(&self, theme: &Theme) -> Result<String, ThemeError> {
        let color_scheme = match theme.require("VARIANT")? {
            "dark"  => "darker.conf",
            "light" => "airy.conf",
            v => return Err(ThemeError::InvalidVariant(v.to_string())),
        };

        // Read the existing qt6ct.conf and replace only the color_scheme_path
        // line, preserving all other user settings.
        let conf_path = self.output_path();
        let existing = std::fs::read_to_string(&conf_path).unwrap_or_default();

        Ok(patch_qt6ct(&existing, color_scheme))
    }
}

/// Replace (or insert) the color_scheme_path line in qt6ct.conf,
/// leaving every other line untouched.
fn patch_qt6ct(existing: &str, scheme_file: &str) -> String {
    let new_line = format!(
        "color_scheme_path=/usr/share/qt6ct/colors/{scheme_file}"
    );

    let mut found = false;
    let mut lines: Vec<String> = existing
        .lines()
        .map(|l| {
            if l.starts_with("color_scheme_path=") {
                found = true;
                new_line.clone()
            } else {
                l.to_string()
            }
        })
        .collect();

    if !found {
        // If the key didn't exist yet, add it under [Appearance] if present,
        // otherwise just append it.
        match lines.iter().position(|l| l.trim() == "[Appearance]") {
            Some(i) => lines.insert(i + 1, new_line),
            None    => lines.push(new_line),
        }
    }

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}
