use crate::{env_sh_file, fish_conf_file, Theme, ThemeError};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SIZE: usize = 96;

// Matugen accent generation: hues come from the wallpaper itself.
const MIN_ACCENT_SATURATION: f32 = 0.18;
// Second pass for muted wallpapers that have a tint but no saturated colors.
const RELAXED_ACCENT_SATURATION: f32 = 0.05;
const MIN_HUE_GAP: f32 = 25.0;
const MAX_WALLPAPER_HUES: usize = 4;
const HUE_STEP: f32 = 12.0;
const LIGHTNESS_STEP: f32 = 0.05;
const LIGHTNESS_RANGE: f32 = 0.15;
// Wallpaper brightness (0..=1) assumed when it can't be measured: no softening.
const NEUTRAL_BRIGHTNESS: f32 = 0.5;
// Largest share of a foreground color replaced by the background, reached on a
// wallpaper as extreme as the variant itself (pitch black for dark, white for light).
// Softening stops early when a color would fall below its minimum contrast.
const MAX_SOFTEN: f32 = 0.85;
// WCAG contrast ratios against the background: body text, then accents and the
// secondary/comment gray.
const MIN_TEXT_CONTRAST: f32 = 7.0;
const MIN_ACCENT_CONTRAST: f32 = 4.5;

/// Material 3 scheme names accepted by `MATUGEN_SCHEME`, matching matugen's
/// `--type` values. Both `rainbow` and `scheme-rainbow` spellings are accepted.
const MATUGEN_SCHEMES: &[&str] = &[
    "content",
    "expressive",
    "fidelity",
    "fruit-salad",
    "monochrome",
    "neutral",
    "rainbow",
    "tonal-spot",
    "vibrant",
    "smart",
];

pub fn resolve_dynamic_palette(theme: &mut Theme) -> Result<(), ThemeError> {
    if theme.vars.get("MODE").map(String::as_str) != Some("dynamic") {
        return Ok(());
    }

    // WALLPAPER=none is an explicit request to clear the backdrop. Keep the
    // last resolved colors (there is no image to sample), but never replace
    // the value with `current_wallpaper()`: that would undo the removal and
    // repaint the old image.
    let explicit_none = theme.vars.get("WALLPAPER").is_some_and(|value| {
        let value = value.trim();
        value.is_empty() || value.eq_ignore_ascii_case("none")
    });
    if explicit_none {
        let content = fs::read_to_string(env_sh_file()).unwrap_or_default();
        retain_current_palette(theme, &content)?;
        theme.vars.insert("WALLPAPER".into(), "none".into());
        return Ok(());
    }

    // A stale wallpaper must not prevent selecting a dynamic theme. Retain
    // the last published palette and wallpaper until a new source is assigned.
    let wallpaper = match resolve_wallpaper(theme) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("theme-manager: {error}; retaining current colors");
            let content = fs::read_to_string(env_sh_file()).unwrap_or_default();
            retain_current_palette(theme, &content)?;
            if let Some(path) = current_wallpaper() {
                theme
                    .vars
                    .insert("WALLPAPER".into(), path.to_string_lossy().into_owned());
            } else {
                theme.vars.insert("WALLPAPER".into(), "none".into());
            }
            return Ok(());
        }
    };
    theme.vars.insert(
        "WALLPAPER".to_string(),
        wallpaper.to_string_lossy().into_owned(),
    );
    let variant = theme.require("VARIANT")?;
    let generated = if let Some(scheme) = theme.vars.get("MATUGEN_SCHEME") {
        let scheme = normalize_matugen_scheme(scheme)?;
        // Without decodable pixels: matugen's primary hue and no brightness softening.
        let (hues, brightness) = match decode_representative_frame(&wallpaper) {
            Ok(pixels) => (wallpaper_hues(&pixels), wallpaper_brightness(&pixels)),
            Err(error) => {
                eprintln!("theme-manager: {error}; using matugen primary, default brightness");
                (Vec::new(), NEUTRAL_BRIGHTNESS)
            }
        };
        generate_matugen_palette(&wallpaper, variant, &scheme, &hues, brightness)?
    } else {
        let pixels = decode_representative_frame(&wallpaper)?;
        generate_palette(&pixels, variant)?
    };

    // Explicit colors remain valid overrides. Dynamic mode only fills keys
    // omitted from the theme file.
    for (key, value) in generated {
        theme.vars.entry(key).or_insert(value);
    }

    Ok(())
}

fn retain_current_palette(theme: &mut Theme, content: &str) -> Result<(), ThemeError> {
    let defaults = generate_palette(&[Rgb(98, 114, 164); 4], theme.require("VARIANT")?)?;
    let current = parse_published_colors(content);
    for (key, default) in defaults {
        // Preserve current colors even if the new dynamic theme has overrides:
        // without a source image, selecting it is not a request to recolor.
        let value = current
            .get(&key)
            .cloned()
            .or_else(|| theme.vars.get(&key).cloned())
            .unwrap_or(default);
        theme.vars.insert(key, value);
    }
    for (key, value) in current {
        theme.vars.insert(key, value);
    }
    Ok(())
}

fn parse_published_colors(content: &str) -> HashMap<String, String> {
    content
        .lines()
        .filter_map(|line| {
            let (key, value) = line.trim().strip_prefix("export THEME_")?.split_once('=')?;
            let value = value.trim().trim_matches('\'');
            if crate::hex_to_rgb(value).is_ok() {
                Some((key.to_string(), value.to_string()))
            } else {
                None
            }
        })
        .collect()
}

