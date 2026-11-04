mod app;
mod storage;
mod ui;

use std::{io, panic, process::ExitCode};

use crossterm::{
    cursor::Show,
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
};

use app::App;
use storage::{Result, Storage};

const USAGE: &str = "Usage: sati [--no-wrap] [--accent '#RRGGBB']";

// Report repeats for plain characters and Enter too; alternate keys preserve shifted text.
const KEYBOARD_FLAGS: KeyboardEnhancementFlags =
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        .union(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
        .union(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES)
        .union(KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS);

#[derive(Debug, PartialEq)]
struct Options {
    wrap: bool,
    accent: ratatui::style::Color,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sati: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let Some(options) = parse_arguments(std::env::args_os().skip(1))? else {
        println!(
            "{USAGE}\n\n--no-wrap          Keep one row per task.\n--accent '#RRGGBB' Set the selection accent (default: #e78a4e)."
        );
        return Ok(());
    };
    let storage = Storage::discover()?;
    let mut app = App::new(storage.load()?);
    app.wrap = options.wrap;
    app.accent = options.accent;
    let mut terminal = match ratatui::try_init() {
        Ok(terminal) => terminal,
        Err(error) => {
            ratatui::restore();
            return Err(error.into());
        }
    };
    // Restore input modes before Ratatui leaves the alternate screen.
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            PopKeyboardEnhancementFlags,
            DisableMouseCapture,
            Show
        );
        previous_hook(info);
    }));

    let result = (|| -> Result<()> {
        execute!(
            io::stdout(),
            EnableMouseCapture,
            EnableBracketedPaste,
            PushKeyboardEnhancementFlags(KEYBOARD_FLAGS)
        )?;
        loop {
            let mut rows = ratatui::layout::Rect::default();
            terminal.draw(|frame| rows = ui::draw(frame, &mut app))?;
            match event::read()? {
                Event::Key(key) => {
                    if app.key(key, &storage)? {
                        break;
                    }
                }
                Event::Mouse(mouse) => app.mouse(mouse, rows, &storage)?,
                Event::Paste(text) => app.paste(&text),
                _ => {}
            }
        }
        Ok(())
    })();
    let input_result = execute!(
        io::stdout(),
        DisableBracketedPaste,
        PopKeyboardEnhancementFlags,
        DisableMouseCapture,
        Show
    );
    let restore_result = ratatui::try_restore();
    result?;
    input_result?;
    restore_result?;
    Ok(())
}

fn parse_arguments(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<Option<Options>> {
    let mut options = Options {
        wrap: true,
        accent: ui::DEFAULT_ACCENT,
    };
    let mut help = false;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == "--no-wrap" {
            options.wrap = false;
        } else if argument == "--accent" {
            let value = arguments
                .next()
                .ok_or("--accent requires a color in #RRGGBB format")?;
            options.accent = parse_accent(&value)?;
        } else if argument == "--help" || argument == "-h" {
            help = true;
        } else {
            return Err(
                format!("unknown argument: {}. {USAGE}", argument.to_string_lossy()).into(),
            );
        }
    }
    Ok((!help).then_some(options))
}

fn parse_accent(value: &std::ffi::OsStr) -> Result<ratatui::style::Color> {
    let text = value.to_str().unwrap_or("");
    if text.len() != 7
        || !text.starts_with('#')
        || !text[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!(
            "invalid accent color: {}. Expected #RRGGBB",
            value.to_string_lossy()
        )
        .into());
    }
    let rgb = u32::from_str_radix(&text[1..], 16)?;
    Ok(ratatui::style::Color::Rgb(
        (rgb >> 16) as u8,
        (rgb >> 8) as u8,
        rgb as u8,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_is_enabled_by_default_and_can_be_disabled() {
        let defaults = parse_arguments([]).unwrap().unwrap();
        assert!(defaults.wrap);
        assert_eq!(defaults.accent, ui::DEFAULT_ACCENT);
        assert!(!parse_arguments(["--no-wrap".into()]).unwrap().unwrap().wrap);
        assert_eq!(parse_arguments(["--help".into()]).unwrap(), None);
        assert!(parse_arguments(["--unknown".into()]).is_err());
    }

    #[test]
    fn accent_accepts_hex_colors_and_combines_with_no_wrap() {
        for values in [
            ["--accent", "#a9B665", "--no-wrap"],
            ["--no-wrap", "--accent", "#A9b665"],
        ] {
            let options = parse_arguments(values.map(Into::into)).unwrap().unwrap();
            assert!(!options.wrap);
            assert_eq!(options.accent, ratatui::style::Color::Rgb(169, 182, 101));
        }
        for color in ["#000000", "#ffffff"] {
            assert!(parse_accent(std::ffi::OsStr::new(color)).is_ok());
        }
    }

    #[test]
    fn accent_rejects_missing_or_invalid_values() {
        assert!(parse_arguments(["--accent".into()]).is_err());
        for color in [
            "",
            "orange",
            "e78a4e",
            "#abc",
            "#12345678",
            "#zzzzzz",
            "#é1234",
            "--no-wrap",
        ] {
            assert!(
                parse_arguments(["--accent".into(), color.into()]).is_err(),
                "accepted {color}"
            );
        }
    }
}
