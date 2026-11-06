use std::str::FromStr;

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

use crate::styles::{DEFAULT_ACCENT, Styles};

#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(try_from = "String")]
pub enum BuiltinTheme {
    #[default]
    Default,
}

impl FromStr for BuiltinTheme {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "default" => Ok(Self::Default),
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
    pub const NAMES: &'static str = "default";
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
