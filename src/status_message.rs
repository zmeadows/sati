use std::time::{Duration, Instant};

use ratatui::style::Style;

// Phases run sequentially; their durations determine the total lifetime.
// Any nonnegative Duration is valid; zero skips a phase (all zero expires immediately).
const ACCENT_DURATION: Duration = Duration::from_millis(300);
const NORMAL_DURATION: Duration = Duration::from_millis(3260);

pub struct StatusMessage {
    message: String,
    started: Instant,
}

impl StatusMessage {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            started: Instant::now(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn compact_message(&self) -> &str {
        match self.message.as_str() {
            "Copy sent to terminal" => "Copy sent",
            message if message.starts_with("Copy failed") => "Copy failed",
            message if message.starts_with("Undid: ") => "Undid",
            message if message.starts_with("Redid: ") => "Redid",
            message if message.starts_with("Cleared ") => "Cleared",
            message if message.starts_with("Theme: ") => message
                .rsplit_once(" (")
                .and_then(|(_, id)| id.strip_suffix(')'))
                .unwrap_or("Theme changed"),
            message => message,
        }
    }

    fn timing(&self, now: Instant) -> Option<(usize, Duration)> {
        timing(
            now.saturating_duration_since(self.started),
            [ACCENT_DURATION, NORMAL_DURATION],
        )
    }

    pub fn next_redraw(&self, now: Instant) -> Option<Duration> {
        self.timing(now).map(|(_, wait)| wait)
    }

    pub fn style(&self, now: Instant, accent: Style, normal: Style) -> Option<Style> {
        self.timing(now).map(|(phase, _)| [accent, normal][phase])
    }
}

fn timing(mut elapsed: Duration, phases: [Duration; 2]) -> Option<(usize, Duration)> {
    for (phase, duration) in phases.into_iter().enumerate() {
        if elapsed < duration {
            return Some((phase, duration - elapsed));
        }
        // Subtract instead of summing deadlines to avoid overflow for long phases.
        elapsed -= duration;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn compact_theme_messages_keep_exact_identifiers() {
        for (message, compact) in [
            ("Theme: Rosé Pine (rose-pine)", "rose-pine"),
            ("Theme: Default (default)", "default"),
            (
                "Theme: Everforest Dark / Medium (everforest-medium)",
                "everforest-medium",
            ),
            ("Copied task", "Copied task"),
            ("Copy sent to terminal", "Copy sent"),
            ("Copy failed: output unavailable", "Copy failed"),
            ("Undid: clear 3 completed tasks", "Undid"),
            ("Redid: delete task", "Redid"),
            ("Cleared 2 tasks, kept 1 carried", "Cleared"),
            ("Nothing to undo", "Nothing to undo"),
        ] {
            assert_eq!(StatusMessage::new(message).compact_message(), compact);
        }
    }

    #[test]
    fn status_messages_use_configured_semantic_styles_without_changing_deadlines() {
        let config: crate::config::Config = toml::from_str("[styles.normal]\nfg='lightblue'\nmodifiers=['bold']\n[styles.muted]\nmodifiers=['italic']\n[styles.accent]\nfg='3'\nmodifiers=[]").unwrap();
        let styles = crate::styles::Styles::resolve(config.theme, &config.styles, None);
        let notice = StatusMessage::new("Copied task");

        for (elapsed, expected) in [
            (0, styles.accent),
            (299, styles.accent),
            (300, styles.normal),
            (3559, styles.normal),
        ] {
            assert_eq!(
                notice.style(
                    notice.started + Duration::from_millis(elapsed),
                    styles.accent,
                    styles.normal
                ),
                Some(expected)
            );
        }

        assert!(
            notice
                .next_redraw(notice.started + Duration::from_millis(3560))
                .is_none()
        );
    }

    #[test]
    fn waits_only_for_boundaries_and_skips_elapsed_phases() {
        let ms = Duration::from_millis;
        let phases = [ms(300), ms(3260)];

        for (elapsed, phase, wait) in [
            (0, 0, 300),
            (299, 0, 1),
            (300, 1, 3260),
            (1000, 1, 2560),
            (3559, 1, 1),
        ] {
            assert_eq!(timing(ms(elapsed), phases), Some((phase, ms(wait))));
        }

        assert!(timing(ms(3560), phases).is_none());
        assert!(timing(ms(5000), phases).is_none());
    }

    #[test]
    fn zero_short_and_large_durations_skip_without_overflow() {
        let zero = Duration::ZERO;
        let tick = Duration::from_nanos(1);
        assert!(timing(zero, [zero; 2]).is_none());

        for phase in 0..2 {
            let mut phases = [zero; 2];
            phases[phase] = tick;
            assert_eq!(timing(zero, phases), Some((phase, tick)));
            assert!(timing(tick, phases).is_none());
        }

        assert_eq!(
            timing(Duration::MAX, [Duration::MAX; 2]),
            Some((1, Duration::MAX))
        );
    }

    #[test]
    fn phases_select_styles_directly_and_replacements_restart() {
        let first = StatusMessage::new("Copied task");
        let start = first.started;

        for colors in [
            [Color::Rgb(255, 0, 128), Color::Rgb(212, 190, 152)],
            [Color::Red, Color::Reset],
            [Color::Indexed(3), Color::Indexed(8)],
        ] {
            let [accent, normal] = colors.map(|color| Style::default().fg(color));

            for (elapsed, expected) in [(0, accent), (299, accent), (300, normal), (3559, normal)] {
                assert_eq!(
                    first.style(start + Duration::from_millis(elapsed), accent, normal),
                    Some(expected)
                );
            }

            assert!(
                first
                    .style(start + Duration::from_millis(3560), accent, normal)
                    .is_none()
            );
        }

        let replacement = StatusMessage {
            message: "Copy sent to terminal".into(),
            started: start + Duration::from_secs(4),
        };

        assert_eq!(
            replacement.style(
                replacement.started,
                Style::default().fg(Color::Blue),
                Style::default()
            ),
            Some(Style::default().fg(Color::Blue))
        );

        for (elapsed, wait) in [
            (0, Some(300)),
            (300, Some(3260)),
            (3559, Some(1)),
            (3560, None),
        ] {
            assert_eq!(
                replacement.next_redraw(replacement.started + Duration::from_millis(elapsed)),
                wait.map(Duration::from_millis)
            );
        }
    }
}
