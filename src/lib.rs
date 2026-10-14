//! Shared theme parsing and config generation.
//!
//! # Adding a new config target
//!
//! 1. Create a struct in `writers/` (or inline below for small ones).
//! 2. impl `Writer` for it — just `fn generate(&self, theme: &Theme) -> Result<String, ThemeError>`.
//! 3. Add it to the `all_writers()` list in `writers/mod.rs`.
//!
//! That's it. Output path, optional reload hook, and error reporting are all
//! handled by `apply_theme`.

pub mod api;
pub mod dynamic;
pub mod writers;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME not set"))
}

pub fn themes_dir() -> PathBuf {
    home().join(".config/theme-manager/themes")
}

/// True if `name` resolves to an executable file via `PATH`.
/// Writers use this (together with the config-dir check in
/// [`app_enabled`]) to skip apps that are not installed, so theme
/// switches do not create config bloat for software you don't use.
pub fn binary_exists(name: &str) -> bool {
    let Ok(path_var) = std::env::var("PATH") else {
        return false;
    };
    path_var.split(':').any(|dir| {
        let candidate = PathBuf::from(dir).join(name);
        candidate.is_file() && is_executable(&candidate)
    })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// True when an app should receive theme output: its binary is on
/// `PATH` or its config root already exists under `$HOME`.
/// `config_rel` is relative to `$HOME`, e.g. `".config/ghostty"`.
pub fn app_enabled(binary: &str, config_rel: &str) -> bool {
    binary_exists(binary) || home().join(config_rel).exists()
}

pub fn state_file() -> PathBuf {
    home().join(".local/state/theme/current")
}

pub fn amoled_state_file() -> PathBuf {
    home().join(".local/state/theme/amoled")
}

pub fn amoled_enabled() -> bool {
    fs::read_to_string(amoled_state_file())
        .map(|value| matches!(value.trim(), "1" | "true" | "on" | "enabled"))
        .unwrap_or(false)
}

pub fn set_amoled_enabled(enabled: bool) -> Result<(), ThemeError> {
    write_file(
        &amoled_state_file(),
        if enabled { "true\n" } else { "false\n" },
    )
}


pub fn get_transparency(theme: &Theme) -> f32 {
    match theme.vars.get("TRANSPARENT").map(|s| s.trim().to_lowercase()) {
        None => 1.0,
        Some(v) => {
            // Numeric opacity first: TRANSPARENT=0.5 means 50% opacity.
            if let Ok(opacity) = v.parse::<f32>() {
                return opacity.clamp(0.0, 1.0);
            }
            // Boolean fallback: TRANSPARENT=true needs a concrete opacity.
            // 0.9 matches the fog theme's explicit "transparent" value.
            match v.as_str() {
                "true" | "1" | "yes" | "on" => 0.9,
                _ => 1.0,
            }
        }
    }
}

pub fn fish_conf_file() -> PathBuf {
    home().join(".config/fish/conf.d/theme_vars.fish")
}

/// POSIX `export` file sourced by bash/zsh/sh rc files.
pub fn env_sh_file() -> PathBuf {
    home().join(".config/theme-manager/theme_env.sh")
}

/// systemd `environment.d` snippet, loaded by the user manager at login.
pub fn env_systemd_file() -> PathBuf {
    home().join(".config/environment.d/theme-manager.conf")
}

/// Hyprland `hl.env(...)` file, required from `hyprland.lua`.
pub fn hypr_env_file() -> PathBuf {
    home().join(".config/hypr/theme_env.lua")
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub path: PathBuf,
    pub vars: HashMap<String, String>,
}

impl Theme {
    /// Get a required key, returning a clear error if missing.
    pub fn require(&self, key: &str) -> Result<&str, ThemeError> {
        self.vars
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| ThemeError::MissingKey(key.to_string()))
    }

    /// Convert a required hex color key to (r, g, b).
    pub fn require_rgb(&self, key: &str) -> Result<(u8, u8, u8), ThemeError> {
        hex_to_rgb(self.require(key)?)
    }
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ThemeError {
    Io(std::io::Error),
    MissingKey(String),
    InvalidVariant(String),
    InvalidHex(String),
    InvalidStyle(String),
    Dynamic(String),
}

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThemeError::Io(e) => write!(f, "io error: {e}"),
            ThemeError::MissingKey(k) => write!(f, "missing required key: {k}"),
            ThemeError::InvalidVariant(v) => {
                write!(f, "VARIANT must be 'dark' or 'light', got: {v}")
            }
            ThemeError::InvalidHex(h) => write!(f, "invalid hex color: {h}"),
            ThemeError::InvalidStyle(message) => write!(f, "invalid style value: {message}"),
            ThemeError::Dynamic(message) => write!(f, "dynamic palette error: {message}"),
        }
    }
}

