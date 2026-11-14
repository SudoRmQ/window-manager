use crate::{
    command::{self, Command},
    tree::{Layout, TreeOptions},
};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    /// Win key on Windows, Cmd on macOS.
    pub super_: bool,
}

/// A parsed hotkey like `alt+shift+j`. The key is lowercased; backends map it to a
/// virtual-key code (Windows) or kVK code (macOS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    pub mods: Modifiers,
    pub key: String,
}

impl Hotkey {
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut mods = Modifiers::default();
        let parts: Vec<String> = s.split('+').map(|p| p.trim().to_lowercase()).collect();
        let (key, mod_parts) = parts.split_last().ok_or("empty hotkey")?;
        for m in mod_parts {
            match m.as_str() {
                "alt" | "opt" | "option" | "mod1" => mods.alt = true,
                "ctrl" | "control" => mods.ctrl = true,
                "shift" => mods.shift = true,
                "super" | "win" | "cmd" | "command" | "mod4" => mods.super_ = true,
                other => return Err(format!("unknown modifier `{other}` in `{s}`")),
            }
        }
        if key.is_empty() {
            return Err(format!("missing key in `{s}`"));
        }
        Ok(Self {
            mods,
            key: key.clone(),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawConfig {
    gap: i32,
    workspace_layout: Layout,
    focus_wrapping: bool,
    workspace_auto_back_and_forth: bool,
    float_rules: Vec<String>,
    keybindings: BTreeMap<String, String>,
}

impl Default for RawConfig {
    fn default() -> Self {
        Self {
            gap: 8,
            workspace_layout: Layout::Splith,
            focus_wrapping: true,
            workspace_auto_back_and_forth: false,
            float_rules: vec![],
            keybindings: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub gap: i32,
    pub tree: TreeOptions,
    pub float_rules: Vec<String>,
    pub bindings: Vec<(Hotkey, Vec<Command>)>,
}

impl Config {
    pub fn from_toml(src: &str) -> Result<Self, String> {
        let raw: RawConfig = toml::from_str(src).map_err(|e| e.to_string())?;
        let bindings = raw
            .keybindings
            .iter()
            .map(|(k, c)| {
                let cmds = command::parse(c).map_err(|e| format!("binding `{k}`: {e}"))?;
                Ok((Hotkey::parse(k)?, cmds))
            })
            .collect::<Result<_, String>>()?;
        Ok(Self {
            gap: raw.gap.max(0),
            tree: TreeOptions {
                default_layout: raw.workspace_layout,
                focus_wrapping: raw.focus_wrapping,
                auto_back_and_forth: raw.workspace_auto_back_and_forth,
            },
            float_rules: raw.float_rules,
            bindings,
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::from_toml(include_str!("../../../config.example.toml"))
            .expect("bundled config is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_example_parses() {
        let c = Config::default();
        assert!(c.bindings.len() > 20);
    }

    #[test]
    fn hotkey_parse() {
        let h = Hotkey::parse("Alt+Shift+J").unwrap();
        assert!(h.mods.alt && h.mods.shift && !h.mods.ctrl);
        assert_eq!(h.key, "j");
        assert!(Hotkey::parse("hyper+j").is_err());
    }

    #[test]
    fn rejects_bad_command() {
        let e = Config::from_toml("[keybindings]\n\"alt+j\" = \"fly\"").unwrap_err();
        assert!(e.contains("fly"));
    }
}
