//! theme-set <theme-name>
//!
//! Writes the theme name to the state file so theme-daemon picks it up,
//! and writes every shell env file immediately so the calling shell can
//! source its own without waiting for the daemon.

use std::fs;
use theme_manager::writers::{apply_global_env, EnvSh, EnvSystemd, FishConf, HyprEnv};
use theme_manager::Writer;
use theme_manager::{
    amoled_enabled, env_sh_file, env_systemd_file, fish_conf_file, hypr_env_file,
    resolve_theme, set_amoled_enabled, state_file, themes_dir, write_file,
};

fn handle_amoled_option(args: &[String]) -> bool {
    if args.get(1).map(String::as_str) != Some("--amoled") {
        return false;
    }
    let current = amoled_enabled();
    let requested = match args.get(2).map(String::as_str) {
        Some("on" | "true" | "1") => true,
        Some("off" | "false" | "0") => false,
        Some("toggle") => !current,
        Some("status") | None => {
            println!("{}", if current { "on" } else { "off" });
            return true;
        }
        Some(value) => {
            eprintln!("theme-set: invalid AMOLED mode '{value}' (use on, off, toggle, or status)");
            std::process::exit(1);
        }
    };
    if let Err(error) = set_amoled_enabled(requested) {
        eprintln!("theme-set: could not save AMOLED mode: {error}");
        std::process::exit(1);
    }
    eprintln!(
        "theme-set: AMOLED mode {}",
        if requested { "enabled" } else { "disabled" }
    );
    true
}


fn write_env_file(
    writer: &dyn Writer,
    theme: &theme_manager::Theme,
    path: &std::path::Path,
    label: &str,
) {
    match writer.generate(theme) {
        Err(e) => {
            eprintln!("theme-set: failed to generate {label}: {e}");
            std::process::exit(1);
        }
        Ok(content) => {
            if let Err(e) = write_file(path, &content) {
                eprintln!("theme-set: failed to write {label}: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if handle_amoled_option(&args) {
        return;
    }

    if args.len() < 2 {
        eprintln!("usage: theme-set <theme-name> | theme-set --amoled <on|off|toggle|status>");
        std::process::exit(1);
    }

    let name = &args[1];
    let theme_path = themes_dir().join(name);

    if !theme_path.is_file() {
        eprintln!("theme-set: theme file not found: {}", theme_path.display());
        std::process::exit(1);
    }

    let theme = match resolve_theme(&theme_path) {
        Ok(theme) => theme,
        Err(e) => {
            eprintln!("theme-set: failed to resolve theme: {e}");
            std::process::exit(1);
        }
    };

    // Write env files immediately so the calling shell can source its own
    // and GUI/systemd targets are fresh even before the daemon applies.
    write_env_file(&FishConf, &theme, &fish_conf_file(), "fish conf");
    write_env_file(&EnvSh, &theme, &env_sh_file(), "posix env");
    write_env_file(&EnvSystemd, &theme, &env_systemd_file(), "systemd env");
    if HyprEnv.enabled() {
        write_env_file(&HyprEnv, &theme, &hypr_env_file(), "hypr env");
    } else {
        eprintln!("theme-set: [hypr-env] skipped (not installed)");
    }
    apply_global_env(&theme);

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