impl From<std::io::Error> for ThemeError {
    fn from(e: std::io::Error) -> Self {
        ThemeError::Io(e)
    }
}

// ---------------------------------------------------------------------------
// Writer trait
// ---------------------------------------------------------------------------

/// Implement this to add a new config target.
///
/// `generate` returns the full file content as a String.
/// The framework handles writing to disk and calling `reload`.
///
/// Contract (enforced by review, not the compiler):
/// - NEVER read, patch, or overwrite the user's own config files.
///   Always write a separate manager-owned generated file next to them.
/// - The user imports that file themselves (include/require/@import/...).
/// - Every generated file starts with a header comment: what generated it
///   plus a short "To use:" tutorial with the exact import line.
pub trait Writer {
    /// Human-readable name shown in log output (e.g. "ghostty", "hyprland").
    fn name(&self) -> &'static str;

    /// Absolute path to write the generated content to.
    fn output_path(&self) -> PathBuf;

    /// Generate the full file content from the theme.
    fn generate(&self, theme: &Theme) -> Result<String, ThemeError>;

    /// Whether this writer should run. Override to skip apps that are
    /// not installed (binary missing and no config dir), so theme
    /// switches don't create config bloat. Default is always-on for
    /// core targets (shell env, gtk, qt, nvim).
    fn enabled(&self) -> bool {
        true
    }

    /// Optional: called after a successful write. Default is a no-op.
    fn reload(&self) {}
}

// ---------------------------------------------------------------------------
// Shared helpers (available to all writers)
// ---------------------------------------------------------------------------

pub fn hex_to_rgb(hex: &str) -> Result<(u8, u8, u8), ThemeError> {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 {
        return Err(ThemeError::InvalidHex(hex.to_string()));
    }
    let r =
        u8::from_str_radix(&h[0..2], 16).map_err(|_| ThemeError::InvalidHex(hex.to_string()))?;
    let g =
        u8::from_str_radix(&h[2..4], 16).map_err(|_| ThemeError::InvalidHex(hex.to_string()))?;
    let b =
        u8::from_str_radix(&h[4..6], 16).map_err(|_| ThemeError::InvalidHex(hex.to_string()))?;
    Ok((r, g, b))
}

/// Format an rgb tuple as "r, g, b" for use in CSS.
pub fn rgb_str(c: (u8, u8, u8)) -> String {
    format!("{}, {}, {}", c.0, c.1, c.2)
}

pub fn write_file(path: &Path, content: &str) -> Result<(), ThemeError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Theme parsing
// ---------------------------------------------------------------------------

pub fn parse_theme(path: &Path) -> Result<Theme, ThemeError> {
    let content = fs::read_to_string(path)?;
    let mut vars = HashMap::new();

    for (lineno, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line.split_once('=') {
            None => {
                eprintln!("theme-manager: line {}: no '=' found, skipping", lineno + 1);
            }
            Some((key, val)) => {
                let key = key.trim().to_string();
                if key.is_empty() {
                    eprintln!("theme-manager: line {}: empty key, skipping", lineno + 1);
                    continue;
                }
                let val = val.trim().to_string();
                if vars.contains_key(&key) {
                    eprintln!(
                        "theme-manager: line {}: duplicate key '{key}', overriding previous value",
                        lineno + 1
                    );
                }
                vars.insert(key, val);
            }
        }
    }

    match vars.get("VARIANT").map(String::as_str) {
        Some("dark") | Some("light") => {}
        Some(v) => return Err(ThemeError::InvalidVariant(v.to_string())),
        None => return Err(ThemeError::MissingKey("VARIANT".to_string())),
    }

    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(Theme {
        name,
        path: path.to_path_buf(),
        vars,
    })
}

