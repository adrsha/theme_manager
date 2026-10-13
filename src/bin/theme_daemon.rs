//! theme-daemon
//!
//! Watches ~/.local/state/theme/current via inotify.
//! On change, parses the named theme and regenerates all config files.
//!
//! Start via Hyprland: exec-once = theme-daemon

use inotify::{Inotify, WatchMask};
use std::fs;
use std::path::PathBuf;
use theme_manager::{
    amoled_enabled, amoled_state_file, apply_amoled_palette, apply_style_defaults, apply_theme,
    dynamic::{current_wallpaper, resolve_dynamic_palette},
    env_sh_file, parse_theme, resolve_theme, state_file, themes_dir,
};

fn read_current_theme() -> Option<String> {
    let content = fs::read_to_string(state_file()).ok()?;
    let name = content.trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn load_and_apply(name: &str) {
    let path = themes_dir().join(name);
    match resolve_theme(&path) {
        Ok(theme) => apply_theme(&theme),
        Err(e) => eprintln!("theme-daemon: failed to resolve theme '{name}': {e}"),
    }
}

fn apply_changed_wallpaper(name: &str, wallpaper: &std::path::Path) {
    let result = (|| {
        let mut theme = parse_theme(&themes_dir().join(name))?;
        if theme.vars.get("MODE").map(String::as_str) != Some("dynamic") {
            return Ok::<_, theme_manager::ThemeError>(());
        }
        theme
            .vars
            .insert("WALLPAPER".into(), wallpaper.to_string_lossy().into_owned());
        resolve_dynamic_palette(&mut theme)?;
        if amoled_enabled() {
            apply_amoled_palette(&mut theme);
        }
        apply_style_defaults(&mut theme)?;
        apply_theme(&theme);
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("theme-daemon: wallpaper update failed: {error}");
    }
}

fn active_dynamic_theme(name: &str) -> bool {
    parse_theme(&themes_dir().join(name))
        .ok()
        .is_some_and(|theme| theme.vars.get("MODE").map(String::as_str) == Some("dynamic"))
}

fn ensure_state_file_exists() -> PathBuf {
    let state = state_file();
    if let Some(parent) = state.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| {
            eprintln!("theme-daemon: could not create state dir: {e}");
        });
    }
    // Create empty state file if it doesn't exist so inotify can watch it.
    if !state.exists() {
        fs::write(&state, "").unwrap_or_else(|e| {
            eprintln!("theme-daemon: could not create state file: {e}");
        });
    }
    state
}

fn main() {
    eprintln!("theme-daemon: starting");

    let state = ensure_state_file_exists();
    let amoled_state = amoled_state_file();
    if let Some(parent) = amoled_state.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|e| eprintln!("theme-daemon: could not create AMOLED state dir: {e}"));
    }
    if !amoled_state.exists() {
        fs::write(&amoled_state, "false\n")
            .unwrap_or_else(|e| eprintln!("theme-daemon: could not create AMOLED state file: {e}"));
    }

    // Apply the current theme immediately on startup.
    if let Some(name) = read_current_theme() {
        eprintln!("theme-daemon: applying initial theme '{name}'");
        load_and_apply(&name);
    } else {
        eprintln!("theme-daemon: no current theme set, waiting for theme-set");
    }

    let mut inotify = Inotify::init().unwrap_or_else(|e| {
        eprintln!("theme-daemon: failed to init inotify: {e}");
        std::process::exit(1);
    });

    // Watch the parent directory for CLOSE_WRITE on the state file.
    // Watching the file directly misses atomic writes (write to tmp + rename).
    let watch_dir = state.parent().expect("state file has no parent");

    inotify
        .watches()
        .add(watch_dir, WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO)
        .unwrap_or_else(|e| {
            eprintln!("theme-daemon: failed to add inotify watch: {e}");
            std::process::exit(1);
        });

    // Parent watches also catch editor/importer atomic replacements.
    let theme_watch = inotify
        .watches()
        .add(themes_dir(), WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO)
        .expect("could not watch theme directory");
    let env_path = env_sh_file();
    let env_watch = inotify
        .watches()
        .add(
            env_path.parent().unwrap(),
            WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO,
        )
        .expect("could not watch theme environment directory");
    let mut last_wallpaper = current_wallpaper();

    eprintln!("theme-daemon: watching {}", watch_dir.display());

    let mut buffer = [0u8; 1024];

    loop {
        let events = match inotify.read_events_blocking(&mut buffer) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("theme-daemon: inotify read error: {e}");
                continue;
            }
        };

        let state_filename = state.file_name();
        let amoled_filename = amoled_state.file_name();

        for event in events {
            if event.wd == theme_watch {
                if let Some(name) = read_current_theme() {
                    if event.name.and_then(|n| n.to_str()) == Some(name.as_str())
                        && active_dynamic_theme(&name)
                    {
                        load_and_apply(&name);
                        last_wallpaper = current_wallpaper();
                    }
                }
                continue;
            }
            if event.wd == env_watch {
                if event.name == env_path.file_name() {
                    let wallpaper = current_wallpaper();
                    if wallpaper != last_wallpaper {
                        last_wallpaper = wallpaper.clone();
                        if let (Some(name), Some(path)) = (read_current_theme(), wallpaper) {
                            apply_changed_wallpaper(&name, &path);
                            last_wallpaper = current_wallpaper();
                        }
                    }
                }
                continue;
            }
            // Theme selection and AMOLED mode share the same apply pipeline.
            if event.name != state_filename && event.name != amoled_filename {
                continue;
            }

            match read_current_theme() {
                Some(name) => {
                    eprintln!("theme-daemon: theme changed to '{name}'");
                    load_and_apply(&name);
                    last_wallpaper = current_wallpaper();
                }
                None => {
                    eprintln!("theme-daemon: state file is empty, ignoring");
                }
            }
        }
    }
}
