use crate::{home, Theme, ThemeError, Writer};
use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf};

pub struct Tty;

// Standard base16 -> ANSI order: black..white, then the bright row.
const SLOTS: [&str; 16] = [
    "BASE00", "BASE08", "BASE0B", "BASE0A", "BASE0D", "BASE0E", "BASE0C", "BASE05",
    "BASE03", "BASE08", "BASE0B", "BASE0A", "BASE0D", "BASE0E", "BASE0C", "BASE07",
];

// Linux O_NOCTTY: the daemon is a session leader, and opening a tty without
// this flag could make it the daemon's controlling terminal.
const O_NOCTTY: i32 = 0o400;

impl Writer for Tty {
    fn name(&self) -> &'static str { "tty" }

    fn output_path(&self) -> PathBuf {
        home().join(".local/state/theme/tty_colors")
    }

    fn enabled(&self) -> bool { true }

    // The Linux console palette escape: ESC ] P <hex index> <rrggbb>
    fn generate(&self, theme: &Theme) -> Result<String, ThemeError> {
        let mut out = String::new();
        for (i, key) in SLOTS.iter().enumerate() {
            let hex = theme.require(key)?.trim_start_matches('#');
            out.push_str(&format!("\x1b]P{i:X}{hex}"));
        }
        Ok(out)
    }

    // Repaint every TTY we're allowed to write to (best-effort).
    fn reload(&self) {
        let Ok(seq) = fs::read(self.output_path()) else { return };
        for n in 1..=6 {
            let tty = fs::OpenOptions::new()
                .write(true)
                .custom_flags(O_NOCTTY)
                .open(format!("/dev/tty{n}"));
            if let Ok(mut tty) = tty {
                let _ = tty.write_all(&seq);
            }
        }
    }
}
