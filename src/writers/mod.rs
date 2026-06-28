//! Config writers — one module per target.
//!
//! # To add a new target:
//!
//!   1. Create `src/writers/my_app.rs`
//!   2. `pub struct MyApp;`
//!   3. `impl Writer for MyApp { ... }`
//!   4. `mod my_app; pub use my_app::MyApp;` here in mod.rs
//!   5. Add `Box::new(MyApp)` to `all_writers()` below.
//!
//! If your writer needs access to Theme inside reload() (e.g. wallpaper),
//! skip the trait's reload() and instead add a free function call in
//! apply_theme() in lib.rs, like apply_wallpaper() below.

mod fish_conf;
mod ghostty;
mod hyprland;
mod nvim;
mod swaync;
mod variant;
mod vicinae;
mod wallpaper;

pub use fish_conf::FishConf;
pub use ghostty::Ghostty;
pub use hyprland::Hyprland;
pub use nvim::NvimPatch;
pub use swaync::Swaync;
pub use variant::{GtkSettings, Gtk4Settings, Qt6ct};
pub use vicinae::Vicinae;
pub use wallpaper::apply_wallpaper;

use crate::{Theme, Writer};

/// The ordered list of all file-generating writers.
/// Add a `Box::new(MyApp)` here when you add a new target.
pub fn all_writers(theme: &Theme) -> Vec<Box<dyn Writer>> {
    vec![
        Box::new(Ghostty),
        Box::new(Hyprland),
        Box::new(Vicinae { name: theme.name.clone() }),
        Box::new(Swaync),
        Box::new(NvimPatch),
        Box::new(GtkSettings),
        Box::new(Gtk4Settings),
        Box::new(Qt6ct),
    ]
}

// Nvim socket reload lives here since it's not a simple file write.
pub fn reload_nvim(theme: &Theme) {
    use std::process::Command;

    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", get_uid()));

    let nvim_listen = std::env::var("NVIM_LISTEN_ADDRESS").unwrap_or_default();

    let sockets = list_nvim_sockets(&runtime_dir);

    for socket in &sockets {
        if socket == &nvim_listen {
            continue;
        }
        let result = Command::new("nvim")
            .args(["--server", socket, "--remote-expr", "v:lua.reload_nvchad_theme()"])
            .output();

        match result {
            Ok(o) if o.status.success() => {
                eprintln!("theme-manager: [nvim] reloaded at {socket}");
            }
            _ => eprintln!("theme-manager: [nvim] could not reload at {socket}"),
        }
    }

    let result = Command::new("nvim")
        .args(["--headless", "-c", "lua require('base46').compile()", "-c", "qa"])
        .output();

    match result {
        Ok(o) if o.status.success() => eprintln!("theme-manager: [nvim] recompiled base46 cache"),
        _ => eprintln!("theme-manager: [nvim] base46 recompile failed"),
    }

    let _ = theme;
}

fn list_nvim_sockets(runtime_dir: &str) -> Vec<String> {
    let mut sockets = Vec::new();
    let entries = match std::fs::read_dir(runtime_dir) {
        Ok(e) => e,
        Err(_) => return sockets,
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with("nvim.") {
            let socket = entry.path().join("0");
            if socket.exists() {
                sockets.push(socket.to_string_lossy().into_owned());
            }
        }
    }
    sockets
}

fn get_uid() -> u32 {
    extern "C" { fn getuid() -> u32; }
    unsafe { getuid() }
}