fn resolve_wallpaper(theme: &Theme) -> Result<PathBuf, ThemeError> {
    let configured = theme
        .vars
        .get("WALLPAPER")
        .map(String::as_str)
        .filter(|path| !path.is_empty() && *path != "none")
        .ok_or_else(|| ThemeError::Dynamic("MODE=dynamic requires WALLPAPER".to_string()))?;

    let value = if configured == "current" {
        current_wallpaper().ok_or_else(|| {
            ThemeError::Dynamic(
                "WALLPAPER=current but no active wallpaper could be found".to_string(),
            )
        })?
    } else {
        PathBuf::from(configured)
    };

    if !value.is_file() {
        return Err(ThemeError::Dynamic(format!(
            "dynamic wallpaper '{}' does not exist",
            value.display()
        )));
    }
    Ok(value)
}

pub fn current_wallpaper() -> Option<PathBuf> {
    // Last switch wins: fish, POSIX, then the live process environment
    // (now global, so non-fish processes resolve here too).
    if let Ok(content) = fs::read_to_string(fish_conf_file()) {
        if let Some(path) = parse_fish_wallpaper(&content).filter(|path| path.is_file()) {
            return Some(path);
        }
    }

    if let Ok(content) = fs::read_to_string(env_sh_file()) {
        if let Some(path) = parse_sh_wallpaper(&content).filter(|path| path.is_file()) {
            return Some(path);
        }
    }

    if let Ok(value) = std::env::var("THEME_WALLPAPER") {
        let value = PathBuf::from(value);
        if value.is_file() {
            return Some(value);
        }
    }
    None
}

fn parse_fish_wallpaper(content: &str) -> Option<PathBuf> {
    let prefix = "set -gx THEME_WALLPAPER '";
    content.lines().find_map(|line| {
        let value = line.strip_prefix(prefix)?.strip_suffix('\'')?;
        Some(PathBuf::from(value.replace("'\\''", "'")))
    })
}

fn parse_sh_wallpaper(content: &str) -> Option<PathBuf> {
    let prefix = "export THEME_WALLPAPER='";
    content.lines().find_map(|line| {
        let value = line.strip_prefix(prefix)?.strip_suffix('\'')?;
        Some(PathBuf::from(value.replace("'\\''", "'")))
    })
}

fn decode_representative_frame(path: &Path) -> Result<Vec<Rgb>, ThemeError> {
    let filter = if is_animated_media(path) {
        "thumbnail=30,scale=96:96:force_original_aspect_ratio=increase,crop=96:96"
    } else {
        "scale=96:96:force_original_aspect_ratio=increase,crop=96:96"
    };
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            filter,
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| ThemeError::Dynamic(format!("could not run ffmpeg: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ThemeError::Dynamic(format!(
            "ffmpeg could not decode '{}': {}",
            path.display(),
            stderr.trim()
        )));
    }

    let expected = SIZE * SIZE * 3;
    if output.stdout.len() != expected {
        return Err(ThemeError::Dynamic(format!(
            "ffmpeg returned {} bytes for '{}', expected {expected}",
            output.stdout.len(),
            path.display()
        )));
    }

    Ok(output
        .stdout
        .chunks_exact(3)
        .map(|pixel| Rgb(pixel[0], pixel[1], pixel[2]))
        .collect())
}

fn is_animated_media(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some("gif" | "webp" | "mp4" | "m4v" | "mkv" | "mov" | "webm" | "avi" | "mpg" | "mpeg")
    )
}

fn generate_palette(pixels: &[Rgb], variant: &str) -> Result<HashMap<String, String>, ThemeError> {
    let dark = match variant {
        "dark" => true,
        "light" => false,
        value => return Err(ThemeError::InvalidVariant(value.to_string())),
    };
    let candidates = dominant_colors(pixels);
    let seed = candidates.first().copied().unwrap_or(Rgb(98, 114, 164));
    let seed_hsl = rgb_to_hsl(seed);

    let background = |lightness| hsl_to_rgb(Hsl(seed_hsl.0, seed_hsl.1.min(0.22), lightness));
    let foreground = |lightness| hsl_to_rgb(Hsl(seed_hsl.0, seed_hsl.1.min(0.16), lightness));
    let accent = |hue: f32| select_accent(&candidates, hue, dark);

    let (base00, base01, base02, base03, base04, base05, base06, base07) = if dark {
        (
            background(0.08),
            background(0.11),
            background(0.17),
            background(0.28),
            foreground(0.48),
            foreground(0.72),
            foreground(0.82),
            foreground(0.90),
        )
    } else {
        (
            background(0.96),
            background(0.92),
            background(0.84),
            background(0.66),
            foreground(0.46),
            foreground(0.28),
            foreground(0.20),
            foreground(0.12),
        )
    };

    let red = accent(5.0);
    let orange = accent(30.0);
    let yellow = accent(52.0);
    let green = accent(125.0);
    let teal = accent(170.0);
    let cyan = accent(190.0);
    let blue = accent(220.0);
    let purple = accent(280.0);
    let pink = accent(330.0);
    let baby_pink = mix(pink, base07, 0.35);

    let mut palette = HashMap::new();
    for (key, color) in [
        ("BASE00", base00),
        ("BASE01", base01),
        ("BASE02", base02),
        ("BASE03", base03),
        ("BASE04", base04),
        ("BASE05", base05),
        ("BASE06", base06),
        ("BASE07", base07),
        ("BASE08", red),
        ("BASE09", orange),
        ("BASE0A", yellow),
        ("BASE0B", green),
        ("BASE0C", cyan),
        ("BASE0D", blue),
        ("BASE0E", purple),
        ("BASE0F", pink),
        ("BLACK", base00),
        ("DARKER_BLACK", base01),
        ("BLACK2", base02),
        ("ONE_BG", base01),
        ("ONE_BG2", base02),
        ("ONE_BG3", base03),
        ("GREY", base03),
        ("WHITE", base05),
        ("RED", red),
        ("GREEN", green),
        ("YELLOW", yellow),
        ("BLUE", blue),
        ("CYAN", cyan),
        ("TEAL", teal),
        ("PINK", pink),
        ("BABY_PINK", baby_pink),
        ("PURPLE", purple),
        ("LIGHT_GREY", base04),
    ] {
        palette.insert(key.to_string(), color.to_hex());
    }
    add_base30_compatibility(
        &mut palette,
        dark,
        base00,
        base01,
        base02,
        base03,
        base04,
        green,
        blue,
        yellow,
        purple,
        orange,
    );
    Ok(palette)
}

