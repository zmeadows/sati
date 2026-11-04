mod app;
mod storage;
mod ui;

use std::{io, panic, process::ExitCode};

use crossterm::{
    cursor::Show,
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
};

use app::App;
use storage::{Result, Storage};

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
    let Some(wrap) = parse_arguments(std::env::args_os().skip(1))? else {
        println!(
            "Usage: sati [--no-wrap]\n\nLong tasks wrap by default. --no-wrap keeps one row per task."
        );
        return Ok(());
    };
    let storage = Storage::discover()?;
    let mut app = App::new(storage.load()?);
    app.wrap = wrap;
    let mut terminal = match ratatui::try_init() {
        Ok(terminal) => terminal,
        Err(error) => {
            ratatui::restore();
            return Err(error.into());
        }
    };
    // Ratatui's hook restores raw mode and the alternate screen; also release the mouse.
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableMouseCapture, Show);
        previous_hook(info);
    }));

    let result = (|| -> Result<()> {
        execute!(io::stdout(), EnableMouseCapture)?;
        loop {
            let mut rows = ratatui::layout::Rect::default();
            terminal.draw(|frame| rows = ui::draw(frame, &mut app))?;
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if app.key(key, &storage)? {
                        break;
                    }
                }
                Event::Mouse(mouse) => app.mouse(mouse, rows, &storage)?,
                _ => {}
            }
        }
        Ok(())
    })();
    let mouse_result = execute!(io::stdout(), DisableMouseCapture, Show);
    let restore_result = ratatui::try_restore();
    result?;
    mouse_result?;
    restore_result?;
    Ok(())
}

fn parse_arguments(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<Option<bool>> {
    let mut wrap = true;
    let mut help = false;
    for argument in arguments {
        if argument == "--no-wrap" {
            wrap = false;
        } else if argument == "--help" || argument == "-h" {
            help = true;
        } else {
            return Err(format!(
                "unknown argument: {}. Usage: sati [--no-wrap]",
                argument.to_string_lossy()
            )
            .into());
        }
    }
    Ok((!help).then_some(wrap))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_is_enabled_by_default_and_can_be_disabled() {
        assert_eq!(parse_arguments([]).unwrap(), Some(true));
        assert_eq!(parse_arguments(["--no-wrap".into()]).unwrap(), Some(false));
        assert_eq!(parse_arguments(["--help".into()]).unwrap(), None);
        assert!(parse_arguments(["--unknown".into()]).is_err());
    }
}
