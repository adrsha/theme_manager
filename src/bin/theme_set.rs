//! theme-set <theme-name>
//!
//! Writes the theme name to the state file so theme-daemon picks it up,
//! and writes the fish conf.d file immediately so the calling shell can
//! source it.

use std::fs;
use theme_manager::{parse_theme, state_file, themes_dir, write_file, fish_conf_file};
use theme_manager::writers::FishConf;
use theme_manager::Writer;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("usage: theme-set <theme-name>");
        std::process::exit(1);
    }

    let name = &args[1];
    let theme_path = themes_dir().join(name);

    if !theme_path.is_file() {
        eprintln!("theme-set: theme file not found: {}", theme_path.display());
        std::process::exit(1);
    }

    let theme = match parse_theme(&theme_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("theme-set: failed to parse theme: {e}");
            std::process::exit(1);
        }
    };

    // Write fish conf.d immediately so the calling shell function can source it.
    match FishConf.generate(&theme) {
        Err(e) => {
            eprintln!("theme-set: failed to generate fish conf: {e}");
            std::process::exit(1);
        }
        Ok(content) => {
            if let Err(e) = write_file(&fish_conf_file(), &content) {
                eprintln!("theme-set: failed to write fish conf: {e}");
                std::process::exit(1);
            }
        }
    }

    // Write state file — this is what the daemon watches.
    let state = state_file();
    if let Some(parent) = state.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            eprintln!("theme-set: could not create state dir: {e}");
            std::process::exit(1);
        }
    }

    if let Err(e) = fs::write(&state, name) {
        eprintln!("theme-set: could not write state file: {e}");
        std::process::exit(1);
    }

    eprintln!("theme-set: set theme to '{name}'");
}
