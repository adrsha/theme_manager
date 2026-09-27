use crate::{env_sh_file, fish_conf_file, Theme, ThemeError};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SIZE: usize = 96;

pub fn resolve_dynamic_palette(theme: &mut Theme) -> Result<(), ThemeError> {
    if theme.vars.get("MODE").map(String::as_str) != Some("dynamic") {
        return Ok(());
    }

    let wallpaper = resolve_wallpaper(theme)?;
    theme.vars.insert(
        "WALLPAPER".to_string(),
        wallpaper.to_string_lossy().into_owned(),
    );
    let pixels = decode_representative_frame(&wallpaper)?;
    let variant = theme.require("VARIANT")?;
    let generated = generate_palette(&pixels, variant)?;

    // Explicit colors remain valid overrides. Dynamic mode only fills keys
    // omitted from the theme file.
    for (key, value) in generated {
        theme.vars.entry(key).or_insert(value);
    }

    Ok(())
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

fn current_wallpaper() -> Option<PathBuf> {
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
    ] {
        palette.insert(key.to_string(), color.to_hex());
    }
    Ok(palette)
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
    use super::{generate_palette, parse_fish_wallpaper, parse_sh_wallpaper, resolve_dynamic_palette, rgb_to_hsl, Rgb};
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
        assert_eq!(theme.vars.len(), 36);
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
    fn dynamic_mode_requires_a_wallpaper() {
        let mut theme = Theme {
            name: "missing-wallpaper".to_string(),
            path: "missing-wallpaper".into(),
            vars: HashMap::from([
                ("VARIANT".to_string(), "dark".to_string()),
                ("MODE".to_string(), "dynamic".to_string()),
            ]),
        };
        assert!(resolve_dynamic_palette(&mut theme).is_err());
    }
}
