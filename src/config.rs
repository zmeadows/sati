use crate::storage::Result;
use ratatui::style::{Color, Modifier};
use serde::{Deserialize, Deserializer, de::Error};
use std::{
    env,
    ffi::OsString,
    fs, io,
    ops::Range,
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

// The configuration file sati reads and watches. Only an explicit `--config` file is
// required to exist; a missing default file means built-in defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigFile {
    pub path: PathBuf,
    pub required: bool,
}

// The raw state of the file at one moment. Checks compare it with the previous state so
// identical contents are never parsed or applied twice.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigText {
    Missing,
    Text(String),
    Unreadable(String),
}

// Why a configuration file could not be used, kept structured so the error overlay can
// show the offending source line instead of one flattened string.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigError {
    pub path: PathBuf,
    pub message: String,
    pub location: Option<SourceLocation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceLocation {
    // 1-based, as editors number lines.
    pub line_number: usize,
    // The source line without its line ending.
    pub line: String,
    // The offending bytes within `line`, on character boundaries; may be empty at the end.
    pub columns: Range<usize>,
}

impl ConfigFile {
    // Only an unusable environment is an error here: no file change could fix it.
    pub fn locate(explicit: Option<&Path>) -> Result<Option<Self>> {
        if let Some(path) = explicit {
            // Watch events report absolute paths, and the working directory never changes.
            return Ok(Some(Self {
                path: std::path::absolute(path)?,
                required: true,
            }));
        }

        let path = default_path(|name| env::var_os(name))?;

        Ok(path.map(|path| Self {
            path,
            required: false,
        }))
    }

    pub fn read(&self) -> ConfigText {
        match fs::read_to_string(&self.path) {
            Ok(text) => ConfigText::Text(text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => ConfigText::Missing,
            Err(error) => ConfigText::Unreadable(error.to_string()),
        }
    }
}

impl Config {
    pub fn parse(file: &ConfigFile, text: &ConfigText) -> std::result::Result<Self, ConfigError> {
        let error = |message: String, location| ConfigError {
            path: file.path.clone(),
            message,
            location,
        };

        let text = match text {
            ConfigText::Text(text) => text,
            ConfigText::Missing if !file.required => return Ok(Self::default()),
            ConfigText::Missing => {
                return Err(error(
                    "The file does not exist. sati applies it as soon as it is created.".into(),
                    None,
                ));
            }
            ConfigText::Unreadable(reason) => {
                return Err(error(format!("Could not read the file: {reason}"), None));
            }
        };

        toml::from_str(text).map_err(|parse_error| {
            let location = parse_error
                .span()
                .and_then(|span| source_location(text, span));

            error(parse_error.message().trim_end().to_string(), location)
        })
    }
}

fn source_location(text: &str, span: Range<usize>) -> Option<SourceLocation> {
    if span.start > text.len() || !text.is_char_boundary(span.start) {
        return None;
    }

    let line_start = text[..span.start].rfind('\n').map_or(0, |index| index + 1);
    let line_end = text[span.start..]
        .find('\n')
        .map_or(text.len(), |index| span.start + index);
    let line = text[line_start..line_end].trim_end_matches('\r');

    // Clamp the span to its first line; a multi-line value is marked from its start.
    let start = (span.start - line_start).min(line.len());
    let end = span.end.saturating_sub(line_start).clamp(start, line.len());

    if !line.is_char_boundary(end) {
        return None;
    }

    Some(SourceLocation {
        line_number: text[..line_start].matches('\n').count() + 1,
        line: line.to_string(),
        columns: start..end,
    })
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

    fn file(fixture: &crate::storage::tests::Fixture, required: bool) -> ConfigFile {
        ConfigFile {
            path: fixture.directory.join("config.toml"),
            required,
        }
    }

    fn parse_error(text: &str) -> ConfigError {
        let fixture = crate::storage::tests::Fixture::new();

        Config::parse(&file(&fixture, false), &ConfigText::Text(text.into())).unwrap_err()
    }

    #[test]
    fn reading_tells_missing_readable_and_unreadable_files_apart() {
        let fixture = crate::storage::tests::Fixture::new();
        let file = file(&fixture, false);

        assert_eq!(file.read(), ConfigText::Missing);

        fs::write(&file.path, "theme = 'nord'").unwrap();
        assert_eq!(file.read(), ConfigText::Text("theme = 'nord'".into()));

        fs::remove_file(&file.path).unwrap();
        fs::create_dir(&file.path).unwrap();
        assert!(matches!(file.read(), ConfigText::Unreadable(_)));
    }

    #[test]
    fn missing_default_files_use_defaults_but_missing_explicit_files_are_errors() {
        let fixture = crate::storage::tests::Fixture::new();

        let config = Config::parse(&file(&fixture, false), &ConfigText::Missing).unwrap();
        assert_eq!(config.display, Display::default());

        let error = Config::parse(&file(&fixture, true), &ConfigText::Missing).unwrap_err();
        assert_eq!(error.path, fixture.directory.join("config.toml"));
        assert!(
            error.message.contains("does not exist"),
            "{}",
            error.message
        );
        assert_eq!(error.location, None);
    }

    #[test]
    fn unreadable_files_are_errors_without_a_source_line() {
        let fixture = crate::storage::tests::Fixture::new();
        let text = ConfigText::Unreadable("permission denied".into());

        let error = Config::parse(&file(&fixture, false), &text).unwrap_err();

        assert!(error.message.contains("permission denied"));
        assert_eq!(error.location, None);
    }

    #[test]
    fn invalid_values_point_at_their_line_and_columns() {
        let error = parse_error("[display]\nwrap = 'x'\nkeybinds = true\n");

        assert!(
            error.message.contains("expected a boolean"),
            "{}",
            error.message
        );
        assert_eq!(
            error.location,
            Some(SourceLocation {
                line_number: 2,
                line: "wrap = 'x'".into(),
                columns: 7..10,
            })
        );
    }

    #[test]
    fn malformed_toml_and_unknown_keys_report_their_position() {
        let location = parse_error("bad toml").location.unwrap();
        assert_eq!((location.line_number, location.columns), (1, 4..4));

        let error = parse_error("[display]\r\nwrpa = true\r\n");

        assert!(error.message.contains("unknown field `wrpa`"));
        assert_eq!(
            error.location,
            Some(SourceLocation {
                line_number: 2,
                line: "wrpa = true".into(),
                columns: 0..4,
            })
        );
    }

    #[test]
    fn unknown_themes_keep_the_list_of_available_themes() {
        let error = parse_error("theme='unknown'");

        assert!(error.message.contains("Available themes: default"));
        assert_eq!(error.location.unwrap().columns, 6..15);
    }

    #[test]
    fn spans_are_clamped_to_their_first_line_and_character_boundaries() {
        let text = "a = 1\nb = '''\nfirst\nsecond'''\n";

        let location = source_location(text, 10..30).unwrap();

        assert_eq!(location.line_number, 2);
        assert_eq!(location.line, "b = '''");
        assert_eq!(location.columns, 4..7);
        assert!(source_location(text, 99..100).is_none());
        assert!(source_location("é", 1..2).is_none());
    }
}
