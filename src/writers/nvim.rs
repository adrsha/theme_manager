use crate::{Theme, ThemeError, Writer};
use std::path::PathBuf;

pub struct NvimPatch;

impl Writer for NvimPatch {
    fn name(&self) -> &'static str { "nvim-patch" }

    fn output_path(&self) -> PathBuf { PathBuf::from("/tmp/nvim_theme_patch.lua") }

    fn generate(&self, theme: &Theme) -> Result<String, ThemeError> {
        let mut out = format!("vim.env.THEME_NAME = \"{}\"\n", theme.name);
        for (key, val) in &theme.vars {
            out.push_str(&format!("vim.env[\"THEME_{key}\"] = \"{val}\"\n"));
        }
        Ok(out)
    }
}
