use std::fs;
use std::io::ErrorKind;
use std::path::Path;

impl Writer for GtkSettings {
    fn name(&self) -> &'static str {
        "gtk"
    }

    fn output_path(&self) -> PathBuf {
        crate::home().join(".config/gtk-3.0/settings.ini")
    }

    fn generate(&self, theme: &Theme) -> Result<String, ThemeError> {
        gtk_settings_content(theme, &self.output_path())
    }

    fn reload(&self) {}
}

impl Writer for Gtk4Settings {
    fn name(&self) -> &'static str {
        "gtk4"
    }

    fn output_path(&self) -> PathBuf {
        crate::home().join(".config/gtk-4.0/settings.ini")
    }

    fn generate(&self, theme: &Theme) -> Result<String, ThemeError> {
        gtk_settings_content(theme, &self.output_path())
    }

    // gsettings covers both gtk3 and gtk4 so no second call needed here.
    fn reload(&self) {}
}

/// Returns the existing settings.ini with only `gtk-application-prefer-dark-theme`
/// changed. Every other line, comment and section is kept as it was.
fn gtk_settings_content(theme: &Theme, path: &Path) -> Result<String, ThemeError> {
    const KEY: &str = "gtk-application-prefer-dark-theme";

    let prefer_dark = match theme.require("VARIANT")? {
        "dark" => "1",
        "light" => "0",
        v => return Err(ThemeError::InvalidVariant(v.to_string())),
    };
    let setting = format!("{KEY} = {prefer_dark}");

    let existing = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        // Refuse to continue rather than overwrite a file we could not read.
        Err(e) => return Err(ThemeError::Io(e)),
    };

    let mut out: Vec<String> = Vec::new();
    let mut in_settings = false;
    let mut has_settings = false;
    let mut wrote_key = false;

    for line in existing.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_settings && !wrote_key {
                out.push(setting.clone());
                wrote_key = true;
            }
            in_settings = trimmed == "[Settings]";
            has_settings |= in_settings;
            out.push(line.to_string());
            continue;
        }

        let is_our_key = in_settings
            && line.split('=').next().map(str::trim) == Some(KEY);
        if is_our_key {
            // Replace the first occurrence in place, drop any duplicates.
            if !wrote_key {
                out.push(setting.clone());
                wrote_key = true;
            }
            continue;
        }

        out.push(line.to_string());
    }

    // The [Settings] section was the last one in the file.
    if in_settings && !wrote_key {
        out.push(setting.clone());
        wrote_key = true;
    }

    // No [Settings] section at all: add one.
    if !has_settings {
        out.push("[Settings]".to_string());
        out.push(setting);
    }

    Ok(out.join("\n") + "\n")
}
