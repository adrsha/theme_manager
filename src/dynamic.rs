use crate::{env_sh_file, fish_conf_file, Theme, ThemeError};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SIZE: usize = 96;

// --- Hue detection ---------------------------------------------------------
// Prefer clearly colored pixels; a muted wallpaper retries with a lower floor
// so its faint tint is still used.
const MIN_ACCENT_SATURATION: f32 = 0.18;
const RELAXED_ACCENT_SATURATION: f32 = 0.05;
// Detected hues are at least this many degrees apart.
const MIN_HUE_GAP: f32 = 25.0;
const MAX_WALLPAPER_HUES: usize = 8;
// A hue only counts when at least this share of the wallpaper's pixels carries
// it, so a few stray pixels never become an accent.
const MIN_HUE_SHARE: f32 = 0.01;
// Pixels within this many degrees of a hue peak belong to that hue.
const HUE_MERGE_RADIUS: f32 = 15.0;
// Hue peaks are searched at this many evenly spaced angles.
const HUE_PEAK_STEPS: usize = 36;
// Hue is noise in pure-black and near-white pixels, so they never vote.
const MIN_HUE_LIGHTNESS: f32 = 0.03;
const MAX_HUE_LIGHTNESS: f32 = 0.95;

// --- Accents ---------------------------------------------------------------
// Semantic slot order, with the hue each slot stands for. A wallpaper hue is
// placed in the slot whose name it is closest to (crimson -> red, ...).
const SLOT_HUES: [f32; 9] = [5.0, 30.0, 52.0, 125.0, 190.0, 220.0, 280.0, 330.0, 170.0];
// A slot no wallpaper hue is close to takes the nearest found hue and moves it
// this share of the way toward the slot's own hue (at most 0.28 * 180 = 50
// degrees), so every accent stays near a hue the wallpaper really has and
// neighboring slots still differ. Slots derived from the same hue also step
// their lightness by LIGHTNESS_STEP.
const SLOT_PULL: f32 = 0.28;
const LIGHTNESS_STEP: f32 = 0.05;
// Accents keep the wallpaper's own saturation and lightness, within these
// limits, and are then lifted only as far as needed for MIN_ACCENT_CONTRAST.
const ACCENT_SATURATION: (f32, f32) = (0.25, 0.85);
const DARK_ACCENT_LIGHTNESS: (f32, f32) = (0.40, 0.80);
const LIGHT_ACCENT_LIGHTNESS: (f32, f32) = (0.30, 0.55);
// Used when the wallpaper has no usable hue (grayscale).
const FALLBACK_HUE: Hsl = Hsl(225.0, 0.45, 0.60);

// --- Neutrals and text -----------------------------------------------------
const MAX_BACKGROUND_SATURATION: f32 = 0.22;
const MAX_FOREGROUND_SATURATION: f32 = 0.16;
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
    if theme.vars.contains_key("MATUGEN_SCHEME") {
        eprintln!("theme-manager: MATUGEN_SCHEME is no longer used and is ignored");
    }
    let pixels = decode_representative_frame(&wallpaper)?;
    let generated = generate_palette(&pixels, variant)?;

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

