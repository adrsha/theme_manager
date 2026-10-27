//! Wallpaper process management.
//!
//! The daemon owns the backdrop: stills are painted by `awww`, videos are
//! played by `mpvpaper`, and a theme with `WALLPAPER=none` clears both. Keeping
//! this here means clients only select a theme; they never have to start,
//! stop, or reconcile wallpaper daemons themselves.
use crate::{dynamic::current_wallpaper, Theme};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::Duration;

/// The last wallpaper this process applied. `None` means startup, so the
/// current theme's wallpaper is always reconciled once.
static APPLIED: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));

/// Reconcile the active wallpaper with the current theme.
///
/// The theme file is authoritative. For dynamic themes the daemon writes the
/// resolved path back into the snapshot, but a theme that explicitly declares
/// `none` must still clear an old wallpaper.
pub fn sync(theme: &Theme) {
    let requested = theme
        .vars
        .get("WALLPAPER")
        .map(String::as_str)
        .unwrap_or("none");
    let wanted = resolve(requested);

    if applied(&wanted) && running(&wanted) {
        return;
    }

    apply(&wanted);
    if let Ok(mut last) = APPLIED.lock() {
        *last = Some(wanted);
    }
}

/// Resolve `current`, missing files, and empty values to a concrete value.
fn resolve(requested: &str) -> String {
    let value = requested.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("none") {
        return "none".to_string();
    }

    let path = if value.eq_ignore_ascii_case("current") {
        match current_wallpaper() {
            Some(path) => path,
            None => {
                eprintln!("theme-manager: [wallpaper] WALLPAPER=current but no wallpaper is active");
                return "none".to_string();
            }
        }
    } else {
        PathBuf::from(value)
    };

    if !path.is_file() {
        eprintln!(
            "theme-manager: [wallpaper] '{}' not found, clearing the backdrop",
            path.display()
        );
        return "none".to_string();
    }
    path.to_string_lossy().into_owned()
}

fn applied(wanted: &str) -> bool {
    APPLIED
        .lock()
        .ok()
        .and_then(|last| last.clone())
        .as_deref()
        == Some(wanted)
}

/// Whether the process that should own `wanted` is already running.
fn running(wanted: &str) -> bool {
    if wanted == "none" {
        return !awww_running() && !mpvpaper_running();
    }
    if is_video(Path::new(wanted)) {
        mpvpaper_running()
    } else {
        awww_running()
    }
}

fn apply(wanted: &str) {
    if wanted == "none" {
        stop_mpvpaper();
        stop_awww();
        eprintln!("theme-manager: [wallpaper] cleared");
        return;
    }

    let path = Path::new(wanted);
    if is_video(path) {
        stop_awww();
        start_mpvpaper(path);
    } else {
        stop_mpvpaper();
        paint_still(path);
    }
}

fn paint_still(path: &Path) {
    if !awww_running() {
        let started = Command::new("awww-daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match started {
            Ok(_) => {
                eprintln!("theme-manager: [wallpaper] started awww-daemon");
                thread::sleep(Duration::from_millis(600));
            }
            Err(error) => {
                eprintln!("theme-manager: [wallpaper] could not start awww-daemon: {error}");
                return;
            }
        }
    }

    match Command::new("awww").arg("img").arg(path).status() {
        Ok(status) if status.success() => {
            eprintln!("theme-manager: [wallpaper] set to '{}'", path.display())
        }
        Ok(status) => eprintln!("theme-manager: [wallpaper] awww img failed with {status}"),
        Err(error) => eprintln!("theme-manager: [wallpaper] awww unavailable: {error}"),
    }
}

fn start_mpvpaper(path: &Path) {
    // -f forks so the daemon can reap the starter process; mpvpaper keeps the
    // actual player in the background. Audio is never wanted for a wallpaper.
    let result = Command::new("mpvpaper")
        .args(["-f", "-o", "no-audio loop", "ALL"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match result {
        Ok(status) if status.success() => {
            eprintln!(
                "theme-manager: [wallpaper] playing '{}' with mpvpaper",
                path.display()
            )
        }
        Ok(status) => eprintln!("theme-manager: [wallpaper] mpvpaper exited with {status}"),
        Err(error) => eprintln!("theme-manager: [wallpaper] mpvpaper unavailable: {error}"),
    }
}

fn awww_running() -> bool {
    Command::new("awww")
        .arg("query")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn mpvpaper_running() -> bool {
    Command::new("pgrep")
        .args(["-f", "mpvpaper.*ALL"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Stops awww through its CLI. `killall` does not match the Nix wrapper's
/// truncated process name (`.awww-daemon-wr`), so the daemon keeps displaying
/// the old image.
fn stop_awww() {
    let stopped = Command::new("awww")
        .arg("kill")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    if !stopped {
        let _ = Command::new("pkill")
            .args(["-f", "awww-daemon"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// mpvpaper's Nix wrapper also hides behind a shortened process name, so use a
/// command-line match with a pkill fallback.
fn stop_mpvpaper() {
    let _ = Command::new("killall")
        .args(["mpvpaper", "--quiet"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("pkill")
        .args(["-f", "mpvpaper.*ALL"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Video files are played with mpvpaper; everything else is a still.
fn is_video(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some("mp4" | "m4v" | "mkv" | "mov" | "webm" | "avi" | "mpg" | "mpeg")
    )
}

#[cfg(test)]
mod tests {
    use super::{is_video, resolve};
    use std::path::Path;

    #[test]
    fn detects_common_video_extensions_case_insensitively() {
        for path in ["wall.mp4", "wall.WEBM", "wall.mkv", "wall.mov", "wall.avi"] {
            assert!(is_video(Path::new(path)), "{path} should be a video");
        }
    }

    #[test]
    fn leaves_awww_supported_images_untouched() {
        for path in ["wall.jpg", "wall.jpeg", "wall.png", "wall.gif", "wall.webp"] {
            assert!(
                !is_video(Path::new(path)),
                "{path} should go directly to awww"
            );
        }
    }

    #[test]
    fn empty_none_and_missing_files_clear_the_backdrop() {
        assert_eq!(resolve(""), "none");
        assert_eq!(resolve("none"), "none");
        assert_eq!(resolve("NONE"), "none");
        assert_eq!(resolve("/definitely/not/a/wallpaper.jpg"), "none");
    }
}
