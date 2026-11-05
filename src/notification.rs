use std::time::{Duration, Instant};

use ratatui::style::Style;

// Phases run sequentially; their durations determine the total lifetime.
// Any nonnegative Duration is valid; zero skips a phase (all zero expires immediately).
const NOTIFICATION_ACCENT_DURATION: Duration = Duration::from_millis(180);
const NOTIFICATION_NORMAL_DURATION: Duration = Duration::from_millis(1200);
const NOTIFICATION_DIM_DURATION: Duration = Duration::from_millis(400);

pub struct Notification {
    message: String,
    started: Instant,
}

impl Notification {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            started: Instant::now(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    fn timing(&self, now: Instant) -> Option<(usize, Duration)> {
        timing(
            now.saturating_duration_since(self.started),
            [
                NOTIFICATION_ACCENT_DURATION,
                NOTIFICATION_NORMAL_DURATION,
                NOTIFICATION_DIM_DURATION,
            ],
        )
    }

    pub fn next_redraw(&self, now: Instant) -> Option<Duration> {
        self.timing(now).map(|(_, wait)| wait)
    }

    pub fn style(&self, now: Instant, accent: Style, normal: Style, dim: Style) -> Option<Style> {
        self.timing(now)
            .map(|(phase, _)| [accent, normal, dim][phase])
    }
}

fn timing(mut elapsed: Duration, phases: [Duration; 3]) -> Option<(usize, Duration)> {
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
    use ratatui::style::{Color, Modifier};

    #[test]
    fn waits_only_for_boundaries_and_skips_elapsed_phases() {
        let ms = Duration::from_millis;
        let phases = [ms(180), ms(1200), ms(400)];
        for (elapsed, phase, wait) in [
            (0, 0, 180),
            (179, 0, 1),
            (180, 1, 1200),
            (1000, 1, 380),
            (1380, 2, 400),
            (1779, 2, 1),
        ] {
            assert_eq!(timing(ms(elapsed), phases), Some((phase, ms(wait))));
        }
        assert!(timing(ms(1780), phases).is_none());
        assert!(timing(ms(5000), phases).is_none());
    }

    #[test]
    fn zero_short_and_large_durations_skip_without_overflow() {
        let zero = Duration::ZERO;
        let tick = Duration::from_nanos(1);
        assert!(timing(zero, [zero; 3]).is_none());
        for phase in 0..3 {
            let mut phases = [zero; 3];
            phases[phase] = tick;
            assert_eq!(timing(zero, phases), Some((phase, tick)));
            assert!(timing(tick, phases).is_none());
        }
        assert_eq!(
            timing(Duration::MAX, [Duration::MAX; 3]),
            Some((1, Duration::MAX))
        );
    }

    #[test]
    fn phases_select_colors_directly_and_replacements_restart() {
        let first = Notification::new("Copied task");
        let start = first.started;
        for colors in [
            [
                Color::Rgb(255, 0, 128),
                Color::Rgb(212, 190, 152),
                Color::Rgb(146, 131, 116),
            ],
            [Color::Red, Color::Reset, Color::Indexed(8)],
        ] {
            for (elapsed, phase) in [
                (0, 0),
                (90, 0),
                (179, 0),
                (180, 1),
                (1379, 1),
                (1380, 2),
                (1779, 2),
            ] {
                assert_eq!(
                    first.style(
                        start + Duration::from_millis(elapsed),
                        Style::default().fg(colors[0]),
                        Style::default().fg(colors[1]),
                        Style::default().fg(colors[2]).add_modifier(Modifier::DIM)
                    ),
                    Some(if phase == 2 {
                        Style::default()
                            .fg(colors[phase])
                            .add_modifier(Modifier::DIM)
                    } else {
                        Style::default().fg(colors[phase])
                    })
                );
            }
            assert!(
                first
                    .style(
                        start + Duration::from_millis(1780),
                        Style::default().fg(colors[0]),
                        Style::default().fg(colors[1]),
                        Style::default().fg(colors[2]).add_modifier(Modifier::DIM)
                    )
                    .is_none()
            );
        }
        let replacement = Notification {
            message: "Copy sent to terminal".into(),
            started: start + Duration::from_secs(2),
        };
        assert_eq!(
            replacement.style(
                replacement.started,
                Style::default().fg(Color::Blue),
                Style::default(),
                Style::default().add_modifier(Modifier::DIM)
            ),
            Some(Style::default().fg(Color::Blue))
        );
        assert_eq!(
            replacement.next_redraw(replacement.started),
            Some(NOTIFICATION_ACCENT_DURATION)
        );
    }
}
