//! Darkly GTK 3/4 widget coverage, recolored from theme-manager palettes.
//! Original Darkly GTK CSS: https://github.com/wrymt/darkly-gtk (LGPL-2.1-or-later).
use crate::{hex_to_rgb, Theme, ThemeError, Writer};
use std::path::PathBuf;

pub struct Gtk3Css;
pub struct Gtk4Css;

impl Writer for Gtk3Css {
    fn name(&self) -> &'static str { "gtk3-css" }
    fn output_path(&self) -> PathBuf {
        crate::home().join(".local/share/themes/ThemeManager/gtk-3.0/gtk.css")
    }
    fn generate(&self, theme: &Theme) -> Result<String, ThemeError> {
        css(theme, true)
    }
    fn reload(&self) { install_assets("gtk-3.0"); }
}
impl Writer for Gtk4Css {
    fn name(&self) -> &'static str { "gtk4-css" }
    fn output_path(&self) -> PathBuf {
        crate::home().join(".local/share/themes/ThemeManager/gtk-4.0/gtk.css")
    }
    fn generate(&self, theme: &Theme) -> Result<String, ThemeError> {
        css(theme, false)
    }
    fn reload(&self) { install_assets("gtk-4.0"); }
}

fn install_assets(version: &str) {
    let target = crate::home().join(".local/share/themes/ThemeManager").join(version).join("theme-manager-assets");
    if let Err(error) = std::fs::create_dir_all(&target) {
        eprintln!("theme-manager: GTK asset directory: {error}");
        return;
    }
    for (name, data) in [
        ("bullet-symbolic.svg", &include_bytes!("gtk_assets/bullet-symbolic.svg")[..]),
        ("check-symbolic.svg", &include_bytes!("gtk_assets/check-symbolic.svg")[..]),
        ("close-symbolic.svg", &include_bytes!("gtk_assets/close-symbolic.svg")[..]),
        ("dash-symbolic.svg", &include_bytes!("gtk_assets/dash-symbolic.svg")[..]),
        ("maximize-symbolic.svg", &include_bytes!("gtk_assets/maximize-symbolic.svg")[..]),
        ("minimize-symbolic.svg", &include_bytes!("gtk_assets/minimize-symbolic.svg")[..]),
        ("restore-symbolic.svg", &include_bytes!("gtk_assets/restore-symbolic.svg")[..]),
        ("bullet-symbolic.symbolic.png", &include_bytes!("gtk_assets/bullet-symbolic.symbolic.png")[..]),
        ("check-symbolic.symbolic.png", &include_bytes!("gtk_assets/check-symbolic.symbolic.png")[..]),
        ("dash-symbolic.symbolic.png", &include_bytes!("gtk_assets/dash-symbolic.symbolic.png")[..]),
    ] {
        if let Err(error) = std::fs::write(target.join(name), data) {
            eprintln!("theme-manager: GTK asset {name}: {error}");
        }
    }
}

fn palette<'a>(theme: &'a Theme, key: &str, fallback: &'a str) -> Result<&'a str, ThemeError> {
    let color = theme.vars.get(key).map(String::as_str).unwrap_or(fallback);
    hex_to_rgb(color)?;
    Ok(color)
}

