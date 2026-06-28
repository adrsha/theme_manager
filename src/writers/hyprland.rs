use crate::{Theme, ThemeError, Writer, dots};
use std::path::PathBuf;

pub struct Hyprland;

impl Writer for Hyprland {
    fn name(&self) -> &'static str { "hyprland" }

    fn output_path(&self) -> PathBuf { dots().join("hypr/colors.conf") }

    fn generate(&self, theme: &Theme) -> Result<String, ThemeError> {
        let border = theme.require("BASE02")?.trim_start_matches('#');
        let bg     = theme.require("DARKER_BLACK")?.trim_start_matches('#');

        Ok(format!(
            "general {{\n\
             \tcol.active_border   = rgb({border})\n\
             \tcol.inactive_border = rgb({border})\n\
             }}\n\
             misc {{\n\
             \tbackground_color = rgb({bg})\n\
             }}\n"
        ))
    }
}
