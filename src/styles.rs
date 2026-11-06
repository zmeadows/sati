use ratatui::style::{Color, Modifier, Style};

pub const DEFAULT_ACCENT: Color = Color::LightYellow;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Styles {
    pub normal: Style,
    pub muted: Style,
    pub completed: Style,
    pub border: Style,
    pub accent: Style,
}

impl Styles {
    pub fn resolve(
        theme: crate::themes::BuiltinTheme,
        overrides: &crate::config::StyleOverrides,
        cli_accent: Option<Color>,
    ) -> Self {
        let bases = theme.base_styles();
        let apply = |base: Style, value: &crate::config::StyleOverride| {
            let style = if let Some(fg) = value.fg {
                base.fg(fg)
            } else {
                base
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
        if let Some(fg) = cli_accent {
            accent = accent.fg(fg);
        }
        Self {
            normal,
            muted,
            completed,
            border,
            accent,
        }
    }
}

impl Default for Styles {
    fn default() -> Self {
        Self::resolve(
            crate::themes::BuiltinTheme::Default,
            &crate::config::StyleOverrides::default(),
            None,
        )
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
}