/// Resolve dynamic colors and apply the manager-wide AMOLED override.
/// Every writer receives this final palette, so static and dynamic themes
/// behave consistently across terminals, notifications, launchers, and apps.
pub fn resolve_theme(path: &Path) -> Result<Theme, ThemeError> {
    let mut theme = parse_theme(path)?;
    dynamic::resolve_dynamic_palette(&mut theme)?;
    if amoled_enabled() {
        apply_amoled_palette(&mut theme);
    }
    apply_style_defaults(&mut theme)?;
    Ok(theme)
}

/// Resolve optional compositor/shell geometry into a complete, validated set.
/// Themes can override these values while older themes keep the current look.
pub fn apply_style_defaults(theme: &mut Theme) -> Result<(), ThemeError> {
    let radius = theme
        .vars
        .entry("CORNER_RADIUS".to_string())
        .or_insert_with(|| "20".to_string());
    let radius_value = radius.parse::<u8>().map_err(|_| {
        ThemeError::InvalidStyle(format!(
            "CORNER_RADIUS must be an integer from 0 to 64, got {radius}"
        ))
    })?;
    if radius_value > 64 {
        return Err(ThemeError::InvalidStyle(format!(
            "CORNER_RADIUS must be from 0 to 64, got {radius_value}"
        )));
    }

    let border_size = theme
        .vars
        .entry("BORDER_SIZE".to_string())
        .or_insert_with(|| "0".to_string());
    let border_size_value = border_size.parse::<u8>().map_err(|_| {
        ThemeError::InvalidStyle(format!(
            "BORDER_SIZE must be an integer from 0 to 10, got {border_size}"
        ))
    })?;
    if border_size_value > 10 {
        return Err(ThemeError::InvalidStyle(format!(
            "BORDER_SIZE must be from 0 to 10, got {border_size_value}"
        )));
    }

    if !theme.vars.contains_key("BORDER_COLOR") {
        let fallback = theme.require("BASE02")?.to_string();
        theme.vars.insert("BORDER_COLOR".to_string(), fallback);
    }
    hex_to_rgb(theme.require("BORDER_COLOR")?)?;
    Ok(())
}

pub fn apply_amoled_palette(theme: &mut Theme) {
    theme.vars.insert("VARIANT".to_string(), "dark".to_string());
    theme.vars.insert("AMOLED".to_string(), "true".to_string());

    // True-black base with small, neutral elevation steps. Foregrounds are
    // replaced as well because light themes otherwise carry dark text into
    // the black override. Accent colors remain theme-specific.
    for (key, value) in [
        ("BASE00", "#000000"),
        ("BASE01", "#040404"),
        ("BASE02", "#0a0a0a"),
        ("BASE03", "#181818"),
        ("BASE04", "#8a8a8a"),
        ("BASE05", "#d8d8d8"),
        ("BASE06", "#eeeeee"),
        ("BASE07", "#ffffff"),
        ("BLACK", "#000000"),
        ("DARKER_BLACK", "#000000"),
        ("BLACK2", "#080808"),
        ("ONE_BG", "#040404"),
        ("ONE_BG2", "#0a0a0a"),
        ("ONE_BG3", "#141414"),
        ("GREY", "#686868"),
        ("GREY_FG", "#8a8a8a"),
        ("GREY_FG2", "#a8a8a8"),
        ("LIGHT_GREY", "#b8b8b8"),
        ("WHITE", "#d8d8d8"),
        ("LINE", "#181818"),
        ("STATUSLINE_BG", "#080808"),
        ("LIGHTBG", "#0a0a0a"),
        ("PMENU_BG", "#101010"),
        ("FOLDER_BG", "#141414"),
    ] {
        theme.vars.insert(key.to_string(), value.to_string());
    }
}

// ---------------------------------------------------------------------------
// Apply all writers
// ---------------------------------------------------------------------------

pub fn apply_theme(theme: &Theme) {
    // Env files first so shells and GUI apps see them before anything else.
    run_writer(&writers::FishConf, theme);
    run_writer(&writers::EnvSh, theme);
    run_writer(&writers::EnvSystemd, theme);
    run_writer(&writers::HyprEnv, theme);

    for writer in writers::all_writers(theme) {
        run_writer(writer.as_ref(), theme);
    }

    writers::reload_nvim(theme);
    writers::apply_system_variant(theme);
    // Global env (systemd/dbus/hypr) so new processes inherit the palette.
    writers::apply_global_env(theme);
    // Quickshell watches the published snapshot and draws the wallpaper.
    if let Err(error) = api::publish(theme) {
        eprintln!("theme-manager: [api] publish failed: {error}");
    }
}

