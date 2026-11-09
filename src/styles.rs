use ratatui::style::{Color, Modifier, Style};

use crate::config::{StyleOverride, StyleOverrides};
use crate::themes::BuiltinTheme;

pub const DEFAULT_ACCENT: Color = Color::LightYellow;

/// The selected theme, the inputs layered over it, and the styles they resolve to.
///
/// The inputs are private so `styles` is always `Styles::resolve` of the current theme,
/// configuration overrides, and CLI accent; only the theme changes after startup.
#[derive(Debug, PartialEq)]
pub struct Appearance {
    theme: BuiltinTheme,
    overrides: StyleOverrides,
    cli_accent: Option<Color>,
    styles: Styles,
}

impl Appearance {
    pub fn new(theme: BuiltinTheme, overrides: StyleOverrides, cli_accent: Option<Color>) -> Self {
        Self {
            styles: Styles::resolve(theme, &overrides, cli_accent),
            theme,
            overrides,
            cli_accent,
        }
    }

    pub fn theme(&self) -> BuiltinTheme {
        self.theme
    }

    pub fn styles(&self) -> &Styles {
        &self.styles
    }

    pub fn set_theme(&mut self, theme: BuiltinTheme) {
        self.theme = theme;
        self.styles = Styles::resolve(theme, &self.overrides, self.cli_accent);
    }

    /// Lets rendering tests adjust individual resolved styles without building a configuration.
    #[cfg(test)]
    pub fn styles_mut(&mut self) -> &mut Styles {
        &mut self.styles
    }
}

