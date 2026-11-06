use std::str::FromStr;

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

use crate::styles::{DEFAULT_ACCENT, Styles};

#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(try_from = "String")]
pub enum BuiltinTheme {
    #[default]
    Default,
    GruvboxMaterial,
}

impl FromStr for BuiltinTheme {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "default" => Ok(Self::Default),
            "gruvbox-material" => Ok(Self::GruvboxMaterial),
            _ => Err(format!(
                "unknown theme: {name:?}. Available themes: {}",
                Self::NAMES
            )),
        }
    }
}

impl TryFrom<String> for BuiltinTheme {
    type Error = String;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        name.parse()
    }
}

impl BuiltinTheme {
    pub const NAMES: &'static str = "default, gruvbox-material";
    // Role defaults patch their resolved parent, allowing TOML overrides on that
    // parent to cascade before the child's theme defaults and overrides apply.
    pub fn base_styles(self) -> Styles {
        match self {
            Self::Default => Styles {
                normal: Style::default()
                    .fg(Color::Reset)
                    .remove_modifier(Modifier::all()),
                muted: Style::default().add_modifier(Modifier::DIM),
                completed: Style::default(),
                border: Style::default(),
                accent: Style::default().fg(DEFAULT_ACCENT),
            },
            Self::GruvboxMaterial => {
                // Each palette role explicitly clears inherited modifiers.
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(212, 190, 152)),
                    muted: plain.fg(Color::Rgb(168, 153, 132)),
                    completed: plain.fg(Color::Rgb(146, 131, 116)),
                    border: plain.fg(Color::Rgb(90, 82, 76)),
                    accent: plain.fg(Color::Rgb(231, 138, 78)),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gruvbox_material_has_exact_palette_and_clears_inherited_modifiers() {
        let config: crate::config::Config = toml::from_str("theme='gruvbox-material'").unwrap();
        assert_eq!(config.theme, BuiltinTheme::GruvboxMaterial);
        assert_eq!(
            "gruvbox-material".parse::<BuiltinTheme>().unwrap(),
            config.theme
        );
        assert!("Gruvbox-Material".parse::<BuiltinTheme>().is_err());
        let styles = Styles::resolve(config.theme, &config.styles, None);
        for (role, color) in [
            (styles.normal, Color::Rgb(212, 190, 152)),
            (styles.muted, Color::Rgb(168, 153, 132)),
            (styles.completed, Color::Rgb(146, 131, 116)),
            (styles.border, Color::Rgb(90, 82, 76)),
            (styles.accent, Color::Rgb(231, 138, 78)),
        ] {
            assert_eq!(role.fg, Some(color));
            assert_eq!(role.bg, None);
            assert!(role.add_modifier.is_empty());
            assert_eq!(role.sub_modifier, Modifier::all());
        }
        let inherited: crate::config::Config =
            toml::from_str("theme='gruvbox-material'\n[styles.normal]\nmodifiers=['bold']")
                .unwrap();
        let inherited = Styles::resolve(inherited.theme, &inherited.styles, None);
        assert_eq!(inherited.normal.add_modifier, Modifier::BOLD);
        for role in [
            inherited.muted,
            inherited.completed,
            inherited.border,
            inherited.accent,
        ] {
            assert!(role.add_modifier.is_empty());
        }
        let config: crate::config::Config = toml::from_str("theme='gruvbox-material'\n[styles.normal]\nmodifiers=['bold']\n[styles.muted]\nfg='green'\nmodifiers=['italic']\n[styles.completed]\nmodifiers=['dim']\n[styles.border]\nmodifiers=[]\n[styles.accent]\nfg='blue'\nmodifiers=['bold']").unwrap();
        let styles = Styles::resolve(config.theme, &config.styles, Some(Color::Red));
        assert_eq!(styles.normal.add_modifier, Modifier::BOLD);
        assert_eq!(styles.muted.fg, Some(Color::Green));
        assert_eq!(styles.muted.add_modifier, Modifier::ITALIC);
        assert_eq!(styles.completed.add_modifier, Modifier::DIM);
        assert!(styles.border.add_modifier.is_empty());
        assert_eq!(styles.accent.fg, Some(Color::Red));
        assert_eq!(styles.accent.add_modifier, Modifier::BOLD);
    }

    #[test]
    fn names_are_strict_and_default_preserves_the_existing_palette() {
        assert_eq!(
            "default".parse::<BuiltinTheme>().unwrap(),
            BuiltinTheme::Default
        );
        for name in ["", "Default", "DEFAULT", "gruvbox", " default"] {
            let error = name.parse::<BuiltinTheme>().unwrap_err();
            assert!(error.contains("Available themes: default"));
        }
        let omitted: crate::config::Config = toml::from_str("").unwrap();
        let explicit: crate::config::Config = toml::from_str("theme='default'").unwrap();
        assert_eq!(omitted.theme, explicit.theme);
        let actual = Styles::resolve(explicit.theme, &explicit.styles, None);
        let normal = Style::default()
            .fg(Color::Reset)
            .remove_modifier(Modifier::all());
        let muted = normal.add_modifier(Modifier::DIM);
        assert_eq!(
            actual,
            Styles {
                normal,
                muted,
                completed: muted,
                border: muted,
                accent: normal.fg(Color::LightYellow)
            }
        );
        for text in ["theme='Default'", "theme='unknown'", "theme=1", "theme=[]"] {
            assert!(toml::from_str::<crate::config::Config>(text).is_err());
        }
    }
}