fn run_writer(writer: &dyn Writer, theme: &Theme) {
    if !writer.enabled() {
        eprintln!("theme-manager: [{}] skipped (not installed)", writer.name());
        return;
    }
    let path = writer.output_path();
    match writer.generate(theme) {
        Err(e) => {
            eprintln!("theme-manager: [{}] generate failed: {e}", writer.name());
        }
        Ok(content) => match write_file(&path, &content) {
            Err(e) => eprintln!(
                "theme-manager: [{}] write to {} failed: {e}",
                writer.name(),
                path.display()
            ),
            Ok(()) => {
                eprintln!(
                    "theme-manager: [{}] wrote {}",
                    writer.name(),
                    path.display()
                );
                writer.reload();
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        app_enabled, apply_amoled_palette, apply_style_defaults, binary_exists,
        get_transparency, themes_dir, Theme, Writer,
    };
    use crate::writers::{Ghostty, Swaync, Vicinae};
    use std::collections::HashMap;

    fn theme(variant: &str) -> Theme {
        Theme {
            name: format!("{variant}-test"),
            path: format!("/tmp/{variant}-test").into(),
            vars: HashMap::from([
                ("VARIANT".to_string(), variant.to_string()),
                (
                    "BASE00".to_string(),
                    if variant == "light" {
                        "#f8f8f8"
                    } else {
                        "#202020"
                    }
                    .to_string(),
                ),
                (
                    "BASE05".to_string(),
                    if variant == "light" {
                        "#202020"
                    } else {
                        "#d0d0d0"
                    }
                    .to_string(),
                ),
                ("BASE0D".to_string(), "#336699".to_string()),
                ("BLUE".to_string(), "#336699".to_string()),
            ]),
        }
    }

    #[test]
    fn amoled_converts_light_and_dark_themes_to_readable_true_black() {
        for variant in ["light", "dark"] {
            let mut theme = theme(variant);
            apply_amoled_palette(&mut theme);
            assert_eq!(theme.vars["VARIANT"], "dark");
            assert_eq!(theme.vars["AMOLED"], "true");
            assert_eq!(theme.vars["BASE00"], "#000000");
            assert_eq!(theme.vars["BLACK"], "#000000");
            assert_eq!(theme.vars["BASE05"], "#d8d8d8");
            assert_eq!(theme.vars["ONE_BG"], "#040404");
            assert_eq!(theme.vars["ONE_BG3"], "#141414");
        }
    }

    #[test]
    fn amoled_preserves_theme_accent_identity() {
        let mut theme = theme("light");
        apply_amoled_palette(&mut theme);
        assert_eq!(theme.vars["BASE0D"], "#336699");
        assert_eq!(theme.vars["BLUE"], "#336699");
    }

    #[test]
    fn style_defaults_are_complete_and_theme_overrides_are_preserved() {
        let mut default_theme = theme("dark");
        default_theme
            .vars
            .insert("BASE02".to_string(), "#445566".to_string());
        apply_style_defaults(&mut default_theme).unwrap();
        assert_eq!(default_theme.vars["CORNER_RADIUS"], "20");
        assert_eq!(default_theme.vars["BORDER_SIZE"], "0");
        assert_eq!(default_theme.vars["BORDER_COLOR"], "#445566");

        let mut custom_theme = theme("dark");
        custom_theme.vars.extend([
            ("CORNER_RADIUS".to_string(), "6".to_string()),
            ("BORDER_SIZE".to_string(), "2".to_string()),
            ("BORDER_COLOR".to_string(), "#abcdef".to_string()),
        ]);
        apply_style_defaults(&mut custom_theme).unwrap();
        assert_eq!(custom_theme.vars["CORNER_RADIUS"], "6");
        assert_eq!(custom_theme.vars["BORDER_SIZE"], "2");
        assert_eq!(custom_theme.vars["BORDER_COLOR"], "#abcdef");
    }

    #[test]
    fn transparency_accepts_floats_booleans_and_defaults_opaque() {
        let with_val = |val: Option<&str>| {
            let mut t = theme("dark");
            if let Some(v) = val {
                t.vars.insert("TRANSPARENT".to_string(), v.to_string());
            } else {
                t.vars.remove("TRANSPARENT");
            }
            get_transparency(&t)
        };
        assert_eq!(with_val(Some("0.5")), 0.5);
        assert_eq!(with_val(Some(" 0.9 ")), 0.9);
        assert_eq!(with_val(Some("true")), 0.9);
        assert_eq!(with_val(Some("TRUE")), 0.9);
        assert_eq!(with_val(Some("false")), 1.0);
        assert_eq!(with_val(Some("garbage")), 1.0);
        assert_eq!(with_val(None), 1.0);
        // Out-of-range floats clamp instead of leaking into configs.
        assert_eq!(with_val(Some("2.0")), 1.0);
        assert_eq!(with_val(Some("-1.0")), 0.0);
    }

    #[test]
    fn themes_live_under_config_without_dots_dependency() {
        assert_eq!(
            themes_dir(),
            super::home().join(".config/theme-manager/themes")
        );
    }

    #[test]
    fn binary_detection_matches_path_lookup() {
        // `sh` is expected on every POSIX test host; the bogus name
        // must never resolve.
        assert!(binary_exists("sh"));
        assert!(!binary_exists("theme-manager-definitely-not-a-real-binary"));
    }

    #[test]
    fn app_gate_passes_installed_apps_and_blocks_missing_ones() {
        // Binary present -> enabled even without a config dir.
        assert!(app_enabled("sh", ".config/theme-manager-no-such-dir-xyz"));
        // Neither binary nor config dir -> disabled (no bloat).
        assert!(!app_enabled(
            "theme-manager-definitely-not-a-real-binary",
            ".config/theme-manager-no-such-dir-xyz"
        ));
    }

    #[test]
    fn conditional_writers_point_at_real_config_paths() {
        use crate::writers::{Gtk3Css, Gtk4Css, Gtk4Settings, GtkSettings, HyprEnv, Hyprland, Kitty, Mango, Qt6ct, Zathura};
        let home = super::home();
        assert_eq!(
            Ghostty.output_path(),
            home.join(".config/ghostty/themes/active")
        );
        assert_eq!(Hyprland.output_path(), home.join(".config/hypr/colors.lua"));
        assert_eq!(Mango.output_path(), home.join(".config/mango/theme.conf"));
        assert_eq!(HyprEnv.output_path(), home.join(".config/hypr/theme_env.lua"));
        // Global env files: POSIX for all sh-compatible shells, systemd for GUI apps.
        assert_eq!(
            crate::env_sh_file(),
            home.join(".config/theme-manager/theme_env.sh")
        );
        assert_eq!(
            crate::env_systemd_file(),
            home.join(".config/environment.d/theme-manager.conf")
        );
        assert_eq!(
            Kitty.output_path(),
            home.join(".config/kitty/themes/active.conf")
        );
        assert_eq!(
            Swaync.output_path(),
            home.join(".config/swaync/theme-vars.css")
        );
        assert_eq!(
            Vicinae { name: "x".into() }.output_path(),
            home.join(".config/vicinae/themes/x.toml")
        );
        assert_eq!(
            Zathura.output_path(),
            home.join(".config/zathura/themes/active")
        );
        // GTK/Qt writers own clearly-named snippets, never the real configs.
        assert_eq!(
            GtkSettings.output_path(),
            home.join(".config/gtk-3.0/theme-manager.ini")
        );
        assert_eq!(
            Gtk4Settings.output_path(),
            home.join(".config/gtk-4.0/theme-manager.ini")
        );
        assert_eq!(Gtk3Css.output_path(), home.join(".local/share/themes/ThemeManager/gtk-3.0/gtk.css"));
        assert_eq!(Gtk4Css.output_path(), home.join(".local/share/themes/ThemeManager/gtk-4.0/gtk.css"));
        assert_eq!(
            Qt6ct.output_path(),
            home.join(".config/qt6ct/theme-manager.conf")
        );
    }

    #[test]
    fn every_writer_emits_a_manager_owned_file_with_import_tutorial() {
        use crate::writers::{
            EnvSh, EnvSystemd, FishConf, Gtk3Css, Gtk4Css, Gtk4Settings, GtkSettings,
            HyprEnv, Hyprland, Kitty, Mango, NvimPatch, Qt6ct, Zathura,
        };

        let mut t = theme("dark");
        for (key, color) in [
            ("BASE01", "#111111"),
            ("BASE02", "#222222"),
            ("BASE03", "#333333"),
            ("BASE07", "#eeeeee"),
            ("BASE08", "#888888"),
            ("BASE0A", "#aaaaaa"),
            ("BASE0B", "#bbbbbb"),
            ("BASE0C", "#cccccc"),
            ("BASE0E", "#dddddd"),
            ("BLACK", "#111111"),
            ("DARKER_BLACK", "#0a0a0a"),
            ("BLACK2", "#222222"),
            ("ONE_BG", "#222222"),
            ("ONE_BG2", "#333333"),
            ("ONE_BG3", "#444444"),
            ("CYAN", "#cccccc"),
            ("WHITE", "#eeeeee"),
            ("BLUE", "#336699"),
            ("RED", "#ff0000"),
            ("TEAL", "#00dddd"),
            ("GREEN", "#00ff00"),
            ("BABY_PINK", "#ff99cc"),
            ("PINK", "#ff44aa"),
            ("YELLOW", "#ffff00"),
            ("PURPLE", "#aa00ff"),
            ("GREY", "#777777"),
        ] {
            t.vars.insert(key.into(), color.into());
        }
        apply_style_defaults(&mut t).unwrap();

        let vicinae = Vicinae {
            name: "test".into(),
        };
        let outputs: Vec<(&str, String, &str)> = vec![
            ("ghostty", Ghostty.generate(&t).unwrap(), "config-file"),
            ("hyprland", Hyprland.generate(&t).unwrap(), "require"),
            ("mango", Mango.generate(&t).unwrap(), "source_optional"),
            (
                "kitty",
                Kitty.generate(&t).unwrap(),
                "include themes/active.conf",
            ),
            ("swaync", Swaync.generate(&t).unwrap(), "@import"),
            (
                "vicinae",
                vicinae.generate(&t).unwrap(),
                "vicinae theme set",
            ),
            ("zathura", Zathura.generate(&t).unwrap(), "include"),
            ("gtk", GtkSettings.generate(&t).unwrap(), "settings.ini"),
            ("gtk4", Gtk4Settings.generate(&t).unwrap(), "settings.ini"),
            ("gtk3-css", Gtk3Css.generate(&t).unwrap(), "GTK_THEME"),
            ("gtk4-css", Gtk4Css.generate(&t).unwrap(), "GTK_THEME"),
            ("qt6ct", Qt6ct.generate(&t).unwrap(), "qt6ct.conf"),
            ("fish", FishConf.generate(&t).unwrap(), "conf.d"),
            ("env-sh", EnvSh.generate(&t).unwrap(), "theme_env.sh"),
            ("env-systemd", EnvSystemd.generate(&t).unwrap(), "environment.d"),
            ("hypr-env", HyprEnv.generate(&t).unwrap(), "theme_env"),
            ("nvim-patch", NvimPatch.generate(&t).unwrap(), "dofile"),
        ];
        for (name, output, marker) in outputs {
            assert!(
                output.contains("Generated by theme-manager"),
                "{name} is missing the generated-file header"
            );
            assert!(
                output.contains("To use"),
                "{name} is missing the import tutorial"
            );
            assert!(
                output.contains(marker),
                "{name} tutorial is missing its import line ({marker})"
            );
        }
    }

    #[test]
    fn swaync_emits_vars_only_and_never_touches_user_rules() {
        let mut t = theme("dark");
        for (key, color) in [
            ("ONE_BG", "#222222"),
            ("ONE_BG2", "#333333"),
            ("ONE_BG3", "#444444"),
            ("BLACK2", "#222222"),
            ("DARKER_BLACK", "#0a0a0a"),
            ("CYAN", "#cccccc"),
            ("WHITE", "#eeeeee"),
            ("BASE03", "#333333"),
            ("BLUE", "#336699"),
        ] {
            t.vars.insert(key.into(), color.into());
        }
        let out = Swaync.generate(&t).unwrap();
        assert!(out.contains(":root"));
        assert!(out.contains("theme-vars.css"));
    }
}
