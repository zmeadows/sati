use crate::storage::Result;
use serde::Deserialize;
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
    pub display: Display,
}

#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Display {
    pub wrap: Option<bool>,
    pub logo: Option<bool>,
    pub keybinds: Option<bool>,
    pub rounded_corners: Option<bool>,
    pub ascii_markers: Option<bool>,
    pub completed_at_bottom: Option<bool>,
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
    fn partial_and_strict_schema() {
        let config: Config = toml::from_str("[display]\nwrap = false").unwrap();
        assert_eq!(config.display.wrap, Some(false));
        assert_eq!(config.display.logo, None);
        assert_eq!(
            toml::from_str::<Config>("").unwrap().display,
            Display::default()
        );
        for text in [
            "bad toml",
            "unknown = true",
            "[unknown]",
            "[styles.accent]\nfg='red'",
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
