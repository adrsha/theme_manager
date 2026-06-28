use crate::{Theme, ThemeError, Writer, dots};
use std::path::PathBuf;

pub struct Ghostty;

impl Writer for Ghostty {
    fn name(&self) -> &'static str { "ghostty" }

    fn output_path(&self) -> PathBuf { dots().join("ghostty/themes/active") }

    fn generate(&self, theme: &Theme) -> Result<String, ThemeError> {
        let t = theme;
        Ok(format!(
            "palette = 0={}\n\
             palette = 1={}\n\
             palette = 2={}\n\
             palette = 3={}\n\
             palette = 4={}\n\
             palette = 5={}\n\
             palette = 6={}\n\
             palette = 7={}\n\
             palette = 8={}\n\
             palette = 9={}\n\
             palette = 10={}\n\
             palette = 11={}\n\
             palette = 12={}\n\
             palette = 13={}\n\
             palette = 14={}\n\
             palette = 15={}\n\
             background           = {}\n\
             foreground           = {}\n\
             cursor-color         = {}\n\
             selection-background = {}\n\
             selection-foreground = {}\n",
            t.require("BASE00")?,
            t.require("BASE08")?,
            t.require("BASE0B")?,
            t.require("BASE0A")?,
            t.require("BASE0D")?,
            t.require("BASE0E")?,
            t.require("BASE0C")?,
            t.require("BASE05")?,
            t.require("BASE01")?,
            t.require("BASE08")?,
            t.require("BASE0B")?,
            t.require("BASE0A")?,
            t.require("BASE0D")?,
            t.require("BASE0E")?,
            t.require("BASE0C")?,
            t.require("BASE07")?,
            t.require("BASE00")?,
            t.require("BASE05")?,
            t.require("BASE05")?,
            t.require("BASE02")?,
            t.require("BASE05")?,
        ))
    }
}
