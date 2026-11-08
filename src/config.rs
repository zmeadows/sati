use crate::storage::Result;
use ratatui::style::{Color, Modifier};
use serde::{Deserialize, Deserializer, de::Error};
use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub theme: crate::themes::BuiltinTheme,
    #[serde(default)]
    pub display: Display,
    #[serde(default)]
    pub styles: StyleOverrides,
}

#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Display {
    pub wrap: Option<bool>,
    pub keybinds: Option<bool>,
    pub rounded_corners: Option<bool>,
    pub ascii_markers: Option<bool>,
    pub notifications: Option<bool>,
}

#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct StyleOverrides {
    pub normal: StyleOverride,
    pub muted: StyleOverride,
    pub completed: StyleOverride,
    pub border: StyleOverride,
    #[serde(deserialize_with = "accent_style")]
    pub accent: StyleOverride,
}

#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleOverride {
    #[serde(default, deserialize_with = "color")]
    pub fg: Option<Color>,
    #[serde(default, deserialize_with = "background")]
    pub bg: Option<Color>,
    #[serde(default, deserialize_with = "modifiers")]
    pub modifiers: Option<Modifier>,
}

fn color<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Color>, D::Error> {
    let text = String::deserialize(deserializer)?;
    text.parse()
        .map(Some)
        .map_err(|_| D::Error::custom(format!("invalid color: {text}")))
}

fn background<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Color>, D::Error> {
    let color = color(deserializer)?;

    match color {
        Some(Color::Rgb(..) | Color::Reset) => Ok(color),
        _ => Err(D::Error::custom(
            "background must be #RRGGBB or reset (OSC 11 requires an exact RGB color)",
        )),
    }
}

fn modifiers<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Modifier>, D::Error> {
    let names = Vec::<String>::deserialize(deserializer)?;
    let mut modifiers = Modifier::empty();

    for name in names {
        let modifier = match name.as_str() {
            "bold" => Modifier::BOLD,
            "dim" => Modifier::DIM,
            "italic" => Modifier::ITALIC,
            "underlined" => Modifier::UNDERLINED,
            "crossed_out" => Modifier::CROSSED_OUT,
            _ => return Err(D::Error::custom(format!("unknown modifier: {name}"))),
        };

        if modifiers.contains(modifier) {
            return Err(D::Error::custom(format!("duplicate modifier: {name}")));
        }

        modifiers.insert(modifier);
    }

    Ok(Some(modifiers))
}

fn accent_style<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<StyleOverride, D::Error> {
    let style = StyleOverride::deserialize(deserializer)?;

    if style.fg == Some(Color::Reset) {
        return Err(D::Error::custom("styles.accent.fg must not be reset"));
    }

    Ok(style)
}

impl Config {
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        let discovered;

        let path = match explicit {
            Some(path) => path,
            None => {
                discovered = default_path(|name| env::var_os(name))?;

                let Some(path) = discovered.as_deref() else {
                    return Ok(Self::default());
                };

                path
            }
        };

        // Only the default file is optional; an explicit `--config` file must exist.
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if explicit.is_none() && error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(
                    format!("could not read configuration {}: {error}", path.display()).into(),
                );
            }
        };

        toml::from_str(&text)
            .map_err(|error| format!("invalid configuration {}: {error}", path.display()).into())
    }
}

fn default_path(get: impl Fn(&str) -> Option<OsString>) -> Result<Option<PathBuf>> {
    let value = |name| get(name).filter(|value| !value.is_empty());

    #[cfg(not(any(target_os = "macos", windows)))]
    let base = if let Some(path) = value("XDG_CONFIG_HOME") {
        let path = PathBuf::from(path);

        if !path.is_absolute() {
            return Err("XDG_CONFIG_HOME must be an absolute path".into());
        }

        Some(path)
    } else {
        value("HOME").map(|home| PathBuf::from(home).join(".config"))
    };

    #[cfg(target_os = "macos")]
    let base = value("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(windows)]
    let base = value("APPDATA").map(PathBuf::from);

    // No discoverable native location means no optional default configuration.
    base.map(|base| {
        if !base.is_absolute() {
            return Err("configuration directory must be an absolute path".into());
        }
        Ok(base.join("sati/config.toml"))
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backgrounds_accept_exact_rgb_or_reset_on_every_role() {
        for role in ["normal", "muted", "completed", "border", "accent"] {
            for value in ["'#123456'", "'reset'"] {
                assert!(toml::from_str::<Config>(&format!("[styles.{role}]\nbg={value}")).is_ok());
            }

            for value in ["'red'", "'3'", "'#123'", "12", "[]"] {
                assert!(toml::from_str::<Config>(&format!("[styles.{role}]\nbg={value}")).is_err());
            }
        }
    }

    #[test]
    fn style_fields_are_strict_and_colors_use_ratatui_parsing() {
        for role in ["normal", "muted", "completed", "border", "accent"] {
            for value in ["yellow", "lightyellow", "darkgray", "3", "#e78a4e"] {
                assert!(
                    toml::from_str::<Config>(&format!("[styles.{role}]\nfg='{value}'")).is_ok()
                );
            }

            for property in [
                "fg=12",
                "fg='orange'",
                "modifiers='dim'",
                "modifiers=[1]",
                "modifiers=['unknown']",
                "modifiers=['bold','bold']",
                "bg='blue'",
                "unknown=true",
            ] {
                let text = format!("[styles.{role}]\n{property}");
                assert!(toml::from_str::<Config>(&text).is_err(), "accepted {text}");
            }
        }

        assert!(toml::from_str::<Config>("[styles.accent]\nfg='reset'").is_err());
        assert!(toml::from_str::<Config>("[styles.normal]\nfg='reset'").is_ok());
        assert!(toml::from_str::<Config>("[styles.unknown]").is_err());
    }

    #[test]
    fn partial_and_strict_schema() {
        let config: Config = toml::from_str("[display]\nwrap = false").unwrap();
        assert_eq!(config.display.wrap, Some(false));
        assert_eq!(config.display.keybinds, None);
        assert_eq!(
            toml::from_str::<Config>("").unwrap().display,
            Display::default()
        );

        for text in [
            "bad toml",
            "unknown = true",
            "[unknown]",
            "[styles.unknown]",
            "[display]\nwrpa = true",
            "[display]\nwrap = 'false'",
            "[display]\nwrap = 0",
        ] {
            assert!(toml::from_str::<Config>(text).is_err(), "accepted {text}");
        }
    }

    #[test]
    fn native_discovery_is_optional_absolute_and_never_creates_directories() {
        let root = std::env::temp_dir();
        let path = default_path(|_| Some(root.clone().into_os_string()))
            .unwrap()
            .unwrap();

        let expected = if cfg!(target_os = "macos") {
            root.join("Library/Application Support/sati/config.toml")
        } else {
            root.join("sati/config.toml")
        };

        assert_eq!(path, expected);
        assert!(default_path(|_| None).unwrap().is_none());
        assert!(default_path(|_| Some("relative".into())).is_err());

        #[cfg(not(any(target_os = "macos", windows)))]
        {
            assert_eq!(
                default_path(|name| if name == "HOME" {
                    Some(root.clone().into())
                } else {
                    Some("".into())
                })
                .unwrap(),
                Some(root.join(".config/sati/config.toml"))
            );
        }
    }
}
