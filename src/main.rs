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
    let storage = Storage::discover()?;
    let mut app = App::new(storage.load()?);
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