/// NvChad/base46 expects more `base_30` keys than the 16-color palette
/// provides. Fill them for every dynamic palette so chadrc does not fall back
/// to the built-in onedark theme when theme-manager generated the colors.
#[allow(clippy::too_many_arguments)]
fn add_base30_compatibility(
    palette: &mut HashMap<String, String>,
    dark: bool,
    base00: Rgb,
    base01: Rgb,
    base02: Rgb,
    base03: Rgb,
    base04: Rgb,
    green: Rgb,
    blue: Rgb,
    yellow: Rgb,
    purple: Rgb,
    orange: Rgb,
) {
    // Light themes need the opposite direction: accents must get darker to
    // stay readable on a bright background.
    let highlight = |color: Rgb| {
        mix(
            color,
            if dark {
                Rgb(255, 255, 255)
            } else {
                Rgb(0, 0, 0)
            },
            0.20,
        )
    };
    let shadow = |color: Rgb| {
        mix(
            color,
            if dark {
                Rgb(0, 0, 0)
            } else {
                Rgb(255, 255, 255)
            },
            0.25,
        )
    };
    for (key, color) in [
        ("GREY_FG", base04),
        ("GREY_FG2", highlight(base04)),
        (
            "LINE",
            if dark {
                mix(base02, base00, 0.35)
            } else {
                base03
            },
        ),
        ("VIBRANT_GREEN", highlight(green)),
        ("NORD_BLUE", highlight(blue)),
        ("SUN", highlight(yellow)),
        ("DARK_PURPLE", shadow(purple)),
        ("ORANGE", orange),
        (
            "STATUSLINE_BG",
            if dark {
                mix(base01, base00, 0.35)
            } else {
                base01
            },
        ),
        ("LIGHTBG", if dark { base02 } else { base01 }),
        ("PMENU_BG", base02),
        ("FOLDER_BG", base04),
    ] {
        palette.insert(key.to_string(), color.to_hex());
    }
}

fn normalize_matugen_scheme(value: &str) -> Result<String, ThemeError> {
    let value = value.trim();
    let name = value.strip_prefix("scheme-").unwrap_or(value);
    if MATUGEN_SCHEMES.contains(&name) {
        Ok(format!("scheme-{name}"))
    } else {
        Err(ThemeError::InvalidStyle(format!(
            "unknown MATUGEN_SCHEME '{value}' (use one of: {})",
            MATUGEN_SCHEMES.join(", ")
        )))
    }
}

fn generate_matugen_palette(
    wallpaper: &Path,
    variant: &str,
    scheme: &str,
    hues: &[Hsl],
    brightness: f32,
) -> Result<HashMap<String, String>, ThemeError> {
    let output = Command::new("matugen")
        .args(["image"])
        .arg(wallpaper)
        .args([
            "-m",
            variant,
            "-t",
            scheme,
            "-j",
            "hex",
            "--dry-run",
            "-q",
            "--source-color-index",
            "0",
        ])
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| ThemeError::Dynamic(format!("could not run matugen: {error}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ThemeError::Dynamic(format!(
            "matugen could not generate a {scheme} palette for '{}': {}",
            wallpaper.display(),
            stderr.trim()
        )));
    }

    palette_from_matugen_json(
        &String::from_utf8_lossy(&output.stdout),
        variant,
        hues,
        brightness,
    )
}

/// Read one role from matugen's `colors` object for the requested mode.
fn matugen_color(json: &str, role: &str, mode: &str) -> Option<Rgb> {
    let colors = json.split_once("\"colors\"")?.1;
    let role_key = format!("\"{role}\"");
    let (_, after_role) = colors.split_once(&role_key)?;
    let mode_key = format!("\"{mode}\"");
    let (_, after_mode) = after_role.split_once(&mode_key)?;
    let (_, after_color) = after_mode.split_once("\"color\"")?;
    let (_, after_colon) = after_color.split_once(':')?;
    let hex = after_colon.trim_start().strip_prefix('"')?.split('"').next()?;
    crate::hex_to_rgb(hex)
        .ok()
        .map(|(r, g, b)| Rgb(r, g, b))
}

