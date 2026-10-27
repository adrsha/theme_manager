# theme-manager

A minimal Rust-based theming system for Hyprland and Mango. A single theme file
drives config generation for Ghostty, Hyprland, Mango, swaync, vicinae, GTK,
Qt, and Neovim. The daemon also owns the wallpaper: `awww` paints stills,
`mpvpaper` plays videos, and the resolved state is published to
`~/.local/state/theme/active.json` for shell clients.

## How it works

Two binaries:

- **`theme-set <name>`** — activates a theme. Writes every shell env file
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

Runtime helpers, all optional at build time and looked up on `PATH`:

| Helper | Used for |
|---|---|
| `matugen` | `MATUGEN_SCHEME` Material 3 palettes |
| `ffmpeg` | fallback palette extraction and video frames |
| `awww` | still-image wallpapers |
| `mpvpaper` | video wallpapers |
| `procps` (`pgrep`/`pkill`) | wallpaper process detection and cleanup |

Missing helpers are logged; themes that do not need them keep working.

Add to `hyprland.lua`:

```lua
hl.on("hyprland.start", function()
    hl.exec_cmd("theme-daemon")
end)
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

For bash/zsh/sh, source the POSIX file instead (e.g. in `~/.bashrc`):

```sh
theme() {
    command theme-set "$1" && . "$HOME/.config/theme-manager/theme_env.sh"
}

[ -f "$HOME/.config/theme-manager/theme_env.sh" ] && . "$HOME/.config/theme-manager/theme_env.sh"
```

For Hyprland-launched apps, add to `~/.config/hypr/hyprland.lua`
(after `require("colors")`):

```lua
require("theme_env")
```

No setup is needed for systemd user services, portals, or D-Bus-activated
apps: `~/.config/environment.d/theme-manager.conf` is loaded automatically
at login, and every switch is pushed live into the running session (see
"Global environment variables" below). Rebuild, reinstall, and restart the
daemon after updating sources so it learns the new writers.

## Theme file format

Theme files live in `~/.config/theme-manager/themes/`. One file per theme, named without an
extension (e.g. `tokio_paper`, `everblush`).

```sh
# Required
VARIANT=dark          # must be 'dark' or 'light'

# Optional color mode. When dynamic, only VARIANT and WALLPAPER are required;
# all omitted palette colors are extracted from the wallpaper.
MODE=dynamic

# Wallpaper (required by MODE=dynamic; otherwise optional)
WALLPAPER=/home/you/walls/my_wallpaper.png

# Optional Material 3 scheme for MODE=dynamic. Accepted values match matugen:
# content, expressive, fidelity, fruit-salad, monochrome, neutral, rainbow,
# tonal-spot, vibrant, smart. The `scheme-` prefix is also accepted.
MATUGEN_SCHEME=tonal-spot

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

# Optional transparent surface default; controls and text retain palette colors
TRANSPARENT=false

# Shared Hyprland geometry (optional defaults shown)
CORNER_RADIUS=20
BORDER_SIZE=0
BORDER_COLOR=#2f3549

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

All keys are available everywhere as `$THEME_<KEY>` (e.g.
`$THEME_BASE00`, `$THEME_VARIANT`): fish reads `theme_vars.fish`, other
shells source `theme_env.sh`, and GUI apps inherit them via
`environment.d`/Hyprland (see "Global environment variables").

### Dynamic colors

Set `MODE=dynamic` to generate the full Base16 and extended palette directly
from `WALLPAPER`. In this mode a theme can be as small as:

```sh
VARIANT=dark
MODE=dynamic
WALLPAPER=/home/you/walls/city.webm
```

Use `WALLPAPER=current` for a reusable palette that extracts colors from the
wallpaper active immediately before the theme switch. The resolved absolute
path is then carried through to the generated environment and wallpaper
manager.

