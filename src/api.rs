//! App-neutral, versioned theme snapshot. Clients can read once or watch the
//! parent directory for MOVED_TO (atomic replacement) to receive updates.
use crate::{get_transparency, home, Theme, ThemeError};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

pub fn snapshot_path() -> PathBuf {
    home().join(".local/state/theme/active.json")
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c <= '\u{1f}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Stable JSON API: `version`, `name`, `variant`, `amoled`, `opacity`,
/// `colors` (all resolved color keys) and `values` (all resolved theme keys).
/// Unknown keys can be ignored by clients; keys are sorted for reproducibility.
pub fn snapshot(theme: &Theme) -> Result<String, ThemeError> {
    let variant = theme.require("VARIANT")?;
    if variant != "dark" && variant != "light" {
        return Err(ThemeError::InvalidVariant(variant.to_string()));
    }
    let values = theme.vars.iter().filter(|(key, _)| {
        key.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    });
    let mut values: Vec<_> = values.collect();
    values.sort_by_key(|(key, _)| *key);
    let pairs = |only_colors: bool| -> String {
        values
            .iter()
            .filter(|(_, value)| {
                !only_colors
                    || (value.len() == 7
                        && value.starts_with('#')
                        && value[1..].bytes().all(|b| b.is_ascii_hexdigit()))
            })
            .map(|(key, value)| format!("{}:{}", json_string(key), json_string(value)))
            .collect::<Vec<_>>()
            .join(",")
    };
    Ok(format!(
        "{{\"version\":1,\"name\":{},\"variant\":{},\"amoled\":{},\"opacity\":{},\"colors\":{{{}}},\"values\":{{{}}}}}\n",
        json_string(&theme.name), json_string(variant), variant == "dark" &&
            theme.vars.get("AMOLED").map(String::as_str) == Some("true"),
        get_transparency(theme), pairs(true), pairs(false)
    ))
}

/// Publish complete snapshot via rename. Readers never see partial JSON.
pub fn publish(theme: &Theme) -> Result<(), ThemeError> {
    let content = snapshot(theme)?;
    let path = snapshot_path();
    let parent = path.parent().expect("snapshot has parent");
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".active-{}.json.tmp", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(ThemeError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    #[test]
    fn snapshot_is_sorted_escaped_and_resolved() {
        let theme = Theme {
            name: "odd\"name".into(),
            path: "/tmp/x".into(),
            vars: HashMap::from([
                ("VARIANT".into(), "dark".into()),
                ("AMOLED".into(), "true".into()),
                ("BASE05".into(), "#abcdef".into()),
                ("BASE00".into(), "#123456".into()),
                ("WALLPAPER".into(), "a\nb".into()),
            ]),
        };
        let json = snapshot(&theme).unwrap();
        assert!(json.contains("\"name\":\"odd\\\"name\""));
        assert!(json.contains("\"amoled\":true,\"opacity\":1"));
        assert!(json.contains("\"WALLPAPER\":\"a\\nb\""));
        assert!(json.find("BASE00").unwrap() < json.find("BASE05").unwrap());
        assert!(json.contains("\"colors\":{\"BASE00\":\"#123456\",\"BASE05\":\"#abcdef\"}"));
    }
}
