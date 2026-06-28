//! Shared theme parsing and config generation.
//!
//! # Adding a new config target
//!
//! 1. Create a struct in `writers/` (or inline below for small ones).
//! 2. impl `Writer` for it — just `fn generate(&self, theme: &Theme) -> Result<String, ThemeError>`.
//! 3. Add it to the `all_writers()` list in `writers/mod.rs`.
//! That's it. Output path, optional reload hook, and error reporting are all
//! handled by `apply_theme`.

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

pub fn dots() -> PathBuf {
    home().join("dots")
}

pub fn themes_dir() -> PathBuf {
    dots().join("themes")
}

pub fn state_file() -> PathBuf {
    home().join(".local/state/theme/current")
}

pub fn fish_conf_file() -> PathBuf {
    home().join(".config/fish/conf.d/theme_vars.fish")
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
}

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThemeError::Io(e)             => write!(f, "io error: {e}"),
            ThemeError::MissingKey(k)     => write!(f, "missing required key: {k}"),
            ThemeError::InvalidVariant(v) => write!(f, "VARIANT must be 'dark' or 'light', got: {v}"),
            ThemeError::InvalidHex(h)     => write!(f, "invalid hex color: {h}"),
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
pub trait Writer {
    /// Human-readable name shown in log output (e.g. "ghostty", "hyprland").
    fn name(&self) -> &'static str;

    /// Absolute path to write the generated content to.
    fn output_path(&self) -> PathBuf;

    /// Generate the full file content from the theme.
    fn generate(&self, theme: &Theme) -> Result<String, ThemeError>;

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
    let r = u8::from_str_radix(&h[0..2], 16).map_err(|_| ThemeError::InvalidHex(hex.to_string()))?;
    let g = u8::from_str_radix(&h[2..4], 16).map_err(|_| ThemeError::InvalidHex(hex.to_string()))?;
    let b = u8::from_str_radix(&h[4..6], 16).map_err(|_| ThemeError::InvalidHex(hex.to_string()))?;
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
                vars.insert(key, val.to_string());
            }
        }
    }

    match vars.get("VARIANT").map(String::as_str) {
        Some("dark") | Some("light") => {}
        Some(v) => return Err(ThemeError::InvalidVariant(v.to_string())),
        None     => return Err(ThemeError::MissingKey("VARIANT".to_string())),
    }

    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(Theme { name, path: path.to_path_buf(), vars })
}

// ---------------------------------------------------------------------------
// Apply all writers
// ---------------------------------------------------------------------------

pub fn apply_theme(theme: &Theme) {
    // Fish conf is written first so it's ready before anything else.
    run_writer(&writers::FishConf, theme);

    for writer in writers::all_writers(theme) {
        run_writer(writer.as_ref(), theme);
    }

    writers::reload_nvim(theme);

    // Wallpaper handled last and directly (not via Writer trait) because
    // it needs Theme access inside its reload logic.
    writers::apply_wallpaper(theme);
}

fn run_writer(writer: &dyn Writer, theme: &Theme) {
    let path = writer.output_path();
    match writer.generate(theme) {
        Err(e) => {
            eprintln!("theme-manager: [{}] generate failed: {e}", writer.name());
        }
        Ok(content) => {
            match write_file(&path, &content) {
                Err(e) => eprintln!(
                    "theme-manager: [{}] write to {} failed: {e}",
                    writer.name(),
                    path.display()
                ),
                Ok(()) => {
                    eprintln!("theme-manager: [{}] wrote {}", writer.name(), path.display());
                    writer.reload();
                }
            }
        }
    }
}