impl Default for Appearance {
    fn default() -> Self {
        Self::new(BuiltinTheme::default(), StyleOverrides::default(), None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Styles {
    pub normal: Style,
    pub muted: Style,
    pub completed: Style,
    pub border: Style,
    pub accent: Style,
    pub heading: Style,
    pub carried: Style,
}

impl Styles {
    pub fn resolve(
        theme: BuiltinTheme,
        overrides: &StyleOverrides,
        cli_accent: Option<Color>,
    ) -> Self {
        let bases = theme.base_styles();

        let apply = |base: Style, value: &StyleOverride| {
            let style = if let Some(fg) = value.fg {
                base.fg(fg)
            } else {
                base
            };

            let style = if let Some(bg) = value.bg {
                style.bg(bg)
            } else {
                style
            };

            if let Some(modifiers) = value.modifiers {
                style
                    .remove_modifier(Modifier::all())
                    .add_modifier(modifiers)
            } else {
                style
            }
        };

        let normal = apply(bases.normal, &overrides.normal);
        let muted = apply(normal.patch(bases.muted), &overrides.muted);
        let completed = apply(muted.patch(bases.completed), &overrides.completed);
        let border = apply(muted.patch(bases.border), &overrides.border);
        let mut accent = apply(normal.patch(bases.accent), &overrides.accent);
        let heading = apply(normal.patch(bases.heading), &overrides.heading);
        let carried = apply(normal.patch(bases.carried), &overrides.carried);

        if let Some(fg) = cli_accent {
            accent = accent.fg(fg);
        }

        Self {
            normal,
            muted,
            completed,
            border,
            accent,
            heading,
            carried,
        }
    }
}

impl Default for Styles {
    fn default() -> Self {
        Self::resolve(BuiltinTheme::Default, &StyleOverrides::default(), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(text: &str) -> Styles {
        let config: crate::config::Config = toml::from_str(text).unwrap();
        Styles::resolve(config.theme, &config.styles, None)
    }

    #[test]
    fn cascade_inherits_foregrounds_and_replaces_modifier_sets() {
        assert_eq!(resolve(""), Styles::default());

        let styles = resolve("[styles.normal]\nfg='lightblue'\nmodifiers=['bold']");
        assert_eq!(styles.normal.fg, Some(Color::LightBlue));
        assert_eq!(styles.muted.fg, Some(Color::LightBlue));
        assert_eq!(styles.muted.add_modifier, Modifier::BOLD | Modifier::DIM);
        assert_eq!(styles.completed, styles.muted);
        assert_eq!(styles.border, styles.muted);
        assert_eq!(styles.accent.fg, Some(DEFAULT_ACCENT));
        assert_eq!(styles.accent.add_modifier, Modifier::BOLD);
        assert_eq!(styles.heading, styles.normal.fg(Color::LightCyan));
        assert_eq!(styles.carried, styles.normal.fg(Color::Yellow));

        for (text, modifiers) in [("['italic']", Modifier::ITALIC), ("[]", Modifier::empty())] {
            let styles = resolve(&format!(
                "[styles.normal]\nfg='lightblue'\nmodifiers=['bold']\n[styles.muted]\nfg='green'\nmodifiers={text}"
            ));

            assert_eq!(styles.muted.fg, Some(Color::Green));
            assert_eq!(styles.muted.add_modifier, modifiers);
            assert_eq!(styles.completed, styles.muted);
            assert_eq!(styles.border, styles.muted);
            assert_eq!(styles.normal.fg, Some(Color::LightBlue));
            assert_eq!(styles.normal.patch(styles.muted).add_modifier, modifiers);
        }

        let styles = resolve(
            "[styles.completed]\nfg='red'\nmodifiers=[]\n[styles.border]\nfg='3'\nmodifiers=['underlined','crossed_out']",
        );

        assert_eq!(styles.completed.fg, Some(Color::Red));
        assert!(styles.completed.add_modifier.is_empty());
        assert_eq!(
            styles.muted.patch(styles.completed).add_modifier,
            Modifier::empty()
        );
        assert_eq!(styles.border.fg, Some(Color::Indexed(3)));
        assert_eq!(
            styles.border.add_modifier,
            Modifier::UNDERLINED | Modifier::CROSSED_OUT
        );
    }

    #[test]
    fn defaults_resolve_semantic_inheritance_without_contextual_modifiers() {
        let styles = Styles::default();
        assert_eq!(styles.normal.fg, Some(Color::Reset));
        assert_eq!(styles.muted, styles.normal.add_modifier(Modifier::DIM));
        assert_eq!(styles.completed, styles.muted);
        assert_eq!(styles.border, styles.muted);
        assert_eq!(styles.accent, styles.normal.fg(Color::LightYellow));
        assert_eq!(styles.heading, styles.normal.fg(Color::LightCyan));
        assert_eq!(styles.carried, styles.normal.fg(Color::Yellow));
        assert!(
            !styles
                .completed
                .add_modifier
                .contains(Modifier::CROSSED_OUT)
        );
        assert!(
            !styles
                .muted
                .patch(styles.normal)
                .add_modifier
                .contains(Modifier::DIM)
        );
        assert!(
            !styles
                .muted
                .patch(styles.accent)
                .add_modifier
                .contains(Modifier::DIM)
        );

        let accent = styles.accent.add_modifier(Modifier::ITALIC);
        let overridden = accent.fg(Color::Indexed(3));
        assert_eq!(overridden.add_modifier, accent.add_modifier);
        assert_eq!(overridden.fg, Some(Color::Indexed(3)));
    }

    #[test]
    fn heading_and_carried_patch_normal_independently_of_accent_and_completed() {
        let styles = resolve(
            "[styles.normal]\nbg='#102030'\nmodifiers=['italic']\n[styles.muted]\nfg='green'\n[styles.completed]\nfg='blue'\nmodifiers=['dim']\n[styles.accent]\nfg='red'\nmodifiers=['bold']",
        );

        assert_eq!(styles.heading, styles.normal.fg(Color::LightCyan));
        assert_eq!(styles.carried, styles.normal.fg(Color::Yellow));
        assert_eq!(styles.heading.bg, Some(Color::Rgb(16, 32, 48)));
        assert_eq!(styles.carried.add_modifier, Modifier::ITALIC);

        let styles = resolve(
            "[styles.heading]\nfg='#a9b665'\nbg='#000000'\nmodifiers=[]\n[styles.carried]\nfg='reset'\nmodifiers=['underlined']",
        );

        assert_eq!(styles.heading.fg, Some(Color::Rgb(169, 182, 101)));
        assert_eq!(styles.heading.bg, Some(Color::Rgb(0, 0, 0)));
        assert!(styles.heading.add_modifier.is_empty());
        assert_eq!(styles.carried.fg, Some(Color::Reset));
        assert_eq!(styles.carried.add_modifier, Modifier::UNDERLINED);

        let unchanged = Styles::default();
        assert_eq!(
            (styles.normal, styles.muted, styles.accent),
            (unchanged.normal, unchanged.muted, unchanged.accent)
        );

        // The CLI accent replaces only the accent foreground.
        for theme in ["default", "gruvbox-material"] {
            let config: crate::config::Config =
                toml::from_str(&format!("theme='{theme}'")).unwrap();
            let plain = Styles::resolve(config.theme, &config.styles, None);
            let accented = Styles::resolve(config.theme, &config.styles, Some(Color::Blue));

            assert_eq!(
                accented,
                Styles {
                    accent: plain.accent.fg(Color::Blue),
                    ..plain
                }
            );
        }
    }
}