/// Build the full palette from the wallpaper's pixels.
///
/// Everything comes from the wallpaper itself: backgrounds carry its overall
/// tint, accents are its own hues at their own saturation and brightness (a
/// muted wallpaper gives muted accents, a vivid one vivid accents), and only
/// contrast limits move a color away from what the wallpaper shows.
fn generate_palette(pixels: &[Rgb], variant: &str) -> Result<HashMap<String, String>, ThemeError> {
    let dark = match variant {
        "dark" => true,
        "light" => false,
        value => return Err(ThemeError::InvalidVariant(value.to_string())),
    };

    let tint = wall_tint(pixels);
    let background_at = |lightness| {
        hsl_to_rgb(Hsl(tint.0, tint.1.min(MAX_BACKGROUND_SATURATION), lightness))
    };
    let foreground_at = |lightness| {
        hsl_to_rgb(Hsl(tint.0, tint.1.min(MAX_FOREGROUND_SATURATION), lightness))
    };

    // BASE00 is the main background; BASE01 is the darkest (lightest in the
    // light variant) surface, BASE02/BASE03 step toward the foreground.
    let (base00, base01, base02, base03) = if dark {
        (
            background_at(0.08),
            background_at(0.05),
            background_at(0.11),
            background_at(0.15),
        )
    } else {
        (
            background_at(0.97),
            background_at(0.99),
            background_at(0.94),
            background_at(0.90),
        )
    };

    // Text on a wallpaper that is very dark (dark variant) or very bright
    // (light variant) is pulled toward the background, never below its contrast.
    let amount = softness(wallpaper_brightness(pixels), dark) * MAX_SOFTEN;
    let soften =
        |color: Rgb, min_contrast: f32| soften_toward(color, base00, amount, min_contrast);
    let (base04, base05) = if dark {
        (
            soften(foreground_at(0.62), MIN_ACCENT_CONTRAST),
            soften(foreground_at(0.86), MIN_TEXT_CONTRAST),
        )
    } else {
        (
            soften(foreground_at(0.38), MIN_ACCENT_CONTRAST),
            soften(foreground_at(0.16), MIN_TEXT_CONTRAST),
        )
    };
    let text_tint = if dark {
        Rgb(255, 255, 255)
    } else {
        Rgb(0, 0, 0)
    };
    let base06 = mix(base05, text_tint, 0.10);
    let base07 = mix(base05, text_tint, 0.20);

    let accents = assign_accent_slots(&wallpaper_hues(pixels))
        .map(|(wall, tweak)| accent_color(wall, tweak, base00, dark));
    let [red, orange, yellow, green, cyan, blue, purple, pink, teal] = accents;
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

/// The wallpaper's overall tint: the hue most of its colored pixels share, with
/// their average saturation. Gray when the wallpaper has no color at all.
fn wall_tint(pixels: &[Rgb]) -> Hsl {
    let colored: Vec<Hsl> = pixels
        .iter()
        .map(|&pixel| rgb_to_hsl(pixel))
        .filter(|c| c.1 >= RELAXED_ACCENT_SATURATION)
        .collect();
    let Some(peak) = heaviest_hue(&colored, |_| 1.0) else {
        return Hsl(0.0, 0.0, 0.5);
    };
    let members: Vec<Hsl> = colored
        .into_iter()
        .filter(|c| hue_distance(c.0, peak) <= HUE_MERGE_RADIUS)
        .collect();
    mean_hsl(&members, |_| 1.0)
}

/// Place each wallpaper hue in the slot it is closest to, closest pairs first,
/// so every hue appears under its natural name. Slots left empty are derived
/// from the nearest placed hue (see `toward_slot`). Each entry is the slot's
/// wallpaper color plus a lightness tweak (0 for hues found in the wallpaper).
fn assign_accent_slots(hues: &[Hsl]) -> [(Hsl, f32); SLOT_HUES.len()] {
    let fallback = [FALLBACK_HUE];
    let hues = if hues.is_empty() { &fallback[..] } else { hues };

    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for (hue_index, hue) in hues.iter().enumerate() {
        for (slot, slot_hue) in SLOT_HUES.iter().enumerate() {
            pairs.push((hue_distance(hue.0, *slot_hue), hue_index, slot));
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut placed: [Option<Hsl>; SLOT_HUES.len()] = [None; SLOT_HUES.len()];
    let mut hue_used = vec![false; hues.len()];
    for (_, hue_index, slot) in pairs {
        if !hue_used[hue_index] && placed[slot].is_none() {
            hue_used[hue_index] = true;
            placed[slot] = Some(hues[hue_index]);
        }
    }

    let mut repeats = [0usize; SLOT_HUES.len()];
    let mut result = [(FALLBACK_HUE, 0.0); SLOT_HUES.len()];
    for slot in 0..SLOT_HUES.len() {
        if let Some(hue) = placed[slot] {
            result[slot] = (hue, 0.0);
            continue;
        }
        // At least one slot is placed: `hues` is never empty and there are
        // more slots than hues. The fallback only guards that invariant.
        let nearest = (0..SLOT_HUES.len())
            .filter(|&other| placed[other].is_some())
            .min_by(|&a, &b| {
                hue_distance(SLOT_HUES[slot], SLOT_HUES[a])
                    .total_cmp(&hue_distance(SLOT_HUES[slot], SLOT_HUES[b]))
            });
        if let Some((other, source)) = nearest.and_then(|o| placed[o].map(|hue| (o, hue))) {
            repeats[other] += 1;
            result[slot] = toward_slot(source, SLOT_HUES[slot], repeats[other]);
        }
    }
    result
}

/// `source` with its hue moved toward `slot_hue` (see `SLOT_PULL`), plus a
/// lightness tweak of +1, -1, +2, -2, ... times `LIGHTNESS_STEP` for repeat
/// `round` (1, 2, 3, ...), so slots derived from the same hue stay distinct.
fn toward_slot(source: Hsl, slot_hue: f32, round: usize) -> (Hsl, f32) {
    let delta = (slot_hue - source.0 + 540.0).rem_euclid(360.0) - 180.0;
    let magnitude = ((round + 1) / 2) as f32;
    let step = if round % 2 == 1 { magnitude } else { -magnitude };
    (
        Hsl((source.0 + delta * SLOT_PULL).rem_euclid(360.0), source.1, source.2),
        step * LIGHTNESS_STEP,
    )
}

/// The wallpaper color as an accent: its hue, its brightness within sane limits
/// (plus `tweak`), and a saturation midway between the color's relative
/// saturation and its absolute chroma. HSL saturation alone overstates very
/// dark and very light colors (a near-black teal reads as 0.2, a pastel peach
/// as 1.0); the midpoint keeps vivid colors vivid without turning those into
/// loud ones. The result is lifted (lighter on dark, darker on light) only as
/// far as needed to stay readable on `background`.
fn accent_color(wall: Hsl, tweak: f32, background: Rgb, dark: bool) -> Rgb {
    let (min_lightness, max_lightness) = if dark {
        DARK_ACCENT_LIGHTNESS
    } else {
        LIGHT_ACCENT_LIGHTNESS
    };
    let wall_chroma = chroma(&wall);
    let lightness = (wall.2.clamp(min_lightness, max_lightness) + tweak).clamp(0.0, 1.0);
    let room = 1.0 - (2.0 * lightness - 1.0).abs();
    let kept = if room > f32::EPSILON {
        (wall_chroma / room).clamp(0.0, 1.0)
    } else {
        wall.1
    };
    let saturation = (wall.1 * kept)
        .sqrt()
        .clamp(ACCENT_SATURATION.0, ACCENT_SATURATION.1);
    let color = hsl_to_rgb(Hsl(wall.0, saturation, lightness));
    lift_to_contrast(color, background, dark, MIN_ACCENT_CONTRAST)
}

/// Distinct hues of the wallpaper, most dominant first. Each entry is the
/// hue's representative color: colorful pixels count more than dull ones, so a
/// small vivid area is not averaged away by a large dull area of the same hue.
///
/// Hues are counted straight from the pixels, not from the most common exact
/// colors: shading splits one large hue into many near-identical colors, which
/// would crowd out every smaller hue. Prefers saturated pixels; a muted
/// wallpaper retries with a lower saturation floor. Empty only for
/// (near-)grayscale wallpapers.
fn wallpaper_hues(pixels: &[Rgb]) -> Vec<Hsl> {
    if pixels.is_empty() {
        return Vec::new();
    }
    let all: Vec<Hsl> = pixels.iter().map(|&pixel| rgb_to_hsl(pixel)).collect();
    for min_saturation in [MIN_ACCENT_SATURATION, RELAXED_ACCENT_SATURATION] {
        let voters: Vec<Hsl> = all
            .iter()
            .copied()
            .filter(|c| {
                c.1 >= min_saturation && (MIN_HUE_LIGHTNESS..=MAX_HUE_LIGHTNESS).contains(&c.2)
            })
            .collect();
        let hues = pick_hues(voters, pixels.len());
        if !hues.is_empty() {
            return hues;
        }
    }
    Vec::new()
}

/// Repeatedly take the heaviest hue, then drop everything within
/// `MIN_HUE_GAP` of it so the next hue is clearly different.
fn pick_hues(mut remaining: Vec<Hsl>, total_pixels: usize) -> Vec<Hsl> {
    let mut hues = Vec::new();
    while hues.len() < MAX_WALLPAPER_HUES {
        let Some(peak) = heaviest_hue(&remaining, chroma) else {
            break;
        };
        let members: Vec<Hsl> = remaining
            .iter()
            .copied()
            .filter(|c| hue_distance(c.0, peak) <= HUE_MERGE_RADIUS)
            .collect();
        if members.is_empty() || (members.len() as f32) < MIN_HUE_SHARE * total_pixels as f32 {
            break;
        }
        hues.push(mean_hsl(&members, |c| chroma(c) * chroma(c)));
        remaining.retain(|c| hue_distance(c.0, peak) > MIN_HUE_GAP);
    }
    hues
}

/// How colorful `color` really is (0..=1): HSL saturation scaled by how much
/// room its lightness leaves. Unlike saturation alone it does not rate a
/// near-black or near-white color as vivid.
fn chroma(color: &Hsl) -> f32 {
    color.1 * (1.0 - (2.0 * color.2 - 1.0).abs())
}

/// Hue angle (degrees) with the most `weight` of pixels around it.
fn heaviest_hue(colors: &[Hsl], weight: impl Fn(&Hsl) -> f32) -> Option<f32> {
    (0..HUE_PEAK_STEPS)
        .map(|step| step as f32 * 360.0 / HUE_PEAK_STEPS as f32)
        .map(|angle| {
            let total: f32 = colors
                .iter()
                .filter(|c| hue_distance(c.0, angle) <= HUE_MERGE_RADIUS)
                .map(&weight)
                .sum();
            (angle, total)
        })
        .filter(|&(_, total)| total > 0.0)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(angle, _)| angle)
}

/// Weighted average color of `colors`; hue is averaged as an angle so 350° and
/// 10° give 0°, not 180°. Returns mid-gray when there is nothing to average
/// (empty slice or all weights zero).
fn mean_hsl(colors: &[Hsl], weight: impl Fn(&Hsl) -> f32) -> Hsl {
    let total: f32 = colors.iter().map(&weight).sum();
    if colors.is_empty() || total <= 0.0 {
        return Hsl(0.0, 0.0, 0.5);
    }
    let (mut sin, mut cos, mut saturation, mut lightness) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for color in colors {
        let w = weight(color);
        let radians = color.0.to_radians();
        sin += w * radians.sin();
        cos += w * radians.cos();
        saturation += w * color.1;
        lightness += w * color.2;
    }
    Hsl(
        sin.atan2(cos).to_degrees().rem_euclid(360.0),
        saturation / total,
        lightness / total,
    )
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
/// side gives 0, so ordinary wallpapers keep the normal text tones.
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

/// Move `color` away from `background` (lighter on dark, darker on light) in
/// small lightness steps until it reaches `min_contrast`. Unchanged when it
/// already does; if even the largest shift is not enough, returns that largest
/// shift (the closest it can get).
fn lift_to_contrast(color: Rgb, background: Rgb, dark: bool, min_contrast: f32) -> Rgb {
    const STEP: f32 = 0.02;
    const MAX_STEPS: usize = 25;
    let hsl = rgb_to_hsl(color);
    let direction = if dark { 1.0 } else { -1.0 };
    let mut best = color;
    for i in 0..=MAX_STEPS {
        let lightness = (hsl.2 + direction * STEP * i as f32).clamp(0.0, 1.0);
        best = if i == 0 { color } else { hsl_to_rgb(Hsl(hsl.0, hsl.1, lightness)) };
        if contrast_ratio(best, background) >= min_contrast {
            break;
        }
    }
    best
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
        accent_color, contrast_ratio, generate_palette, hsl_to_rgb, hue_distance, mean_hsl,
        parse_fish_wallpaper, parse_published_colors, parse_sh_wallpaper, resolve_dynamic_palette,
        retain_current_palette, rgb_to_hsl, wallpaper_brightness, wallpaper_hues, Hsl, Rgb,
        FALLBACK_HUE, MIN_ACCENT_CONTRAST, MIN_TEXT_CONTRAST, NEUTRAL_BRIGHTNESS,
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

    const ACCENT_KEYS: [&str; 9] = [
        "BASE08", "BASE09", "BASE0A", "BASE0B", "BASE0C", "BASE0D", "BASE0E", "BASE0F", "TEAL",
    ];

    fn hsl_pixels(hue: f32, saturation: f32, lightness: f32, count: usize) -> Vec<Rgb> {
        vec![hsl_to_rgb(Hsl(hue, saturation, lightness)); count]
    }

    fn color(palette: &HashMap<String, String>, key: &str) -> Rgb {
        let hex = &palette[key];
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
        Rgb(channel(1), channel(3), channel(5))
    }

    fn hsl_of(palette: &HashMap<String, String>, key: &str) -> Hsl {
        rgb_to_hsl(color(palette, key))
    }

    fn palette_for(pixels: &[Rgb], variant: &str) -> HashMap<String, String> {
        generate_palette(pixels, variant).unwrap()
    }

    fn gray_wall(level: u8) -> Vec<Rgb> {
        vec![Rgb(level, level, level); 100]
    }

    #[test]
    fn unknown_variant_is_rejected() {
        assert!(generate_palette(&gray_wall(100), "sepia").is_err());
    }

    #[test]
    fn empty_wallpaper_still_produces_a_palette() {
        let palette = palette_for(&[], "dark");
        assert_eq!(palette.len(), palette_for(&gray_wall(100), "dark").len());
    }

    // --- hue detection ---

    #[test]
    fn small_hue_is_not_crowded_out_by_a_heavily_shaded_hue() {
        // A heavily shaded blue area (48 distinct colors) plus a ~6% red patch
        // that is itself spread over a few shades.
        let mut pixels: Vec<Rgb> = (0..6)
            .flat_map(|s| (0..8).map(move |l| (s, l)))
            .flat_map(|(s, l)| {
                hsl_pixels(230.0, 0.3 + s as f32 * 0.1, 0.15 + l as f32 * 0.065, 120)
            })
            .collect();
        for shade in 0..12 {
            pixels.extend(hsl_pixels(350.0, 0.6, 0.25 + shade as f32 * 0.018, 30));
        }
        let hues = wallpaper_hues(&pixels);
        assert!(hues.iter().any(|h| hue_distance(h.0, 350.0) <= 15.0), "{hues:?}");
        assert!(hues.iter().any(|h| hue_distance(h.0, 230.0) <= 15.0), "{hues:?}");
    }

    #[test]
    fn stray_pixels_do_not_become_a_hue() {
        let mut pixels = hsl_pixels(230.0, 0.5, 0.3, 1000);
        pixels.extend(hsl_pixels(350.0, 0.8, 0.5, 3));
        assert_eq!(wallpaper_hues(&pixels).len(), 1);
    }

    #[test]
    fn a_small_vivid_patch_is_not_averaged_away_by_a_dull_area_of_its_hue() {
        let mut pixels = hsl_pixels(340.0, 0.15, 0.25, 900);
        pixels.extend(hsl_pixels(335.0, 0.8, 0.5, 100));
        let hues = wallpaper_hues(&pixels);
        assert_eq!(hues.len(), 1);
        assert!(hues[0].1 > 0.5, "{hues:?}");
    }

    #[test]
    fn pure_black_and_near_white_pixels_do_not_vote() {
        let mut pixels = hsl_pixels(10.0, 0.9, 0.01, 500);
        pixels.extend(hsl_pixels(100.0, 0.9, 0.99, 500));
        assert!(wallpaper_hues(&pixels).is_empty());
    }

    #[test]
    fn a_nearly_black_wallpaper_keeps_its_faint_tint() {
        let hues = wallpaper_hues(&vec![Rgb(13, 19, 18); 200]);
        assert_eq!(hues.len(), 1);
        assert!(hue_distance(hues[0].0, 165.0) <= 20.0, "{hues:?}");
    }

    #[test]
    fn hue_average_wraps_around_red() {
        let mean = mean_hsl(&[Hsl(350.0, 0.5, 0.4), Hsl(10.0, 0.5, 0.4)], |_| 1.0);
        assert!(hue_distance(mean.0, 0.0) < 1.0, "{mean:?}");
        let empty = mean_hsl(&[], |_| 1.0);
        assert_eq!((empty.1, empty.2), (0.0, 0.5));
        // All-zero weights must not divide by zero.
        let zero = mean_hsl(&[Hsl(10.0, 0.5, 0.4)], |_| 0.0);
        assert_eq!((zero.1, zero.2), (0.0, 0.5));
    }

    // --- accents ---

    fn accent_hues(pixels: &[Rgb], variant: &str) -> Vec<f32> {
        let palette = palette_for(pixels, variant);
        ACCENT_KEYS.iter().map(|key| hsl_of(&palette, key).0).collect()
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
    fn each_wallpaper_hue_lands_in_the_slot_of_its_name() {
        let mut pixels = hsl_pixels(350.0, 0.7, 0.45, 100);
        pixels.extend(hsl_pixels(110.0, 0.6, 0.45, 100));
        pixels.extend(hsl_pixels(215.0, 0.7, 0.45, 100));
        pixels.extend(hsl_pixels(40.0, 0.8, 0.55, 100));
        let palette = palette_for(&pixels, "dark");
        for (key, expected) in [("BASE08", 350.0), ("BASE0B", 110.0), ("BASE0D", 215.0), ("BASE09", 40.0)] {
            let hue = hsl_of(&palette, key).0;
            assert!(hue_distance(hue, expected) <= 12.0, "{key}: {hue} vs {expected}");
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
    fn grayscale_wallpaper_falls_back_to_the_default_hue() {
        let pixels = gray_wall(120);
        assert!(wallpaper_hues(&pixels).is_empty());
        for hue in accent_hues(&pixels, "dark") {
            assert!(hue_distance(hue, FALLBACK_HUE.0) <= 52.0, "hue {hue}");
        }
    }

    #[test]
    fn accents_keep_the_wallpapers_saturation() {
        let muted = palette_for(&hsl_pixels(220.0, 0.3, 0.5, 100), "dark");
        let vivid = palette_for(&hsl_pixels(220.0, 0.9, 0.5, 100), "dark");
        let (muted, vivid) = (hsl_of(&muted, "BASE0D").1, hsl_of(&vivid, "BASE0D").1);
        assert!(vivid > muted + 0.3, "muted {muted}, vivid {vivid}");
    }

    #[test]
    fn a_single_hue_wallpaper_still_gives_nine_distinct_accents() {
        for variant in ["dark", "light"] {
            let palette = palette_for(&hsl_pixels(162.0, 0.4, 0.3, 200), variant);
            let mut seen: Vec<String> = ACCENT_KEYS.iter().map(|k| palette[*k].clone()).collect();
            seen.sort();
            seen.dedup();
            assert_eq!(seen.len(), ACCENT_KEYS.len(), "{variant}: {seen:?}");
        }
    }

    #[test]
    fn a_pastel_wallpaper_color_does_not_become_vivid_in_the_light_variant() {
        let palette = palette_for(&hsl_pixels(23.0, 1.0, 0.85, 200), "light");
        let saturation = hsl_of(&palette, "BASE09").1;
        assert!(saturation < 0.7, "{saturation}");
    }

    #[test]
    fn a_nearly_black_wallpaper_gets_muted_accents() {
        let palette = palette_for(&vec![Rgb(13, 19, 18); 200], "dark");
        for key in ACCENT_KEYS {
            assert!(hsl_of(&palette, key).1 < 0.3, "{key}: {:?}", hsl_of(&palette, key));
        }
    }

    #[test]
    fn a_bright_color_gets_a_brighter_accent_than_a_dull_one_in_the_dark_variant() {
        let background = hsl_to_rgb(Hsl(220.0, 0.1, 0.08));
        let lightness = |wall: Hsl| rgb_to_hsl(accent_color(wall, 0.0, background, true)).2;
        let bright = lightness(Hsl(60.0, 0.6, 0.7));
        let dull = lightness(Hsl(60.0, 0.6, 0.3));
        assert!(bright > dull + 0.1, "bright {bright}, dull {dull}");
    }

    #[test]
    fn every_accent_stays_readable_on_any_wallpaper_color() {
        for variant in ["dark", "light"] {
            for hue in [0.0, 60.0, 120.0, 240.0, 270.0, 330.0] {
                for lightness in [0.10, 0.45, 0.80] {
                    let palette = palette_for(&hsl_pixels(hue, 0.7, lightness, 200), variant);
                    let background = color(&palette, "BASE00");
                    for key in ACCENT_KEYS {
                        let ratio = contrast_ratio(color(&palette, key), background);
                        assert!(
                            ratio >= MIN_ACCENT_CONTRAST,
                            "{variant} hue {hue} l {lightness} {key}: {ratio:.2}"
                        );
                    }
                }
            }
        }
    }

    // --- neutrals and text ---

    #[test]
    fn background_carries_the_wallpapers_dominant_tint() {
        let mut pixels = hsl_pixels(30.0, 0.5, 0.2, 300);
        pixels.extend(hsl_pixels(220.0, 0.8, 0.5, 40));
        for variant in ["dark", "light"] {
            let palette = palette_for(&pixels, variant);
            let background = hsl_of(&palette, "BASE00");
            // 8-bit rounding of a very light, barely tinted color shifts its hue a little.
            assert!(hue_distance(background.0, 30.0) <= 15.0, "{variant}: {background:?}");
            assert!(background.1 <= 0.23, "{variant}: {background:?}");
        }
    }

    #[test]
    fn brightness_is_measured_from_pixels() {
        assert!(wallpaper_brightness(&[Rgb(0, 0, 0); 10]) < 0.01);
        assert!(wallpaper_brightness(&[Rgb(255, 255, 255); 10]) > 0.99);
        assert_eq!(wallpaper_brightness(&[]), NEUTRAL_BRIGHTNESS);
    }

    #[test]
    fn dark_wallpaper_dims_text_in_dark_variant() {
        let dark_wall = palette_for(&gray_wall(5), "dark");
        let mid_wall = palette_for(&gray_wall(128), "dark");
        for key in ["LIGHT_GREY", "BASE05"] {
            assert!(hsl_of(&dark_wall, key).2 < hsl_of(&mid_wall, key).2, "{key}");
            let gap = hsl_of(&dark_wall, key).2 - hsl_of(&dark_wall, "BASE00").2;
            assert!(gap >= 0.3, "{key} gap {gap}");
        }
        // Mid-gray and brighter wallpapers keep the normal tones.
        assert_eq!(mid_wall, palette_for(&gray_wall(230), "dark"));
    }

    #[test]
    fn bright_wallpaper_softens_text_in_light_variant() {
        let bright_wall = palette_for(&gray_wall(245), "light");
        let mid_wall = palette_for(&gray_wall(120), "light");
        for key in ["LIGHT_GREY", "BASE05"] {
            assert!(hsl_of(&bright_wall, key).2 > hsl_of(&mid_wall, key).2, "{key}");
            let gap = hsl_of(&bright_wall, "BASE00").2 - hsl_of(&bright_wall, key).2;
            assert!(gap >= 0.3, "{key} gap {gap}");
        }
        assert_eq!(mid_wall, palette_for(&gray_wall(25), "light"));
    }

    #[test]
    fn softening_keeps_text_readable_on_the_darkest_wallpaper() {
        let palette = palette_for(&gray_wall(0), "dark");
        let background = color(&palette, "BASE00");
        assert!(contrast_ratio(color(&palette, "BASE05"), background) >= MIN_TEXT_CONTRAST);
        assert!(contrast_ratio(color(&palette, "LIGHT_GREY"), background) >= MIN_ACCENT_CONTRAST);
    }

    #[test]
    fn matugen_scheme_in_a_theme_file_is_ignored() {
        // Old theme files may still carry the key; it must not stop the switch.
        let dir = std::env::temp_dir().join(format!(
            "theme-manager-matugen-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let wallpaper = dir.join("wall.png");
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "color=c=0x3050b0:s=64x64", "-frames:v", "1"])
            .arg(&wallpaper)
            .status();
        if made.map(|s| s.success()).unwrap_or(false) {
            let mut vars = HashMap::new();
            vars.insert("MODE".to_string(), "dynamic".to_string());
            vars.insert("VARIANT".to_string(), "dark".to_string());
            vars.insert("MATUGEN_SCHEME".to_string(), "tonal-spot".to_string());
            vars.insert("WALLPAPER".to_string(), wallpaper.to_string_lossy().into_owned());
            let mut theme = Theme { name: "t".into(), path: PathBuf::new(), vars };
            resolve_dynamic_palette(&mut theme).unwrap();
            assert!(theme.vars.contains_key("BASE08"));
        }
        fs::remove_dir_all(dir).ok();
    }
}
