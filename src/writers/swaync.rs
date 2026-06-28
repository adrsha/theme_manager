use crate::{Theme, ThemeError, Writer, dots, rgb_str};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub struct Swaync;

impl Writer for Swaync {
    fn name(&self) -> &'static str { "swaync" }

    fn output_path(&self) -> PathBuf { dots().join("swaync/style.css") }

    fn generate(&self, theme: &Theme) -> Result<String, ThemeError> {
        let base00 = rgb_str(theme.require_rgb("BASE00")?);
        let one_bg = rgb_str(theme.require_rgb("ONE_BG")?);
        let one_bg2 = rgb_str(theme.require_rgb("ONE_BG2")?);
        let one_bg3 = rgb_str(theme.require_rgb("ONE_BG3")?);
        let black2 = rgb_str(theme.require_rgb("BLACK2")?);
        let darker_black = rgb_str(theme.require_rgb("DARKER_BLACK")?);
        let cyan = rgb_str(theme.require_rgb("CYAN")?);
        let white = rgb_str(theme.require_rgb("WHITE")?);
        let base03 = rgb_str(theme.require_rgb("BASE03")?);
        let blue = rgb_str(theme.require_rgb("BLUE")?);

        let vars_css = format!(
            ":root {{\n\
             \t--cc-bg:                      rgba({base00}, 1);\n\
             \n\
             \t--noti-border-color:          rgba({cyan}, 0.15);\n\
             \n\
             \t--noti-bg:                    {one_bg};\n\
             \t--noti-bg-alpha:              1;\n\
             \t--noti-bg-darker:             rgb({darker_black});\n\
             \t--noti-bg-hover:              rgb({one_bg2});\n\
             \t--noti-bg-focus:              rgba({cyan}, 0.2);\n\
             \n\
             \t--noti-close-bg:              rgb({black2});\n\
             \t--noti-close-bg-hover:        rgb({one_bg3});\n\
             \n\
             \t--text-color:                 rgb({white});\n\
             \t--text-color-disabled:        rgb({base03});\n\
             \n\
             \t--bg-selected:                rgb({blue});\n\
             \n\
             \t--notification-icon-size:       64px;\n\
             \t--notification-app-icon-size:   calc(var(--notification-icon-size) / 3);\n\
             \t--notification-group-icon-size: 32px;\n\
             \n\
             \t--border:                     0px solid var(--noti-border-color);\n\
             \t--border-radius:              12px;\n\
             \n\
             \t--notification-shadow:\n\
             \t\t0 0 0 1px rgba(0, 0, 0, 0.45),\n\
             \t\t0 2px 6px rgba(0, 0, 0, 0.65);\n\
             \n\
             \t--font-size-body:             15px;\n\
             \t--font-size-summary:          16px;\n\
             \n\
             \t--hover-transition:           background 0.15s ease-in-out;\n\
             \t--group-collapse-transition:  opacity 400ms ease-in-out;\n\
             }}\n\
             \n\
             /* GTK fallback */\n\
             @define-color cc-bg                  rgba({base00}, 1);\n\
             @define-color noti-border-color      rgba({cyan}, 0.15);\n\
             \n\
             @define-color noti-bg                rgba({one_bg}, 1);\n\
             @define-color noti-bg-opaque         rgb({one_bg});\n\
             @define-color noti-bg-darker         rgb({darker_black});\n\
             \n\
             @define-color noti-bg-hover          rgb({one_bg2});\n\
             @define-color noti-bg-hover-opaque   rgb({one_bg2});\n\
             \n\
             @define-color noti-bg-focus          rgba({cyan}, 0.2);\n\
             \n\
             @define-color noti-close-bg          rgba({black2}, 1);\n\
             @define-color noti-close-bg-hover    rgba({one_bg3}, 1);\n\
             \n\
             @define-color text-color             rgb({white});\n\
             @define-color text-color-disabled    rgb({base03});\n\
             \n\
             @define-color bg-selected            rgb({blue});\n\
             \n"
        );

        let rules_path = dots().join("swaync/styles_rules.css");
        if !rules_path.exists() {
            eprintln!(
                "theme-manager: [swaync] {} not found — writing vars only",
                rules_path.display()
            );
            return Ok(vars_css);
        }

        let rules_css = fs::read_to_string(&rules_path)?;
        Ok(vars_css + &rules_css)
    }

    fn reload(&self) {
        let running = Command::new("pgrep")
            .args(["--exact", "swaync"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !running {
            return;
        }

        let status = Command::new("swaync-client").arg("-rs").status();
        match status {
            Ok(s) if s.success() => eprintln!("theme-manager: [swaync] reloaded"),
            Ok(s) => eprintln!("theme-manager: [swaync] client exited with {s}"),
            Err(e) => eprintln!("theme-manager: [swaync] client failed: {e}"),
        }
    }
}