/// Neutrals come from matugen; every accent is built from `hues` (the
/// wallpaper's own colors), never from fixed hues or Material's red error role.
/// `brightness` (0..=1, see `wallpaper_brightness`) softens foreground colors
/// when the wallpaper is dark (dark variant) or bright (light variant).
fn palette_from_matugen_json(
    json: &str,
    variant: &str,
    hues: &[Hsl],
    brightness: f32,
) -> Result<HashMap<String, String>, ThemeError> {
    let dark = match variant {
        "dark" => true,
        "light" => false,
        value => return Err(ThemeError::InvalidVariant(value.to_string())),
    };
    let role = |name: &str| {
        matugen_color(json, name, variant)
            .ok_or_else(|| ThemeError::Dynamic(format!("matugen output is missing colors.{name}")))
    };

    let background = role("background")?;
    let base01 = role("surface_container_lowest").unwrap_or(background);
    let base02 = role("surface_container_low").unwrap_or(background);
    let base03 = role("surface_container").unwrap_or(background);
    // Pull a foreground color toward the background, keeping `min_contrast`.
    let amount = softness(brightness, dark) * MAX_SOFTEN;
    let soften =
        |color: Rgb, min_contrast: f32| soften_toward(color, background, amount, min_contrast);

    let base04 = soften(role("on_surface_variant")?, MIN_ACCENT_CONTRAST);
    let base05 = soften(role("on_surface")?, MIN_TEXT_CONTRAST);
    let text_tint = if dark {
        Rgb(255, 255, 255)
    } else {
        Rgb(0, 0, 0)
    };
    let base06 = mix(base05, text_tint, 0.10);
    let base07 = mix(base05, text_tint, 0.20);

    let fallback = [rgb_to_hsl(role("primary")?)];
    let hues = if hues.is_empty() { &fallback[..] } else { hues };
    let accents: Vec<Rgb> = (0..9)
        .map(|i| soften(accent_from_hues(hues, i, dark), MIN_ACCENT_CONTRAST))
        .collect();

    let red = accents[0];
    let orange = accents[1];
    let yellow = accents[2];
    let green = accents[3];
    let cyan = accents[4];
    let blue = accents[5];
    let purple = accents[6];
    let pink = accents[7];
    let teal = accents[8];
    let baby_pink = mix(pink, base07, 0.35);

    let mut palette = HashMap::new();
    for (key, color) in [
        ("BASE00", background),
        ("BASE01", base01),
        ("BASE02", base02),
        ("BASE03", base03),
        ("BASE04", base04),
        ("BASE05", base05),
        ("BASE06", base06),
        ("BASE07", base07),
        ("BASE08", red),
        ("BASE09", orange),
        ("BASE0A", yellow),
        ("BASE0B", green),
        ("BASE0C", cyan),
        ("BASE0D", blue),
        ("BASE0E", purple),
        ("BASE0F", pink),
        ("BLACK", background),
        ("DARKER_BLACK", base01),
        ("BLACK2", base02),
        ("ONE_BG", base01),
        ("ONE_BG2", base02),
        ("ONE_BG3", base03),
        ("GREY", base03),
        ("WHITE", base05),
        ("RED", red),
        ("GREEN", green),
        ("YELLOW", yellow),
        ("BLUE", blue),
        ("CYAN", cyan),
        ("TEAL", teal),
        ("PINK", pink),
        ("BABY_PINK", baby_pink),
        ("PURPLE", purple),
        ("LIGHT_GREY", base04),
    ] {
        palette.insert(key.to_string(), color.to_hex());
    }
    add_base30_compatibility(
        &mut palette,
        dark,
        background,
        base01,
        base02,
        base03,
        base04,
        green,
        blue,
        yellow,
        purple,
        orange,
    );
    Ok(palette)
}

/// Mean perceived brightness of the wallpaper, 0 (black) to 1 (white).
fn wallpaper_brightness(pixels: &[Rgb]) -> f32 {
    if pixels.is_empty() {
        return NEUTRAL_BRIGHTNESS;
    }
    let total: f32 = pixels
        .iter()
        .map(|&Rgb(r, g, b)| 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32)
        .sum();
    total / (pixels.len() as f32 * 255.0)
}

/// 0..=1: how far the wallpaper lies past mid-gray toward the variant's own
/// background (darker for dark, brighter for light). Mid-gray or the opposite
/// side gives 0, so ordinary wallpapers keep matugen's tones unchanged.
fn softness(brightness: f32, dark: bool) -> f32 {
    let brightness = brightness.clamp(0.0, 1.0);
    let toward_background = if dark { 1.0 - brightness } else { brightness };
    ((toward_background - 0.5) * 2.0).clamp(0.0, 1.0)
}

/// Mix `color` toward `background` by up to `amount`, using the strongest
/// softening that still keeps `min_contrast` against the background.
fn soften_toward(color: Rgb, background: Rgb, amount: f32, min_contrast: f32) -> Rgb {
    (0..=10)
        .map(|i| mix(color, background, amount * (1.0 - i as f32 / 10.0)))
        .find(|candidate| contrast_ratio(*candidate, background) >= min_contrast)
        .unwrap_or(color)
}

