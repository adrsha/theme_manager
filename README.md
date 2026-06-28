# theme-manager

A minimal Rust-based theming system for Hyprland. A single theme file drives
config generation for Ghostty, Hyprland, swaync, vicinae, GTK, Qt, and Neovim,
with wallpaper management via `awww`.

## How it works

Two binaries:

- **`theme-set <name>`** — activates a theme. Writes the fish env file
  immediately (so your shell picks it up) and writes the theme name to a state
  file that the daemon watches.
- **`theme-daemon`** — started once by Hyprland on login. Watches the state
  file via `inotify` and regenerates all config files whenever it changes.

## Installation

```bash
cargo build --release
cp target/release/theme-set ~/.local/bin/
cp target/release/theme-daemon ~/.local/bin/
```

Add to `hyprland.conf`:

```
exec-once = theme-daemon
```

Add to `~/.config/fish/config.fish` (replace any old `theme` alias):

```fish
function theme
    theme-set $argv[1]
    source ~/.config/fish/conf.d/theme_vars.fish
end

# Re-apply env vars on every new shell from whatever the daemon last set.
if test -f ~/.config/fish/conf.d/theme_vars.fish
    source ~/.config/fish/conf.d/theme_vars.fish
end
```

## Theme file format

Theme files live in `~/dots/themes/`. One file per theme, named without an
extension (e.g. `tokio_paper`, `everblush`).

```sh
# Required
VARIANT=dark          # must be 'dark' or 'light'

# Wallpaper (optional — omit or set to 'none' to show Hyprland background)
WALLPAPER=/home/you/walls/my_wallpaper.png

# Base16 colors
BASE00=#1a1b26
BASE01=#16161e
BASE02=#2f3549
BASE03=#444b6a
BASE05=#a9b1d6
BASE07=#cbccd1
BASE08=#f7768e
BASE09=#ff9e64
BASE0A=#e0af68
BASE0B=#9ece6a
BASE0C=#73daca
BASE0D=#7aa2f7
BASE0E=#bb9af7
BASE0F=#f7768e

# Extended palette (required by swaync, vicinae, hyprland)
BLACK=#1a1b26
DARKER_BLACK=#16161e
BLACK2=#1e202e
ONE_BG=#1e2030
ONE_BG2=#2f3549
ONE_BG3=#444b6a
GREY=#3b4261
WHITE=#a9b1d6
RED=#f7768e
GREEN=#9ece6a
YELLOW=#e0af68
BLUE=#7aa2f7
CYAN=#73daca
TEAL=#1abc9c
PINK=#ff007c
BABY_PINK=#ff75a0
PURPLE=#bb9af7
BASE03=#444b6a
```

All keys are available in your fish shell as `$THEME_<KEY>` (e.g.
`$THEME_BASE00`, `$THEME_VARIANT`) after switching themes.

## Usage

```fish
theme tokio_paper
theme everblush
theme kusanagi
```

The `theme` function updates configs, reloads running apps, and exports all
`THEME_*` vars into your current shell immediately. New terminals pick them up
automatically.

## What gets generated

| Target | Output file | Reload |
|---|---|---|
| Fish env | `~/.config/fish/conf.d/theme_vars.fish` | sourced by shell function |
| Ghostty | `~/dots/ghostty/themes/active` | manual (Ghostty reloads on file change) |
| Hyprland | `~/dots/hypr/colors.conf` | auto (Hyprland watches the file) |
| Vicinae | `~/dots/vicinae-themes/<name>.toml` | `vicinae theme set <name>` |
| swaync | `~/dots/swaync/style.css` | `swaync-client -rs` |
| Neovim patch | `/tmp/nvim_theme_patch.lua` | `v:lua.reload_nvchad_theme()` via socket |
| GTK 3 | `~/.config/gtk-3.0/settings.ini` | `gsettings` |
| GTK 4 | `~/.config/gtk-4.0/settings.ini` | via GTK 3 gsettings call |
| Qt6ct | `~/.config/qt6ct/qt6ct.conf` | requires Qt app restart |
| Wallpaper | — | `awww img <path>` or kills `awww-daemon` |
| Neovim base46 | cache | `nvim --headless` recompile |

### swaync

swaync's style is split into two files:

- `~/dots/swaync/styles_rules.css` — your hand-written rules (not touched)
- `~/dots/swaync/style.css` — generated: CSS variables prepended to your rules

Only edit `styles_rules.css`. `style.css` is overwritten on every theme change.

### GTK dark/light

`VARIANT=dark` sets `gtk-application-prefer-dark-theme = 1` and calls
`gsettings set org.gnome.desktop.interface color-scheme prefer-dark`.  
`VARIANT=light` does the reverse. Running GTK apps update immediately.

### Qt dark/light

`VARIANT=dark` sets `color_scheme_path` to `darker.conf`.  
`VARIANT=light` sets it to `airy.conf`.  
All other lines in `qt6ct.conf` are left untouched. Qt apps need a restart.

### Wallpaper

If `WALLPAPER` is unset, empty, `none`, or points to a file that doesn't
exist, `awww-daemon` is killed and Hyprland's `background_color` (from
`colors.conf`) shows instead.

If the file exists and `awww-daemon` is already running, `awww img <path>` is
called. If the daemon isn't running it is started first, then the wallpaper is
set after a short delay.

## Adding a new config target

1. Create `src/writers/my_app.rs`:

```rust
use crate::{Theme, ThemeError, Writer, dots};
use std::path::PathBuf;

pub struct MyApp;

impl Writer for MyApp {
    fn name(&self) -> &'static str { "my-app" }

    fn output_path(&self) -> PathBuf {
        dots().join("my-app/config")
    }

    fn generate(&self, theme: &Theme) -> Result<String, ThemeError> {
        let bg = theme.require("BASE00")?;
        let fg = theme.require("BASE05")?;
        Ok(format!("background = {bg}\nforeground = {fg}\n"))
    }

    // Optional: called after a successful write.
    fn reload(&self) {
        std::process::Command::new("my-app-reload").status().ok();
    }
}
```

2. Register it in `src/writers/mod.rs`:

```rust
mod my_app;
pub use my_app::MyApp;

pub fn all_writers(theme: &Theme) -> Vec<Box<dyn Writer>> {
    vec![
        // ... existing writers ...
        Box::new(MyApp),
    ]
}
```

That's it. Error reporting, file writing, and logging are handled by the
framework.

### Helpers available in `generate()`

```rust
theme.require("KEY")          // &str or MissingKey error
theme.require_rgb("KEY")      // (u8, u8, u8) parsed from hex, or error
hex_to_rgb("#rrggbb")         // (u8, u8, u8)
rgb_str((r, g, b))            // "r, g, b" — for CSS rgba()
dots()                        // PathBuf to ~/dots
home()                        // PathBuf to $HOME
```

If your writer needs `Theme` inside `reload()` (the trait doesn't pass it),
skip `reload()` and instead write a free function called directly from
`apply_theme()` in `lib.rs`, as done for wallpaper.

## State files

| Path | Contents |
|---|---|
| `~/.local/state/theme/current` | Active theme name, watched by daemon |
| `~/.config/fish/conf.d/theme_vars.fish` | Generated `set -gx` lines, auto-sourced by fish |
| `/tmp/nvim_theme_patch.lua` | Lua snippet patching `vim.env` in running nvim instances |
