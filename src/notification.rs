use std::time::{Duration, Instant};

use ratatui::style::Color;

// Phases run sequentially; zero durations skip a phase. The frame interval must
// be positive and controls smoothness only, not the sum of the phase durations.
const NOTIFICATION_SETTLE_DURATION: Duration = Duration::from_millis(180);
const NOTIFICATION_HOLD_DURATION: Duration = Duration::from_millis(1200);
const NOTIFICATION_FADE_DURATION: Duration = Duration::from_millis(400);
const NOTIFICATION_FRAME_INTERVAL: Duration = Duration::from_millis(25);
const _: () = assert!(!NOTIFICATION_FRAME_INTERVAL.is_zero());

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

    fn timing(&self, now: Instant) -> Option<(usize, f64, Duration)> {
        timing(
            now.saturating_duration_since(self.started),
            [
                NOTIFICATION_SETTLE_DURATION,
                NOTIFICATION_HOLD_DURATION,
                NOTIFICATION_FADE_DURATION,
            ],
            NOTIFICATION_FRAME_INTERVAL,
        )
    }

    pub fn next_redraw(&self, now: Instant) -> Option<Duration> {
        self.timing(now).map(|(_, _, wait)| wait)
    }

    pub fn foreground(
        &self,
        now: Instant,
        accent: Color,
        normal: Color,
        dim: Color,
    ) -> Option<Color> {
        self.timing(now).map(|(phase, progress, _)| match phase {
            0 => interpolate(accent, normal, progress),
            1 => normal,
            _ => interpolate(normal, dim, progress),
        })
    }
}

fn timing(
    mut elapsed: Duration,
    phases: [Duration; 3],
    interval: Duration,
) -> Option<(usize, f64, Duration)> {
    assert!(!interval.is_zero());
    for (phase, duration) in phases.into_iter().enumerate() {
        if elapsed < duration {
            let remaining = duration - elapsed;
            let wait = if phase == 1 {
                remaining
            } else {
                remaining.min(interval)
            };
            return Some((phase, elapsed.as_secs_f64() / duration.as_secs_f64(), wait));
        }
        // Subtract instead of adding deadlines, even for very large durations.
        elapsed -= duration;
    }
    None
}

fn interpolate(from: Color, to: Color, progress: f64) -> Color {
    let (Color::Rgb(r, g, b), Color::Rgb(tr, tg, tb)) = (from, to) else {
        // The current palette and CLI accents are RGB; avoid terminal color queries.
        return to;
    };
    let t = progress.clamp(0.0, 1.0);
    let eased = t * t * (3.0 - 2.0 * t);
    let channel =
        |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * eased).round() as u8;
    Color::Rgb(channel(r, tr), channel(g, tg), channel(b, tb))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadlines_follow_phase_boundaries_and_skip_missed_frames() {
        let ms = Duration::from_millis;
        let phases = [ms(180), ms(1200), ms(400)];
        for (elapsed, phase, wait) in [
            (0, 0, 25),
            (179, 0, 1),
            (180, 1, 1200),
            (1000, 1, 380),
            (1380, 2, 25),
            (1779, 2, 1),
        ] {
            let (actual, _, next) = timing(ms(elapsed), phases, ms(25)).unwrap();
            assert_eq!((actual, next), (phase, ms(wait)));
        }
        assert!(timing(ms(1780), phases, ms(25)).is_none());
        assert!(timing(ms(5000), phases, ms(25)).is_none());
        for interval in [ms(1), ms(25), ms(5000)] {
            assert_eq!(timing(ms(180), phases, interval).unwrap().2, ms(1200));
            assert_eq!(timing(ms(1500), phases, interval).unwrap().1, 0.3);
            assert!(timing(ms(1780), phases, interval).is_none());
        }
    }

    #[test]
    fn zero_short_and_large_durations_do_not_divide_or_overflow() {
        let zero = Duration::ZERO;
        let tick = Duration::from_nanos(1);
        for phases in [
            [zero, zero, zero],
            [tick, zero, zero],
            [zero, tick, zero],
            [zero, zero, tick],
        ] {
            assert!(timing(tick, phases, Duration::from_secs(10)).is_none());
            if phases != [zero; 3] {
                assert_eq!(timing(zero, phases, tick).unwrap().1, 0.0);
                assert_eq!(timing(zero, phases, tick).unwrap().2, tick);
            }
        }
        assert!(timing(zero, [zero; 3], tick).is_none());
        assert_eq!(
            timing(Duration::MAX, [Duration::MAX; 3], tick).unwrap().0,
            1
        );
    }

    #[test]
    fn colors_ease_from_accent_and_replacements_restart_the_lifetime() {
        let accent = Color::Rgb(255, 0, 128);
        let normal = Color::Rgb(100, 100, 100);
        let dim = Color::Rgb(0, 0, 0);
        let first = Notification::new("Copied task");
        let start = first.started;
        assert_eq!(first.foreground(start, accent, normal, dim), Some(accent));
        assert_eq!(
            first.foreground(start + Duration::from_millis(90), accent, normal, dim),
            Some(Color::Rgb(178, 50, 114))
        );
        assert_eq!(
            first.foreground(start + Duration::from_millis(180), accent, normal, dim),
            Some(normal)
        );
        assert_eq!(
            first.foreground(start + Duration::from_millis(1580), accent, normal, dim),
            Some(Color::Rgb(50, 50, 50))
        );
        assert!(
            first
                .foreground(start + Duration::from_millis(1780), accent, normal, dim)
                .is_none()
        );
        let replacement = Notification {
            message: "Copy sent to terminal".into(),
            started: start + Duration::from_secs(2),
        };
        assert_eq!(
            replacement.foreground(replacement.started, accent, normal, dim),
            Some(accent)
        );
        assert_eq!(
            replacement.next_redraw(replacement.started),
            Some(NOTIFICATION_FRAME_INTERVAL)
        );
        assert_eq!(interpolate(accent, normal, -1.0), accent);
        assert_eq!(interpolate(accent, normal, 2.0), normal);
    }
}