/// WCAG contrast ratio between two colors (1.0 to 21.0).
fn contrast_ratio(a: Rgb, b: Rgb) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn luminance(Rgb(r, g, b): Rgb) -> f32 {
    let linear = |channel: u8| {
        let c = channel as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// Distinct hues of the wallpaper, most dominant first. Prefers saturated
/// colors; a muted wallpaper retries with a lower saturation floor so its
/// faint tint is still used. Empty only for (near-)grayscale wallpapers.
fn wallpaper_hues(pixels: &[Rgb]) -> Vec<Hsl> {
    let candidates: Vec<Hsl> = dominant_colors(pixels).into_iter().map(rgb_to_hsl).collect();
    for min_saturation in [MIN_ACCENT_SATURATION, RELAXED_ACCENT_SATURATION] {
        let mut hues: Vec<Hsl> = Vec::new();
        for hsl in candidates.iter().filter(|c| c.1 >= min_saturation) {
            let distinct = hues.iter().all(|h| hue_distance(h.0, hsl.0) >= MIN_HUE_GAP);
            if distinct && hues.len() < MAX_WALLPAPER_HUES {
                hues.push(*hsl);
            }
        }
        if !hues.is_empty() {
            return hues;
        }
    }
    Vec::new()
}

/// Accent `index`: slots cycle through `hues` (non-empty). Each repeat nudges
/// hue and lightness slightly, so a wallpaper with few colors still gets
/// distinct accents that stay close to its own hues.
fn accent_from_hues(hues: &[Hsl], index: usize, dark: bool) -> Rgb {
    let base = hues[index % hues.len()];
    let round = index / hues.len();
    // Variation steps per round: 0, +1, -1, +2, -2, ...
    let magnitude = ((round + 1) / 2) as f32;
    let step = if round % 2 == 1 { magnitude } else { -magnitude };

    let center = if dark { 0.64 } else { 0.42 };
    let lightness = (center + step * LIGHTNESS_STEP)
        .clamp(center - LIGHTNESS_RANGE, center + LIGHTNESS_RANGE);
    hsl_to_rgb(Hsl(
        (base.0 + step * HUE_STEP).rem_euclid(360.0),
        base.1.clamp(0.25, 0.82),
        lightness,
    ))
}

fn dominant_colors(pixels: &[Rgb]) -> Vec<Rgb> {
    let mut buckets: HashMap<u16, (u32, u64, u64, u64)> = HashMap::new();
    for &Rgb(r, g, b) in pixels {
        let key = ((r as u16 >> 3) << 10) | ((g as u16 >> 3) << 5) | (b as u16 >> 3);
        let entry = buckets.entry(key).or_default();
        entry.0 += 1;
        entry.1 += r as u64;
        entry.2 += g as u64;
        entry.3 += b as u64;
    }

    let mut colors: Vec<(f32, Rgb)> = buckets
        .into_values()
        .filter_map(|(count, r, g, b)| {
            if count < 2 {
                return None;
            }
            let color = Rgb(
                (r / count as u64) as u8,
                (g / count as u64) as u8,
                (b / count as u64) as u8,
            );
            let hsl = rgb_to_hsl(color);
            let usable = 1.0 - ((hsl.2 - 0.5).abs() * 1.6).min(0.85);
            Some((count as f32 * (0.35 + hsl.1) * usable, color))
        })
        .collect();
    colors.sort_by(|a, b| b.0.total_cmp(&a.0));
    colors
        .into_iter()
        .take(24)
        .map(|(_, color)| color)
        .collect()
}

fn select_accent(candidates: &[Rgb], target_hue: f32, dark: bool) -> Rgb {
    let selected = candidates
        .iter()
        .copied()
        .filter_map(|color| {
            let hsl = rgb_to_hsl(color);
            if hsl.1 < 0.18 {
                return None;
            }
            let distance = hue_distance(hsl.0, target_hue);
            Some((distance - hsl.1 * 20.0, hsl))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, hsl)| hsl)
        .filter(|hsl| hue_distance(hsl.0, target_hue) <= 70.0)
        .unwrap_or(Hsl(target_hue, 0.68, 0.5));

    hsl_to_rgb(Hsl(
        selected.0,
        selected.1.clamp(0.48, 0.82),
        if dark { 0.64 } else { 0.42 },
    ))
}

fn hue_distance(a: f32, b: f32) -> f32 {
    let distance = (a - b).abs() % 360.0;
    distance.min(360.0 - distance)
}

fn mix(a: Rgb, b: Rgb, amount: f32) -> Rgb {
    let blend = |x: u8, y: u8| (x as f32 * (1.0 - amount) + y as f32 * amount).round() as u8;
    Rgb(blend(a.0, b.0), blend(a.1, b.1), blend(a.2, b.2))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rgb(u8, u8, u8);

impl Rgb {
    fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

#[derive(Clone, Copy, Debug)]
struct Hsl(f32, f32, f32);

fn rgb_to_hsl(Rgb(r, g, b): Rgb) -> Hsl {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return Hsl(0.0, 0.0, lightness);
    }
    let delta = max - min;
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let hue = if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    Hsl(
        if hue < 0.0 { hue + 360.0 } else { hue },
        saturation,
        lightness,
    )
}

fn hsl_to_rgb(Hsl(h, s, l): Hsl) -> Rgb {
    let chroma = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = chroma * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match h {
        h if h < 60.0 => (chroma, x, 0.0),
        h if h < 120.0 => (x, chroma, 0.0),
        h if h < 180.0 => (0.0, chroma, x),
        h if h < 240.0 => (0.0, x, chroma),
        h if h < 300.0 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = l - chroma / 2.0;
    let channel = |value: f32| ((value + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgb(channel(r), channel(g), channel(b))
}

#[cfg(test)]
mod tests {
    use super::{
        generate_palette, hue_distance, normalize_matugen_scheme, palette_from_matugen_json,
        parse_fish_wallpaper, parse_published_colors, parse_sh_wallpaper, resolve_dynamic_palette,
        contrast_ratio, retain_current_palette, rgb_to_hsl, wallpaper_brightness, wallpaper_hues,
        Rgb, MIN_ACCENT_CONTRAST, MIN_TEXT_CONTRAST, NEUTRAL_BRIGHTNESS,
    };
    use crate::Theme;
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn dynamic_palette_contains_every_required_color() {
        let pixels = vec![Rgb(30, 60, 180); 100];
        let palette = generate_palette(&pixels, "dark").unwrap();
        for key in [
            "BASE00",
            "BASE01",
            "BASE02",
            "BASE03",
            "BASE04",
            "BASE05",
            "BASE06",
            "BASE07",
            "BASE08",
            "BASE09",
            "BASE0A",
            "BASE0B",
            "BASE0C",
            "BASE0D",
            "BASE0E",
            "BASE0F",
            "BLACK",
            "DARKER_BLACK",
            "BLACK2",
            "ONE_BG",
            "ONE_BG2",
            "ONE_BG3",
            "GREY",
            "WHITE",
            "RED",
            "GREEN",
            "YELLOW",
            "BLUE",
            "CYAN",
            "TEAL",
            "PINK",
            "BABY_PINK",
            "PURPLE",
            "LIGHT_GREY",
            "GREY_FG",
            "GREY_FG2",
            "LINE",
            "VIBRANT_GREEN",
            "NORD_BLUE",
            "SUN",
            "DARK_PURPLE",
            "ORANGE",
            "STATUSLINE_BG",
            "LIGHTBG",
            "PMENU_BG",
            "FOLDER_BG",
        ] {
            assert!(palette.contains_key(key), "missing {key}");
        }
    }

    #[test]
    fn dark_and_light_modes_generate_contrasting_backgrounds() {
        let pixels = vec![Rgb(180, 40, 90); 100];
        let dark = generate_palette(&pixels, "dark").unwrap();
        let light = generate_palette(&pixels, "light").unwrap();
        let parse = |value: &str| {
            Rgb(
                u8::from_str_radix(&value[1..3], 16).unwrap(),
                u8::from_str_radix(&value[3..5], 16).unwrap(),
                u8::from_str_radix(&value[5..7], 16).unwrap(),
            )
        };
        assert!(rgb_to_hsl(parse(&dark["BASE00"])).2 < 0.2);
        assert!(rgb_to_hsl(parse(&light["BASE00"])).2 > 0.8);
    }

    #[test]
    fn dynamic_resolution_decodes_media_and_preserves_overrides() {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("theme-manager-dynamic-{id}.ppm"));
        let mut ppm = String::from("P3\n4 4\n255\n");
        for _ in 0..16 {
            ppm.push_str("20 80 210\n");
        }
        fs::write(&path, ppm).unwrap();

        let mut theme = Theme {
            name: "dynamic-test".to_string(),
            path: path.clone(),
            vars: HashMap::from([
                ("VARIANT".to_string(), "dark".to_string()),
                ("MODE".to_string(), "dynamic".to_string()),
                ("WALLPAPER".to_string(), path.to_string_lossy().into_owned()),
                ("RED".to_string(), "#123456".to_string()),
            ]),
        };

        resolve_dynamic_palette(&mut theme).unwrap();
        assert_eq!(theme.vars["RED"], "#123456");
        assert!(theme.vars.contains_key("BASE00"));
        assert!(theme.vars.contains_key("PURPLE"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn dynamic_resolution_uses_a_representative_video_frame() {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("theme-manager-dynamic-{id}.mp4"));
        let status = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=64x64:d=1",
            ])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());

        let mut theme = Theme {
            name: "video-test".to_string(),
            path: path.clone(),
            vars: HashMap::from([
                ("VARIANT".to_string(), "light".to_string()),
                ("MODE".to_string(), "dynamic".to_string()),
                ("WALLPAPER".to_string(), path.to_string_lossy().into_owned()),
            ]),
        };
        resolve_dynamic_palette(&mut theme).unwrap();
        assert_eq!(theme.vars.len(), 49);
        assert!(theme.vars["BASE00"].starts_with('#'));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn parses_the_current_wallpaper_from_generated_fish_environment() {
        let wallpaper = parse_fish_wallpaper(
            "set -gx THEME_NAME 'old-theme'\nset -gx THEME_WALLPAPER '/tmp/current wall.jpg'\n",
        );
        assert_eq!(wallpaper, Some(PathBuf::from("/tmp/current wall.jpg")));
    }

    #[test]
    fn parses_the_current_wallpaper_from_generated_posix_environment() {
        let wallpaper = parse_sh_wallpaper(
            "export THEME_NAME='old-theme'\nexport THEME_WALLPAPER='/tmp/current wall.jpg'\n",
        );
        assert_eq!(wallpaper, Some(PathBuf::from("/tmp/current wall.jpg")));
    }

    #[test]
    fn dynamic_mode_without_wallpaper_can_still_resolve() {
        let mut theme = Theme {
            name: "missing-wallpaper".to_string(),
            path: "missing-wallpaper".into(),
            vars: HashMap::from([
                ("VARIANT".to_string(), "dark".to_string()),
                ("MODE".to_string(), "dynamic".to_string()),
            ]),
        };
        resolve_dynamic_palette(&mut theme).unwrap();
        assert!(theme.vars.contains_key("BASE00"));
        assert!(theme.vars.contains_key("BASE0F"));
    }
    #[test]
    fn explicit_none_wallpaper_clears_without_restoring_the_previous_one() {
        for value in ["none", "NONE", ""] {
            let mut theme = Theme {
                name: "cleared-wallpaper".to_string(),
                path: "cleared-wallpaper".into(),
                vars: HashMap::from([
                    ("VARIANT".to_string(), "dark".to_string()),
                    ("MODE".to_string(), "dynamic".to_string()),
                    ("WALLPAPER".to_string(), value.to_string()),
                ]),
            };
            resolve_dynamic_palette(&mut theme).unwrap();
            assert_eq!(theme.vars["WALLPAPER"], "none");
            assert!(theme.vars.contains_key("BASE00"));
        }
    }

    #[test]
    fn missing_wallpaper_retains_published_colors_not_new_overrides() {
        let mut theme = Theme {
            name: "fallback".into(),
            path: "fallback".into(),
            vars: HashMap::from([
                ("VARIANT".into(), "dark".into()),
                ("BASE00".into(), "#ffffff".into()),
            ]),
        };
        retain_current_palette(&mut theme,
            "export THEME_BASE00='#123456'\nexport THEME_BASE0D='#abcdef'\nexport THEME_BORDER_COLOR='#334455'\n").unwrap();
        assert_eq!(theme.vars["BASE00"], "#123456");
        assert_eq!(theme.vars["BASE0D"], "#abcdef");
        assert_eq!(theme.vars["BORDER_COLOR"], "#334455");
        assert!(theme.vars.contains_key("WHITE"));
    }

    #[test]
    fn published_palette_ignores_non_colors_and_invalid_values() {
        let colors = parse_published_colors("export THEME_NAME='carbon'\nexport THEME_BASE00='bad'\nexport THEME_BASE01='#123456'\n");
        assert_eq!(colors.len(), 1);
        assert_eq!(colors["BASE01"], "#123456");
    }

    fn matugen_json(primary: &str, primary_light: &str) -> String {
        let role = |name: &str, dark: &str, light: &str| {
            format!(
                r#""{name}":{{"dark":{{"color":"{dark}"}},"default":{{"color":"{dark}"}},"light":{{"color":"{light}"}}}}"#
            )
        };
        let roles = [
            role("background", "#101417", "#f9f9f9"),
            role("surface_container_lowest", "#0a0f12", "#ffffff"),
            role("surface_container_low", "#181c20", "#f1f4f9"),
            role("surface_container", "#1c2024", "#ebebf3"),
            role("on_surface_variant", "#c1c7ce", "#41474d"),
            role("on_surface", "#dfe3e8", "#181c20"),
            role("primary", primary, primary_light),
            role("secondary", "#b7c9d9", "#4f606e"),
            role("tertiary", "#cec0e8", "#64597c"),
            role("error", "#ffb4ab", "#ba1a1a"),
        ];
        format!(r#"{{"colors":{{{}}}}}"#, roles.join(","))
    }

    #[test]
    fn matugen_scheme_names_accept_both_spellings() {
        assert_eq!(
            normalize_matugen_scheme("tonal-spot").unwrap(),
            "scheme-tonal-spot"
        );
        assert_eq!(
            normalize_matugen_scheme("scheme-rainbow").unwrap(),
            "scheme-rainbow"
        );
        assert!(normalize_matugen_scheme("not-a-scheme").is_err());
    }

    #[test]
    fn builds_a_complete_palette_from_matugen_roles() {
        let json = matugen_json("#95cdf7", "#256489");
        let dark = palette_from_matugen_json(&json, "dark", &[], NEUTRAL_BRIGHTNESS).unwrap();
        assert_eq!(dark["BASE00"], "#101417");
        assert_eq!(dark["BASE04"], "#c1c7ce");
        assert!(dark.contains_key("LIGHT_GREY"));

        let light = palette_from_matugen_json(&json, "light", &[], NEUTRAL_BRIGHTNESS).unwrap();
        assert_eq!(light["BASE00"], "#f9f9f9");
        for key in [
            "GREY_FG",
            "GREY_FG2",
            "LINE",
            "VIBRANT_GREEN",
            "NORD_BLUE",
            "SUN",
            "DARK_PURPLE",
            "ORANGE",
            "STATUSLINE_BG",
            "LIGHTBG",
            "PMENU_BG",
            "FOLDER_BG",
        ] {
            assert!(dark.contains_key(key), "missing {key}");
            assert!(light.contains_key(key), "light is missing {key}");
        }
    }

    #[test]
    fn different_matugen_schemes_produce_different_accents() {
        let cool =
            palette_from_matugen_json(&matugen_json("#95cdf7", "#256489"), "dark", &[], NEUTRAL_BRIGHTNESS).unwrap();
        let expressive =
            palette_from_matugen_json(&matugen_json("#becf76", "#4a5d00"), "dark", &[], NEUTRAL_BRIGHTNESS).unwrap();
        assert_ne!(cool["BASE0D"], expressive["BASE0D"]);
        assert_ne!(cool["BASE0B"], expressive["BASE0B"]);
        assert_eq!(cool["BASE00"], expressive["BASE00"]);
    }

    const ACCENT_KEYS: [&str; 9] = [
        "BASE08", "BASE09", "BASE0A", "BASE0B", "BASE0C", "BASE0D", "BASE0E", "BASE0F", "TEAL",
    ];

    fn accent_hues(pixels: &[Rgb], variant: &str) -> Vec<f32> {
        let json = matugen_json("#95cdf7", "#256489");
        let palette =
            palette_from_matugen_json(
            &json,
            variant,
            &wallpaper_hues(pixels),
            NEUTRAL_BRIGHTNESS,
        )
        .unwrap();
        ACCENT_KEYS
            .iter()
            .map(|key| {
                let hex = &palette[*key];
                let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
                rgb_to_hsl(Rgb(channel(1), channel(3), channel(5))).0
            })
            .collect()
    }

    #[test]
    fn single_hue_wallpaper_keeps_every_accent_near_that_hue() {
        let pixels = vec![Rgb(30, 60, 180); 100];
        let wall_hue = rgb_to_hsl(pixels[0]).0;
        for variant in ["dark", "light"] {
            for hue in accent_hues(&pixels, variant) {
                assert!(hue_distance(hue, wall_hue) <= 52.0, "{variant}: hue {hue}");
            }
        }
    }

    #[test]
    fn multi_hue_wallpaper_only_uses_its_own_hues() {
        let mut pixels = vec![Rgb(200, 40, 40); 50];
        pixels.extend(vec![Rgb(40, 200, 60); 50]);
        let wall_hues: Vec<f32> = pixels.iter().map(|p| rgb_to_hsl(*p).0).collect();
        assert_eq!(wallpaper_hues(&pixels).len(), 2);
        for hue in accent_hues(&pixels, "dark") {
            assert!(
                wall_hues.iter().any(|w| hue_distance(hue, *w) <= 52.0),
                "hue {hue} is not near a wallpaper hue"
            );
        }
    }

    #[test]
    fn muted_wallpaper_still_uses_its_own_tint() {
        // Saturation ~0.08: below the strict floor, but clearly warm.
        let pixels = vec![Rgb(130, 120, 110); 100];
        let wall_hue = rgb_to_hsl(pixels[0]).0;
        assert_eq!(wallpaper_hues(&pixels).len(), 1);
        for hue in accent_hues(&pixels, "dark") {
            assert!(hue_distance(hue, wall_hue) <= 52.0, "hue {hue}");
        }
    }

    #[test]
    fn grayscale_wallpaper_falls_back_to_the_primary_hue() {
        let pixels = vec![Rgb(120, 120, 120); 100];
        assert!(wallpaper_hues(&pixels).is_empty());
        let primary_hue = rgb_to_hsl(Rgb(0x95, 0xcd, 0xf7)).0;
        for hue in accent_hues(&pixels, "dark") {
            assert!(hue_distance(hue, primary_hue) <= 52.0, "hue {hue}");
        }
    }

    fn lightness_of(palette: &std::collections::HashMap<String, String>, key: &str) -> f32 {
        let hex = &palette[key];
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
        rgb_to_hsl(Rgb(channel(1), channel(3), channel(5))).2
    }

    fn palette_at(variant: &str, brightness: f32) -> std::collections::HashMap<String, String> {
        let json = matugen_json("#95cdf7", "#256489");
        palette_from_matugen_json(&json, variant, &[], brightness).unwrap()
    }

    #[test]
    fn brightness_is_measured_from_pixels() {
        assert!(wallpaper_brightness(&[Rgb(0, 0, 0); 10]) < 0.01);
        assert!(wallpaper_brightness(&[Rgb(255, 255, 255); 10]) > 0.99);
        assert_eq!(wallpaper_brightness(&[]), NEUTRAL_BRIGHTNESS);
    }

    #[test]
    fn dark_wallpaper_dims_foregrounds_in_dark_variant() {
        let dark_wall = palette_at("dark", 0.05);
        let mid_wall = palette_at("dark", 0.5);
        let bright_wall = palette_at("dark", 0.9);
        for key in ["GREY_FG", "BASE05", "BASE0D"] {
            assert!(lightness_of(&dark_wall, key) < lightness_of(&mid_wall, key), "{key}");
        }
        // Mid-gray and brighter wallpapers keep matugen's tones.
        assert_eq!(mid_wall, bright_wall);
        // Dimmed, but still clearly readable against the background.
        for key in ["GREY_FG", "BASE05", "BASE0D"] {
            let gap = lightness_of(&dark_wall, key) - lightness_of(&dark_wall, "BASE00");
            assert!(gap >= 0.3, "{key} gap {gap}");
        }
    }

    #[test]
    fn bright_wallpaper_softens_foregrounds_in_light_variant() {
        let bright_wall = palette_at("light", 0.95);
        let mid_wall = palette_at("light", 0.5);
        for key in ["GREY_FG", "BASE05", "BASE0D"] {
            assert!(lightness_of(&bright_wall, key) > lightness_of(&mid_wall, key), "{key}");
            let gap = lightness_of(&bright_wall, "BASE00") - lightness_of(&bright_wall, key);
            assert!(gap >= 0.3, "{key} gap {gap}");
        }
        assert_eq!(mid_wall, palette_at("light", 0.1));
    }

    #[test]
    fn softening_keeps_foregrounds_readable_on_the_darkest_wallpaper() {
        let palette = palette_at("dark", 0.0);
        let rgb = |key: &str| {
            let hex = &palette[key];
            let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
            Rgb(channel(1), channel(3), channel(5))
        };
        let background = rgb("BASE00");
        assert!(contrast_ratio(rgb("BASE05"), background) >= MIN_TEXT_CONTRAST);
        for key in ["GREY_FG", "BASE08", "BASE0B", "BASE0D"] {
            assert!(contrast_ratio(rgb(key), background) >= MIN_ACCENT_CONTRAST, "{key}");
        }
        // And clearly dimmer than on a normal wallpaper.
        let normal = palette_at("dark", 0.5);
        assert!(lightness_of(&palette, "BASE07") + 0.15 < lightness_of(&normal, "BASE07"));
    }
}