fn css(theme: &Theme, gtk3: bool) -> Result<String, ThemeError> {
    let bg = palette(theme, "BASE00", "#20212b")?;
    let surface = palette(theme, "BASE01", bg)?;
    let raised = palette(theme, "BASE02", surface)?;
    let edge = palette(theme, "BORDER_COLOR", palette(theme, "BASE03", raised)?)?;
    let fg = palette(theme, "BASE05", "#e4e6ef")?;
    let muted = palette(theme, "BASE04", fg)?;
    let accent = palette(theme, "BASE0D", "#7299e6")?;
    let danger = palette(theme, "BASE08", "#dc6476")?;
    let success = palette(theme, "BASE0B", accent)?;
    let warning = palette(theme, "BASE09", danger)?;
    let visited = palette(theme, "BASE0E", accent)?;
    let cyan = palette(theme, "BASE0C", accent)?;
    let yellow = palette(theme, "BASE0A", warning)?;
    let orange = palette(theme, "BASE0F", warning)?;
    let (r, g, b) = hex_to_rgb(accent)?;
    let accent_fg = if (r as f64 * 0.299 + g as f64 * 0.587 + b as f64 * 0.114) > 150.0 {
        "#111318"
    } else { "#ffffff" };
    let variant = theme.vars.get("VARIANT").map(String::as_str).unwrap_or("dark");
    let dark = variant == "dark";
    let template = if gtk3 {
        if dark { include_str!("gtk3-dark.css") } else { include_str!("gtk3-light.css") }
    } else { include_str!("gtk4.css") };
    // Darkly defines _breeze colors first, then uses them throughout its widget
    // selectors. Override those definitions before the selectors, not at EOF:
    // GTK's named color resolution is implementation-dependent across versions.
    let colors = [
        ("borders", edge), ("content_view_bg", surface),
        ("error_color_backdrop", danger), ("error_color", danger),
        ("error_color_insensitive_backdrop", muted), ("error_color_insensitive", muted),
        ("insensitive_base_color", raised), ("insensitive_base_fg_color", muted),
        ("insensitive_bg_color", raised), ("insensitive_borders", edge),
        ("insensitive_fg_color", muted), ("insensitive_selected_bg_color", raised),
        ("insensitive_selected_fg_color", muted), ("insensitive_unfocused_bg_color", raised),
        ("insensitive_unfocused_fg_color", muted), ("insensitive_unfocused_selected_bg_color", raised),
        ("insensitive_unfocused_selected_fg_color", muted),
        ("link_color", accent), ("link_visited_color", visited),
        ("success_color_backdrop", success), ("success_color", success),
        ("success_color_insensitive_backdrop", muted), ("success_color_insensitive", muted),
        ("theme_base_color", surface), ("theme_bg_color", bg),
        ("theme_button_background_backdrop", surface),
        ("theme_button_background_backdrop_insensitive", raised),
        ("theme_button_background_insensitive", raised),
        ("theme_button_background_normal", raised),
        ("theme_button_decoration_focus_backdrop", accent),
        ("theme_button_decoration_focus_backdrop_insensitive", muted),
        ("theme_button_decoration_focus", accent),
        ("theme_button_decoration_focus_insensitive", muted),
        ("theme_button_decoration_hover_backdrop", accent),
        ("theme_button_decoration_hover_backdrop_insensitive", muted),
        ("theme_button_decoration_hover", accent),
        ("theme_button_decoration_hover_insensitive", muted),
        ("theme_button_foreground_active_backdrop", fg),
        ("theme_button_foreground_active_backdrop_insensitive", muted),
        ("theme_button_foreground_active", accent_fg),
        ("theme_button_foreground_active_insensitive", muted),
        ("theme_button_foreground_backdrop", fg),
        ("theme_button_foreground_backdrop_insensitive", muted),
        ("theme_button_foreground_insensitive", muted),
        ("theme_button_foreground_normal", fg),
        ("theme_fg_color", fg),
        ("theme_header_background_backdrop", surface),
        ("theme_header_background", surface), ("theme_header_background_light", surface),
        ("theme_header_foreground_backdrop", muted), ("theme_header_foreground", fg),
        ("theme_header_foreground_insensitive_backdrop", muted),
        ("theme_header_foreground_insensitive", muted),
        ("theme_hovering_selected_bg_color", accent),
        ("theme_selected_bg_color", accent), ("theme_selected_fg_color", accent_fg),
        ("theme_text_color", fg),
        ("theme_titlebar_background_backdrop", surface),
        ("theme_titlebar_background", surface), ("theme_titlebar_background_light", surface),
        ("theme_titlebar_foreground_backdrop", muted), ("theme_titlebar_foreground", fg),
        ("theme_titlebar_foreground_insensitive_backdrop", muted),
        ("theme_titlebar_foreground_insensitive", muted),
        ("theme_unfocused_base_color", surface), ("theme_unfocused_bg_color", bg),
        ("theme_unfocused_fg_color", muted),
        ("theme_unfocused_selected_bg_color_alt", raised),
        ("theme_unfocused_selected_bg_color", raised),
        ("theme_unfocused_selected_fg_color", fg),
        ("theme_unfocused_text_color", muted),
        ("theme_unfocused_view_bg_color", surface),
        ("theme_unfocused_view_text_color", muted),
        ("theme_view_active_decoration_color", accent),
        ("theme_view_hover_decoration_color", accent),
        ("tooltip_background", raised), ("tooltip_border", edge),
        ("tooltip_text", fg), ("unfocused_borders", edge),
        ("unfocused_insensitive_borders", edge),
        ("warning_color_backdrop", warning), ("warning_color", warning),
        ("warning_color_insensitive_backdrop", muted),
        ("warning_color_insensitive", muted), ("close_button_bg", danger),
    ];
    let mut result = format!("/* Generated by theme-manager from Darkly GTK (LGPL-2.1-or-later).\n * To use: GTK_THEME=ThemeManager; restart GTK applications.\n */\n");
    // Recolor literal upstream accents and surfaces before injecting named
    // definitions, so palette values identical to upstream literals survive.
    let mut body = template.to_string();
    for (from, to) in [
        ("#4e9a06", success), ("#00a1ec", accent), ("#1b91d5", accent),
        ("#eff0f1", fg), ("#f1f1f1", fg), ("#8b8b8b", muted),
        ("#b3b3b3", muted), ("#b4b4b4", muted), ("#242424", raised),
        ("#2c2c2c", surface), ("#222222", bg), ("#333333", raised),
    ] { body = body.replace(from, to); }
    // GTK 4 imports libadwaita's stock ramps; recolor its public palette too.
    if !gtk3 {
        for (family, color) in [
            ("blue", accent), ("green", success), ("yellow", yellow),
            ("orange", orange), ("red", danger), ("purple", visited),
            ("brown", warning), ("light", fg), ("dark", bg),
            ("cyan", cyan),
        ] {
            for step in 1..=5 {
                let name = format!("@define-color {family}_{step} ");
                if let Some(start) = body.find(&name) {
                    let start = start + name.len();
                    if let Some(end) = body[start..].find(';') {
                        body.replace_range(start..start + end, color);
                    }
                }
                let name = format!("--{family}-{step}: ");
                if let Some(start) = body.find(&name) {
                    let start = start + name.len();
                    if let Some(end) = body[start..].find(';') {
                        body.replace_range(start..start + end, color);
                    }
                }
            }
        }
    }
    // Replace each actual definition; named colors later in Darkly's stylesheet
    // reference these names. This also handles GTK 4's color-scheme media rules.
    for (name, value) in colors {
        let prefix = format!("@define-color {name}_breeze ");
        let start = body.find(&prefix).ok_or_else(|| ThemeError::InvalidStyle(format!("Darkly color missing: {name}")))? + prefix.len();
        let end = start + body[start..].find(';').ok_or_else(|| ThemeError::InvalidStyle(format!("Darkly color unterminated: {name}")))?;
        body.replace_range(start..end, value);
    }
    // Upstream references several PNGs that are not shipped upstream. Drop
    // their complete -gtk-scaled() expressions rather than leaving invalid
    // none placeholders inside that function.
    for name in ["slider-horz-scale-has-marks-above", "text-select-start", "text-select-end"] {
        let needle = format!("-gtk-scaled(url(\"darkly-gtk-assets/{name}");
        while let Some(offset) = body.find(&needle) {
            let end = offset + body[offset..].find("))").ok_or_else(|| ThemeError::InvalidStyle("unterminated Darkly image".into()))? + 2;
            body.replace_range(offset..end, "none");
        }
    }
    body = body.replace("darkly-gtk-assets/", "theme-manager-assets/");
    // Darkly GTK3 resets padding on *; keep its widget-specific rules, then
    // restore a desktop-friendly 4/8px rhythm for interactive controls.
    // Avoid broad containers and views: LibreOffice draws much of its UI via
    // VCL, and changing their box metrics can clip content or alter grids.
    let spacing = if gtk3 {
        r#"
/* Desktop spacing: Material 3's 8dp rhythm, adapted to dense toolbars. */
toolbar, .toolbar { padding: 4px 8px; }
toolbar button, .toolbar button, .primary-toolbar button {
    min-height: 28px; min-width: 28px; padding: 4px 8px; margin: 0 2px;
}
toolbar separator, .toolbar separator { margin: 4px 6px; }
menu menuitem, .menu menuitem, .context-menu menuitem {
    min-height: 24px; padding: 6px 12px;
}
menu separator, .menu separator { margin: 4px 8px; }
notebook > header tab { min-height: 28px; padding: 6px 12px; }
button:not(.flat):not(.titlebutton), modelbutton { min-height: 28px; padding: 6px 12px; }
entry, spinbutton, combobox > box, treeview entry { min-height: 28px; }
popover > contents { padding: 8px; }
"#
    } else {
        r#"
/* GTK4: same spacing rhythm, without GTK3's universal reset. */
toolbar { padding: 4px 8px; }
toolbar button { min-height: 28px; min-width: 28px; padding: 4px 8px; margin: 0 2px; }
toolbar separator { margin: 4px 6px; }
menu menuitem { min-height: 24px; padding: 6px 12px; }
notebook > header tab { min-height: 28px; padding: 6px 12px; }
button:not(.flat):not(.titlebutton), modelbutton { min-height: 28px; padding: 6px 12px; }
entry, spinbutton, dropdown { min-height: 28px; }
popover > contents { padding: 8px; }
"#
    };
    let radius = theme.vars.get("CORNER_RADIUS").and_then(|v| v.parse::<u8>().ok()).unwrap_or(12).min(64);
    result.push_str(&body);
    result.push_str(spacing);
    result.push_str(&format!("\n/* Theme-manager geometry and selection overrides. */\nwindow,dialog,popover,menu {{border-radius:{radius}px}}\nselection,entry selection,textview text selection {{background-color:{accent};color:{accent_fg}}}\n"));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    #[test]
    fn darkly_coverage_tracks_dark_light_and_accent() {
        let mut theme = Theme { name: "test".into(), path: "/tmp/test".into(), vars: HashMap::new() };
        theme.vars.extend([("BASE00".into(), "#101020".into()), ("BASE01".into(), "#202030".into()), ("BASE05".into(), "#eeeeee".into()), ("BASE0D".into(), "#dddddd".into()), ("CORNER_RADIUS".into(), "18".into())]);
        for writer in [&Gtk3Css as &dyn Writer, &Gtk4Css as &dyn Writer] {
            let out = writer.generate(&theme).unwrap();
            for expected in ["#101020", "#dddddd", "#111318", "border-radius:18px", "scrollbar", "popover", "notebook", "switch", "treeview", "theme-manager-assets/"] {
                assert!(out.contains(expected), "missing {expected}");
            }
            assert!(out.contains("min-height: 28px"));
            assert!(out.contains("padding: 6px 12px"));
            assert!(out.contains("toolbar button"));
            assert!(!out.contains("treeview { padding:"));
            assert!(out.len() > 150_000);
            assert!(!out.contains("darkly-gtk-assets/"));
            assert!(!out.contains("-gtk-scaled(none"));
        }
        theme.vars.insert("VARIANT".into(), "light".into());
        theme.vars.insert("BASE0D".into(), "#223355".into());
        let light = Gtk3Css.generate(&theme).unwrap();
        assert!(light.contains("#223355"));
        assert!(light.contains("#ffffff"));
        theme.vars.insert("BASE00".into(), "broken".into());
        assert!(Gtk3Css.generate(&theme).is_err());
    }
}
