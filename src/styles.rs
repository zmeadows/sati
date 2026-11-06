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

impl Default for Styles {
    fn default() -> Self {
        // Explicitly clear inherited DIM on normal/accented child spans.
        let normal = Style::default()
            .fg(Color::Reset)
            .remove_modifier(Modifier::DIM);
        let muted = normal.add_modifier(Modifier::DIM);
        Self {
            normal,
            muted,
            completed: muted,
            border: muted,
            accent: normal.fg(DEFAULT_ACCENT),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