Set `MATUGEN_SCHEME` to let [matugen](https://github.com/InioX/matugen) generate
a Material 3 palette. The value is passed to `matugen image --type`; both
`tonal-spot` and `scheme-tonal-spot` spellings work. Material supplies the
neutral surfaces and the primary/secondary/tertiary/error roles; the remaining
semantic accents are derived from the scheme's own saturation and lightness, so
`expressive`, `rainbow`, `vibrant`, and `monochrome` produce visibly different
palettes. NvChad/base46 keys that Material does not define (`GREY_FG`, `LINE`,
`VIBRANT_GREEN`, `NORD_BLUE`, `SUN`, `DARK_PURPLE`, `ORANGE`, `STATUSLINE_BG`,
`LIGHTBG`, `PMENU_BG`, `FOLDER_BG`) are derived from the generated palette too,
so chadrc never falls back to its built-in onedark theme. Individual color keys
still override generated values.

Without `MATUGEN_SCHEME`, the manager falls back to its FFmpeg-based extractor:
it decodes a representative frame from JPEG, PNG, GIF, animated WebP, or a
supported video, analyzes its dominant colors, and creates variant-appropriate
backgrounds, foregrounds, and semantic accents. In either mode a valid
wallpaper is required; an extraction error stops the switch instead of applying
a partially generated theme.

With `MATUGEN_SCHEME` set, backgrounds and foregrounds still come from matugen,
but every accent (`BASE08`-`BASE0F`, `TEAL`) is built from up to four distinct
hues found in the wallpaper. A wallpaper with few colors reuses those hues with
small hue/lightness variations instead of introducing hues it doesn't contain.
A grayscale wallpaper falls back to matugen's primary hue.
Brightness is matched too: on a wallpaper darker than mid-gray (dark variant)
or brighter than mid-gray (light variant), accents, text and especially the
secondary/comment gray (`BASE04`, `GREY_FG`) are pulled toward the background,
by up to 40% for the secondary gray at the extreme. Mid-gray and opposite-side
wallpapers are unchanged.

## Usage

```fish
theme tokio_paper
theme everblush
theme kusanagi
```

The `theme` function updates configs, reloads running apps, and exports all
`THEME_*` vars into your current shell immediately. New terminals pick them up
automatically.

## AMOLED mode

AMOLED is a persistent manager-wide override, not a separate palette file. It
is applied after static or dynamic theme resolution and before every writer,
so switching themes keeps true-black backgrounds across Ghostty, Hyprland,
swaync, Vicinae, Neovim, Zathura, and the generated Fish environment.

```fish
theme-set --amoled on
theme-set --amoled off
theme-set --amoled toggle
theme-set --amoled status
```

The mode is stored in `~/.local/state/theme/amoled`. Enabling it changes the
effective variant to dark, uses `#000000` for the main background, builds
sub-surfaces from `#040404` through `#181818`, and installs readable light
foregrounds. Theme-specific accent colors are preserved, including when the
source theme is light or dynamically generated. Changing this option causes
the daemon to reapply the current theme and publish updated API snapshot.

## Transparent backgrounds

`TRANSPARENT=true` in a theme sets its default. Validate values strictly: only
`true` or `false` accepted. Its persistent override wins over theme defaults
until reset with `theme-set --transparent auto`:

```sh
theme-set --transparent on
theme-set --transparent off
theme-set --transparent auto
theme-set --transparent status
theme-set --transparent toggle
```

Override lives in `~/.local/state/theme/transparent` (`auto`, `true`, or
`false`). Daemon watches AMOLED state and regenerates targets. Consumers can
watch API snapshot for resolved opacity after theme apply. Directly toggling
Transparent in shell requires installed, running theme-manager binaries.

Supported surfaces: Ghostty main background (`background-opacity=0.02`), swaync
notification/control-center backgrounds (`rgba`), and Vicinae main/secondary
backgrounds (`#AARRGGBB`). Opaque mode restores original backgrounds. Foregrounds,
selection colors, and button accents remain colored. Hyprland provides blur
where configured; actual sampled blur depends on compositor rules and window
namespace. Existing GTK/Qt application themes, Zathura, Neovim, and arbitrary
third-party apps have no general window-opacity option in this manager; their
palettes stay intact. Restart apps that do not reload their appearance live.

## What gets generated

| Target | Output file | Reload |
|---|---|---|
| Fish env | `~/.config/fish/conf.d/theme_vars.fish` | auto-sourced from `conf.d` |
| POSIX env | `~/.config/theme-manager/theme_env.sh` | source from `~/.bashrc`/`~/.zshrc` (snippet in header) |
| systemd env | `~/.config/environment.d/theme-manager.conf` | auto-loaded at login + pushed live on every switch |
| Hyprland env | `~/.config/hypr/theme_env.lua` | `require("theme_env")` in `hyprland.lua`, live via `hyprctl setenv` |
| Ghostty | `~/.config/ghostty/themes/active` | `config-file = ~/.config/ghostty/themes/active` |
| Hyprland | `~/.config/hypr/colors.lua` | `require("colors")` in `hyprland.lua`, then `hyprctl reload config-only` |
| Mango           | `~/.config/mango/theme.conf`                                                      | `source_optional=~/.config/mango/theme.conf` in `config.conf` (below your own colors), then `mmsg dispatch reload_config` |
| Kitty | `~/.config/kitty/themes/active.conf` | `include themes/active.conf` in `kitty.conf` |
| Vicinae | `~/.config/vicinae/themes/<name>.toml` | `vicinae theme set <name>` |
| swaync | `~/.config/swaync/theme-vars.css` | `@import url("theme-vars.css");` at top of your `style.css`, then `swaync-client -rs` |
| Firefox palette | `~/.config/theme-manager/firefox-colors.css` | import from profile `chrome/userChrome.css`; use the variables in your own rules; restart Firefox |
| Zathura | `~/.config/zathura/themes/active` | `include <path>` in `zathurarc` (exact line in file header) |
| Neovim patch | `/tmp/nvim_theme_patch.lua` | `dofile("/tmp/nvim_theme_patch.lua")` in nvim config |
| GTK 3 | `~/.config/gtk-3.0/theme-manager.ini` | snippet: merge `[Settings]` into `settings.ini` once (header says how) |
| GTK 4 | `~/.config/gtk-4.0/theme-manager.ini` | snippet: merge `[Settings]` into `settings.ini` once (header says how) |
| GTK 3/4 CSS | `~/.local/share/themes/ThemeManager/gtk-3.0/gtk.css` and `gtk-4.0/gtk.css` | selected by `GTK_THEME=ThemeManager` or GNOME `gtk-theme`; restart apps |
| App-neutral API | `~/.local/state/theme/active.json` | atomic update; watch parent directory for `MOVED_TO` |
| Qt6ct | `~/.config/qt6ct/theme-manager.conf`, `~/.config/qt6ct/colors/theme-manager.conf` | copy `color_scheme_path` and `style` into `qt6ct.conf` once; generated colors update on switches |
| Wallpaper | `~/.local/state/theme/active.json` | daemon paints with `awww` / `mpvpaper`; clients watch `values.WALLPAPER` |
| Neovim base46 | cache | `nvim --headless` recompile |

Writers for Ghostty, Hyprland, Kitty, Vicinae, swaync, Firefox, and Zathura are
conditional: each one runs only when its binary is on `PATH` or its config
directory already exists under `$HOME`. Uninstalled apps are skipped so theme
switches don't create config bloat. Firefox recognizes both `~/.mozilla/firefox`
and `~/.config/mozilla/firefox` as profile roots.

### Global environment variables

Every switch publishes the same set — `THEME_NAME`, `THEME_PATH`,
`GTK_THEME`, `QT_QPA_PLATFORMTHEME`, plus `THEME_<KEY>` for each resolved
theme key — through four layers:

1. **Fish** (`theme_vars.fish`) — auto-sourced from `conf.d`.
2. **POSIX shells** (`theme-manager/theme_env.sh`) — `export` lines sourced
   by bash/zsh/sh rc files.
3. **systemd/D-Bus** (`environment.d/theme-manager.conf`) — read by the
   systemd user manager at login, so portals, Electron apps, and user
   services inherit the palette.
4. **Hyprland** (`hypr/theme_env.lua`) — `hl.env(...)` calls for apps
   launched from the compositor (conditional: only written when Hyprland
   is installed).

Layers 3 and 4 are also pushed **live** on every apply, so no relogin is
needed: `systemctl --user set-environment`, `dbus-update-activation-environment
--systemd`, and one `hyprctl setenv` per variable. All three are
best-effort — a missing helper on a headless host only logs.

### Ghostty

After writing the active theme, the daemon asks Ghostty to reload its config.
It first uses Ghostty's supported systemd user service:

```bash
systemctl reload --user app-com.mitchellh.ghostty.service
```

If Ghostty was launched directly instead of through systemd/D-Bus, the manager
falls back to sending its documented config-reload signal (`SIGUSR2`) to an
exactly named `ghostty` process. Both methods are best-effort, so switching a
theme still succeeds when Ghostty is not installed or not running.

### Firefox

The Firefox writer publishes **palette variables only**, not a complete browser
skin. It never edits your profile or your existing stylesheets.

1. In `about:config`, set
   `toolkit.legacyUserProfileCustomizations.stylesheets` to `true`.
2. Open `about:support`, find **Profile Directory**, and open it. Create a
   `chrome` directory there if needed, then create or edit `chrome/userChrome.css`.
3. Put the following import at the top, before any style rules. Replace
   `/home/YOU` with your actual home directory (Firefox does not expand `~`):

   ```css
   @import url("file:///home/YOU/.config/theme-manager/firefox-colors.css");

   /* Example rules; adapt these to your own Firefox skin. */
   #navigator-toolbox {
     background-color: var(--tm-bg) !important;
     color: var(--tm-fg) !important;
   }
   ```

   After a theme is applied, the generated file's header contains the exact
   import URL for your machine, including URL escaping where needed.
4. Fully quit and restart Firefox. Repeat the restart after each theme switch;
   Firefox does not hot-reload chrome CSS.

Available variables: `--tm-deep`, `--tm-bg`, `--tm-surface`, `--tm-raised`,
`--tm-hover`, `--tm-border`, `--tm-fg`, `--tm-dim`, `--tm-accent`,
`--tm-on-accent`, and `--tm-radius`. Importing them alone does not change the UI;
your rules must consume them. Firefox UI selectors may change between versions.
Sandboxed Firefox packages may need additional file-access permissions to read
the generated palette outside the profile.

### swaync

The manager owns only the variables file and never touches your stylesheet:

- `~/.config/swaync/theme-vars.css` — generated variables (overwritten each switch)
- `~/.config/swaync/style.css` — yours, never touched

One-time setup: put this line at the very top of your `style.css`, then write
your own rules below it:

```css
@import url("theme-vars.css");
```

Reload with `swaync-client -rs` (done automatically when swaync is running).

### GTK and system dark/light

`VARIANT=dark` sets `gtk-application-prefer-dark-theme = 1`, the GNOME
`color-scheme` to `prefer-dark`, and the GTK theme to `ThemeManager`.
`VARIANT=light` sets corresponding light values and also selects `ThemeManager`. The D-Bus/systemd
activation environment also receives `THEME_VARIANT`, `GTK_THEME`, and
`QT_QPA_PLATFORMTHEME`, so newly launched portal, GTK, Electron, and Qt apps
inherit the selected variant. Running apps that watch the XDG settings portal
update immediately. GTK 3/4 theme CSS is regenerated from Base16 colors on
switch: window surfaces, headerbars, buttons, entries, switches, checks, tabs,
lists, menus, scrollbars, focus/selection, and disabled states. GTK 3 and GTK 4
styles now use the Darkly GTK widget rules (LGPL-2.1-or-later), recolored from
the active palette, with dark/light GTK 3 variants and bundled symbolic assets.
Theme files live under `~/.local/share/themes/ThemeManager/`; apps generally
need a restart to pick up CSS changes. Libadwaita apps may ignore custom GTK
CSS themes.
For file-based setups without a portal, merge the
`theme-manager.ini` snippet (written next to each `settings.ini`, header
explains how) into your own `settings.ini` once — the manager never edits
your `settings.ini` itself.

### App-neutral theme API

After each apply, `theme-daemon` atomically replaces
`~/.local/state/theme/active.json` (schema version `1`). Any client can read
it at startup and watch `~/.local/state/theme/` for `MOVED_TO` on
`active.json`, then reopen it. No shell-specific IPC or polling required.

```json
{"version":1,"name":"example","variant":"dark","amoled":false,"opacity":1,"colors":{"BASE00":"#1a1b26"},"values":{"BASE00":"#1a1b26","VARIANT":"dark"}}
```

`colors` contains resolved `#RRGGBB` palette entries; `values` contains all
resolved theme keys (including geometry and wallpaper). Unknown keys may be
ignored. `opacity` uses the resolved `TRANSPARENT` value (`1` means opaque).
`theme-set --amoled ...` changes state; daemon publishes refreshed snapshot.
Theme selection still uses `theme-set <name>` and `~/.local/state/theme/current`.
GTK's `ThemeManager` theme name is unrelated to any shell framework. Existing
old GTK theme directories are left alone; remove them manually if unwanted.
Generated Darkly GTK3/4 styles add restrained 4/8px spacing to toolbar buttons,
menus, tabs, inputs, and popovers. LibreOffice uses GTK3 VCL on this system,
but its document grid and some toolbar layout are drawn internally: GTK CSS
cannot change all LibreOffice spacing. Restart LibreOffice after updating theme.

### Qt dark/light

Theme switches generate a 22-role Qt6ct palette at
`~/.config/qt6ct/colors/theme-manager.conf` and Darkly-compatible KDE colors at
`~/.local/share/color-schemes/ThemeManager.colors`. Copy `color_scheme_path` and
`style=darkly` from `~/.config/qt6ct/theme-manager.conf` into `[Appearance]` of
your `~/.config/qt6ct/qt6ct.conf` once. Install Darkly Qt6 style plugin
separately; theme-manager does not compile or ship it. Without the plugin,
keep your existing `style` line and still use the generated Qt palette. KDE
apps may need the `ThemeManager` color scheme selected separately. Restart Qt
apps after switching; your `qt6ct.conf` is never modified by theme-manager.

### Wallpaper

`theme-daemon` resolves `WALLPAPER` from the selected theme and owns the
backdrop processes:

- still images are painted with `awww` (the daemon starts `awww-daemon` when
  needed),
- videos are played with `mpvpaper` (muted, looping, on every output),
- `WALLPAPER=none`, a missing file, or clearing the theme stops both.

Switching between a still and a video stops the previous helper first, so the
two never fight over the background. The resolved state is atomically published
to `~/.local/state/theme/active.json`; shell clients watch it for display, but
they no longer start or stop wallpaper daemons themselves. Helper processes are
best-effort: a missing `awww` or `mpvpaper` is logged and the rest of the theme
still applies.

## Adding a new config target

Writer contract: a writer NEVER reads, patches, or overwrites the user's own
config files. It owns exactly one generated file, written wholesale on every
switch. The user imports that file from their own config. Every generated
file starts with a header comment: "Generated by theme-manager" plus a
"To use:" tutorial with the exact import line. Formats without an import
mechanism (GTK `settings.ini`, `qt6ct.conf`) get a merge-snippet whose header
explains the one-time manual copy.

1. Create `src/writers/my_app.rs`:

```rust
use crate::{home, Theme, ThemeError, Writer};
use std::path::PathBuf;

pub struct MyApp;

impl Writer for MyApp {
    fn name(&self) -> &'static str { "my-app" }

    fn output_path(&self) -> PathBuf {
        // Manager-owned file next to the user config — never the user
        // config itself.
        home().join(".config/my-app/theme-manager.conf")
    }

    // Skip output when the app isn't installed, so theme switches
    // don't create config bloat.
    fn enabled(&self) -> bool {
        crate::app_enabled("my-app", ".config/my-app")
    }

    fn generate(&self, theme: &Theme) -> Result<String, ThemeError> {
        let bg = theme.require("BASE00")?;
        let fg = theme.require("BASE05")?;
        Ok(format!(
            "# Generated by theme-manager. Do not edit manually.\n\
             # To use: add this line to ~/.config/my-app/config:\n\
             #   include ~/.config/my-app/theme-manager.conf\n\
             #\n\
             background = {bg}\nforeground = {fg}\n"
        ))
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
home()                        // PathBuf to $HOME
binary_exists("ghostty")      // true when a binary is on PATH
app_enabled("ghostty", ".config/ghostty") // binary OR config dir exists
```

If your writer needs `Theme` inside `reload()` (the trait doesn't pass it),
skip `reload()` and instead write a free function called directly from
`apply_theme()` in `lib.rs`, as done for wallpaper.

## State files

| Path | Contents |
|---|---|
| `~/.local/state/theme/current` | Active theme name, watched by daemon |
| `~/.local/state/theme/amoled` | Persistent manager-wide AMOLED override (`true` or `false`) |
| `~/.local/state/theme/transparent` | Persistent transparent override (`auto`, `true`, or `false`) |
| `~/.config/fish/conf.d/theme_vars.fish` | Generated `set -gx` lines, auto-sourced by fish |
| `~/.config/theme-manager/theme_env.sh` | Generated POSIX `export` lines for bash/zsh/sh |
| `~/.config/environment.d/theme-manager.conf` | Generated systemd `environment.d` snippet for GUI apps/services |
| `~/.config/hypr/theme_env.lua` | Generated `hl.env(...)` lines for Hyprland-launched apps |
| `/tmp/nvim_theme_patch.lua` | Lua snippet patching `vim.env` in running nvim instances |
