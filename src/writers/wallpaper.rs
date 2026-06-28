use crate::Theme;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

pub fn apply_wallpaper(theme: &Theme) {
    let path_str = match theme.vars.get("WALLPAPER") {
        Some(s) if !s.is_empty() && s != "none" => s,
        _ => {
            kill_awww();
            return;
        }
    };

    let path = Path::new(path_str);
    if !path.is_file() {
        eprintln!(
            "theme-manager: [wallpaper] '{}' not found — killing awww",
            path.display()
        );
        kill_awww();
        return;
    }

    if awww_running() {
        set_wallpaper(path_str);
    } else {
        start_awww_daemon();
        thread::sleep(Duration::from_millis(500));
        set_wallpaper(path_str);
    }
}

fn awww_running() -> bool {
    Command::new("awww")
        .arg("query")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn kill_awww() {
    // Exit code 1 just means no process was running — not an error worth logging.
    let _ = Command::new("killall")
        .args(["awww-daemon", "--quiet"])
        .status();
}

fn start_awww_daemon() {
    let result = Command::new("awww-daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    match result {
        Ok(_) => eprintln!("theme-manager: [wallpaper] started awww-daemon"),
        Err(e) => eprintln!("theme-manager: [wallpaper] could not start awww-daemon: {e}"),
    }
}

fn set_wallpaper(path: &str) {
    let result = Command::new("awww").args(["img", path]).status();
    match result {
        Ok(s) if s.success() => eprintln!("theme-manager: [wallpaper] set to '{path}'"),
        Ok(s) => eprintln!("theme-manager: [wallpaper] awww img failed with {s}"),
        Err(e) => eprintln!("theme-manager: [wallpaper] awww not found: {e}"),
    }
}
