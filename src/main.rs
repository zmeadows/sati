mod app;
mod config;
mod export;
mod input;
mod notification;
mod search;
mod storage;
mod ui;

use std::{
    io::{self, Write},
    panic,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use ratatui::crossterm::{
    Command, QueueableCommand,
    cursor::Show,
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableMouseCapture, Event,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};

#[cfg(unix)]
use ratatui::crossterm::event::EnableBracketedPaste;

use app::App;
use storage::{Result, Storage};

const USAGE: &str = "Usage: sati [--config PATH] [--wrap | --no-wrap] [--logo | --no-logo] [--keybinds | --no-keybinds] [--round-corners | --square-corners] [--ascii-markers | --unicode-markers] [--completed-at-bottom | --no-completed-at-bottom] [--accent COLOR] [--version]\n       sati export [--output PATH | -o PATH] [--force]";

// Report repeats for plain characters and Enter too; alternate keys preserve shifted text.
const KEYBOARD_FLAGS: KeyboardEnhancementFlags =
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        .union(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
        .union(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES)
        .union(KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS);

#[derive(Debug, PartialEq)]
struct Options {
    wrap: bool,
    logo: bool,
    keybinds: bool,
    rounded: bool,
    ascii_markers: bool,
    completed_at_bottom: bool,
    accent: ratatui::style::Color,
    print_version: bool,
}

#[derive(Debug, PartialEq)]
struct CliOptions {
    display: config::Display,
    accent: ratatui::style::Color,
    print_version: bool,
    config: Option<std::path::PathBuf>,
}

impl CliOptions {
    fn resolve(self, config: config::Config) -> Options {
        Options {
            wrap: self.display.wrap.or(config.display.wrap).unwrap_or(true),
            logo: self.display.logo.or(config.display.logo).unwrap_or(true),
            keybinds: self
                .display
                .keybinds
                .or(config.display.keybinds)
                .unwrap_or(true),
            rounded: self
                .display
                .rounded_corners
                .or(config.display.rounded_corners)
                .unwrap_or(false),
            ascii_markers: self
                .display
                .ascii_markers
                .or(config.display.ascii_markers)
                .unwrap_or(false),
            completed_at_bottom: self
                .display
                .completed_at_bottom
                .or(config.display.completed_at_bottom)
                .unwrap_or(false),
            accent: self.accent,
            print_version: self.print_version,
        }
    }
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
    let mut arguments = std::env::args_os().skip(1).peekable();
    if arguments
        .peek()
        .is_some_and(|argument| argument == "export")
    {
        arguments.next();
        return export::run(arguments);
    }
    let Some(options) = parse_arguments(arguments)? else {
        println!(
            "{USAGE}\n\n--config PATH     Read this configuration file instead of the default location.\n--wrap / --no-wrap  Wrap tasks / keep one row per task.\n--logo / --no-logo  Show / hide the sati border title.\n--keybinds / --no-keybinds  Show / hide shortcut hints.\n--round-corners / --square-corners  Rounded / sharp borders.\n--ascii-markers / --unicode-markers  ASCII >, [x], [>] / Unicode task markers.\n--completed-at-bottom / --no-completed-at-bottom  Enable / disable grouping.\n--accent COLOR    Set accent: ANSI name, #RRGGBB, or index 0-255 (default: lightyellow).\n--version          Print the version.\n--help, -h         Show this help.\n\nOptional config.toml: Linux $XDG_CONFIG_HOME/sati/config.toml (or ~/.config/sati/config.toml), macOS ~/Library/Application Support/sati/config.toml, Windows %APPDATA%\\sati\\config.toml.\nPrecedence: CLI > TOML > defaults. [display] accepts optional booleans: wrap, logo, keybinds, rounded_corners, ascii_markers, completed_at_bottom.\n\nexport             Export Markdown to stdout or --output/-o PATH.\n--force            Allow replacing an export file."
        );
        return Ok(());
    };
    if options.print_version {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let config = config::Config::load(options.config.as_deref())?;
    let options = options.resolve(config);
    let storage = Storage::discover()?;
    let _instance_lock = storage.lock()?;
    let mut app = App::new(load_tasks(&storage, options.completed_at_bottom)?);
    app.wrap = options.wrap;
    app.logo = options.logo;
    app.keybinds = options.keybinds;
    app.rounded = options.rounded;
    app.unicode_markers = !options.ascii_markers;
    app.completed_at_bottom = options.completed_at_bottom;
    app.accent = options.accent;
    if let Some(cache) = storage.load_history() {
        app.restore_history(cache);
    }
    // Atomics let the panic hook clean up without locking state held by setup.
    let modes = Arc::new(TerminalModes::default());
    let panic_modes = Arc::clone(&modes);
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = panic_modes.restore(&mut io::stdout());
        previous_hook(info);
    }));

    let result = (|| -> Result<()> {
        terminal::enable_raw_mode()?;
        modes.raw.store(true, Ordering::Relaxed);
        // An output failure may occur after the screen-switch command was sent.
        modes.alternate.store(true, Ordering::Relaxed);
        execute!(io::stdout(), EnterAlternateScreen)?;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;
        // The capability probe retries reader errors internally; validate input first.
        event::poll(std::time::Duration::ZERO)?;
        let enhanced = keyboard_support(terminal::supports_keyboard_enhancement())?;
        modes.enable_input(&mut io::stdout(), enhanced)?;
        loop {
            let draw_started = std::time::Instant::now();
            if app
                .notification
                .as_ref()
                .is_some_and(|notice| notice.next_redraw(draw_started).is_none())
            {
                app.notification = None;
            }
            let mut rows = ratatui::layout::Rect::default();
            terminal.draw(|frame| rows = ui::draw(frame, &mut app))?;
            if let Some(wait) = app
                .notification
                .as_ref()
                .and_then(|notice| notice.next_redraw(draw_started))
                && !event::poll(wait.saturating_sub(draw_started.elapsed()))?
            {
                continue;
            }
            match event::read()? {
                Event::Key(key) => {
                    if app.key(key, &storage)? {
                        break;
                    }
                }
                Event::Mouse(mouse) => app.mouse(mouse, rows, &storage)?,
                Event::Paste(text) => app.paste(&text),
                Event::Resize(_, _) => app.notification = None,
                _ => {}
            }
        }
        Ok(())
    })();
    let restore_result = modes.restore(&mut io::stdout());
    finish_session(result, restore_result, app, &storage)
}

fn finish_session(
    result: Result<()>,
    restore_result: io::Result<()>,
    app: App,
    storage: &Storage,
) -> Result<()> {
    match (result, restore_result) {
        (Err(error), Err(restore)) => {
            Err(format!("{error}; could not restore terminal: {restore}").into())
        }
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(format!("could not restore terminal: {error}").into()),
        (Ok(()), Ok(())) => {
            if let Err(error) = storage.save_history(&app.into_history_cache()) {
                let _ = writeln!(
                    io::stderr(),
                    "sati: warning: could not save undo history: {error}"
                );
            }
            Ok(())
        }
    }
}

#[derive(Default)]
struct TerminalModes {
    raw: AtomicBool,
    alternate: AtomicBool,
    mouse: AtomicBool,
    paste: AtomicBool,
    keyboard: AtomicBool,
}

impl TerminalModes {
    fn enable_input(&self, output: &mut impl io::Write, enhanced: bool) -> io::Result<()> {
        enable_optional(output, &self.mouse, EnableMouseCapture)?;
        // Crossterm's Windows console reader does not emit Paste events.
        #[cfg(unix)]
        enable_optional(output, &self.paste, EnableBracketedPaste)?;
        if enhanced {
            enable_optional(
                output,
                &self.keyboard,
                PushKeyboardEnhancementFlags(KEYBOARD_FLAGS),
            )?;
        }
        Ok(())
    }

    fn restore(&self, output: &mut impl io::Write) -> io::Result<()> {
        let active = self.raw.load(Ordering::Relaxed) || self.alternate.load(Ordering::Relaxed);
        let keyboard = if self.keyboard.swap(false, Ordering::Relaxed) {
            execute!(output, PopKeyboardEnhancementFlags)
        } else {
            Ok(())
        };
        let paste = if self.paste.swap(false, Ordering::Relaxed) {
            execute!(output, DisableBracketedPaste)
        } else {
            Ok(())
        };
        // On Windows this restores the console mode captured after raw mode was enabled.
        let mouse = if self.mouse.swap(false, Ordering::Relaxed) {
            execute!(output, DisableMouseCapture)
        } else {
            Ok(())
        };
        let cursor = if active {
            execute!(output, Show)
        } else {
            Ok(())
        };
        let raw = if self.raw.swap(false, Ordering::Relaxed) {
            terminal::disable_raw_mode()
        } else {
            Ok(())
        };
        let alternate = if self.alternate.swap(false, Ordering::Relaxed) {
            execute!(output, LeaveAlternateScreen)
        } else {
            Ok(())
        };
        // Evaluate every cleanup step before returning the first failure.
        keyboard
            .and(paste)
            .and(mouse)
            .and(cursor)
            .and(raw)
            .and(alternate)
    }
}

fn enable_optional(
    output: &mut impl io::Write,
    enabled: &AtomicBool,
    command: impl Command,
) -> io::Result<()> {
    let supported = optional_mode(output.queue(command).map(|_| ()))?;
    // Remember a fully queued command even if flushing subsequently fails.
    enabled.store(supported, Ordering::Relaxed);
    if supported {
        output.flush()?;
    }
    Ok(())
}

fn optional_mode(result: io::Result<()>) -> io::Result<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::Unsupported => Ok(false),
        Err(error) => Err(error),
    }
}

fn keyboard_support(result: io::Result<bool>) -> io::Result<bool> {
    match result {
        // Crossterm 0.28 reports its unanswered-query timeout as Other.
        Err(error)
            if error.kind() == io::ErrorKind::Unsupported
                || error.kind() == io::ErrorKind::TimedOut
                || (error.kind() == io::ErrorKind::Other
                    && error.to_string()
                        == "The keyboard enhancement status could not be read within a normal duration") =>
        {
            Ok(false)
        }
        result => result,
    }
}

fn load_tasks(storage: &Storage, completed_at_bottom: bool) -> Result<Vec<app::Task>> {
    let mut tasks = storage.load()?;
    if completed_at_bottom
        && tasks
            .windows(2)
            .any(|pair| pair[0].completed && !pair[1].completed)
    {
        tasks.sort_by_key(|task| task.completed);
        storage.save(&tasks)?;
    }
    Ok(tasks)
}

fn parse_arguments(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<Option<CliOptions>> {
    let mut options = CliOptions {
        display: config::Display::default(),
        accent: ui::DEFAULT_ACCENT,
        print_version: false,
        config: None,
    };
    let mut help = false;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        let flag = argument.to_str().unwrap_or("");
        let setting = match flag {
            "--wrap" | "--no-wrap" => Some((&mut options.display.wrap, flag == "--wrap")),
            "--logo" | "--no-logo" => Some((&mut options.display.logo, flag == "--logo")),
            "--keybinds" | "--no-keybinds" => {
                Some((&mut options.display.keybinds, flag == "--keybinds"))
            }
            "--round-corners" | "--square-corners" => Some((
                &mut options.display.rounded_corners,
                flag == "--round-corners",
            )),
            "--ascii-markers" | "--unicode-markers" => Some((
                &mut options.display.ascii_markers,
                flag == "--ascii-markers",
            )),
            "--completed-at-bottom" | "--no-completed-at-bottom" => Some((
                &mut options.display.completed_at_bottom,
                flag == "--completed-at-bottom",
            )),
            _ => None,
        };
        if let Some((setting, value)) = setting {
            if setting.is_some_and(|previous| previous != value) {
                return Err(format!("contradictory display flags: {flag}").into());
            }
            *setting = Some(value);
        } else if argument == "--config" {
            if options.config.is_some() {
                return Err("--config specified more than once".into());
            }
            let path = arguments.next().ok_or("--config requires a path")?;
            if path.is_empty()
                || path
                    .to_str()
                    .is_some_and(|value| value.starts_with("--") || value == "-h")
            {
                return Err("--config requires a path".into());
            }
            options.config = Some(path.into());
        } else if argument == "--add-to-bottom" {
            // Accepted for compatibility with v0.1.0; insertion is now key-specific.
        } else if argument == "--version" {
            options.print_version = true;
        } else if argument == "--accent" {
            let value = arguments
                .next()
                .ok_or("--accent requires a color (ANSI name, #RRGGBB, or index 0-255)")?;
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
    value.to_str()
        .and_then(|text| text.parse::<ratatui::style::Color>().ok())
        .filter(|color| *color != ratatui::style::Color::Reset)
        .ok_or_else(|| format!("invalid accent color: {}. Expected an ANSI color name, #RRGGBB, or index 0-255; reset is not allowed", value.to_string_lossy()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_arguments(
        arguments: impl IntoIterator<Item = std::ffi::OsString>,
    ) -> Result<Option<Options>> {
        super::parse_arguments(arguments)
            .map(|options| options.map(|cli| cli.resolve(config::Config::default())))
    }

    #[test]
    fn display_precedence_and_conflicts_cover_both_boolean_directions() {
        let fields = [
            "wrap",
            "logo",
            "keybinds",
            "rounded_corners",
            "ascii_markers",
            "completed_at_bottom",
        ];
        let flags = [
            ("--wrap", "--no-wrap"),
            ("--logo", "--no-logo"),
            ("--keybinds", "--no-keybinds"),
            ("--round-corners", "--square-corners"),
            ("--ascii-markers", "--unicode-markers"),
            ("--completed-at-bottom", "--no-completed-at-bottom"),
        ];
        let values = |options: Options| {
            [
                options.wrap,
                options.logo,
                options.keybinds,
                options.rounded,
                options.ascii_markers,
                options.completed_at_bottom,
            ]
        };
        for (index, (yes, no)) in flags.into_iter().enumerate() {
            for configured in [true, false] {
                let text = format!("[display]\n{} = {configured}", fields[index]);
                let config = || toml::from_str::<config::Config>(&text).unwrap();
                assert_eq!(
                    values(
                        super::parse_arguments([])
                            .unwrap()
                            .unwrap()
                            .resolve(config())
                    )[index],
                    configured
                );
                for (flag, expected) in [(yes, true), (no, false)] {
                    let options = super::parse_arguments([flag.into()])
                        .unwrap()
                        .unwrap()
                        .resolve(config());
                    assert_eq!(values(options)[index], expected);
                }
            }
            for pair in [[yes, no], [no, yes]] {
                assert!(super::parse_arguments(pair.map(Into::into)).is_err());
            }
        }
        assert!(
            super::parse_arguments(["--config", "one", "--config", "two"].map(Into::into)).is_err()
        );
        assert!(super::parse_arguments(["--config"].map(Into::into)).is_err());
    }

    #[test]
    fn history_is_compatible_with_decoded_tasks_not_json_formatting() {
        let fixture = crate::storage::tests::Fixture::new();
        let tasks: Vec<_> = [("one 日本語", false), ("two 🪷", true)]
            .map(|(text, completed)| app::Task {
                carried: false,
                text: text.into(),
                completed,
            })
            .into();
        fixture.storage.save(&tasks).unwrap();
        let mut app = App::new(tasks.clone());
        app.list.select(Some(1));
        fixture
            .storage
            .save_history(&app.into_history_cache())
            .unwrap();
        // Reordering JSON object keys and changing whitespace are not task edits.
        std::fs::write(fixture.task_path(), "[ {\"completed\":false,\"text\":\"one 日本語\"}, {\"completed\":true,\"text\":\"two 🪷\"} ]").unwrap();
        let mut app = App::new(load_tasks(&fixture.storage, false).unwrap());
        app.restore_history(fixture.storage.load_history().unwrap());
        assert_eq!(app.list.selected(), Some(1));
        assert!(!app.can_undo() && !app.can_redo());
        for (index, mut changed) in [tasks.clone(), tasks.clone(), tasks.clone()]
            .into_iter()
            .enumerate()
        {
            // Each iteration changes text, status, or ordering independently.
            match index {
                0 => changed[0].text.push('!'),
                1 => changed[0].completed = true,
                _ => changed.swap(0, 1),
            }
            fixture.storage.save(&changed).unwrap();
            let mut app = App::new(load_tasks(&fixture.storage, false).unwrap());
            app.restore_history(fixture.storage.load_history().unwrap());
            assert_eq!(app.tasks, changed);
            assert_eq!(app.list.selected(), Some(0));
        }
    }

    #[test]
    fn startup_grouping_rejects_incompatible_current_or_historical_order() {
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = App::new(
            [("done", true), ("active", false)]
                .map(|(text, completed)| app::Task {
                    carried: false,
                    text: text.into(),
                    completed,
                })
                .into(),
        );
        fixture.storage.save(&app.tasks).unwrap();
        app.list.select(Some(1));
        fixture
            .storage
            .save_history(&app.into_history_cache())
            .unwrap();
        let grouped = load_tasks(&fixture.storage, true).unwrap();
        assert_eq!(grouped[0].text, "active");
        let mut app = App::new(grouped);
        app.completed_at_bottom = true;
        app.restore_history(fixture.storage.load_history().unwrap());
        assert_eq!(app.list.selected(), Some(0));
        assert!(!app.can_undo() && !app.can_redo());
        // The current order can match even when an old snapshot is incompatible.
        let cache = serde_json::json!({"version":1,"tasks":app.tasks,"selected":1,
            "undo":[{"tasks":[{"text":"done","completed":true},{"text":"active","completed":false}],"selected":0}],"redo":[]});
        app.restore_history(serde_json::from_value(cache).unwrap());
        assert!(!app.can_undo());
        assert_eq!(app.list.selected(), Some(0));
    }

    #[test]
    fn only_successful_sessions_save_history_and_cache_failure_is_nonfatal() {
        let fixture = crate::storage::tests::Fixture::new();
        let tasks: Vec<_> = ["one", "two"]
            .map(|text| app::Task {
                carried: false,
                text: text.into(),
                completed: false,
            })
            .into();
        fixture.storage.save(&tasks).unwrap();
        let held = fixture.storage.lock().unwrap();
        finish_session(Ok(()), Ok(()), App::new(tasks.clone()), &fixture.storage).unwrap();
        let path = fixture.task_path().with_file_name("history.json");
        let bytes = std::fs::read(&path).unwrap();
        for cleanup_fails in [false, true] {
            let mut app = App::new(tasks.clone());
            app.list.select(Some(1));
            let result = if cleanup_fails {
                finish_session(
                    Ok(()),
                    Err(io::ErrorKind::BrokenPipe.into()),
                    app,
                    &fixture.storage,
                )
            } else {
                finish_session(Err("storage error".into()), Ok(()), app, &fixture.storage)
            };
            assert!(result.is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let tasks_before = std::fs::read(fixture.task_path()).unwrap();
        finish_session(Ok(()), Ok(()), App::new(tasks), &fixture.storage).unwrap();
        assert_eq!(std::fs::read(fixture.task_path()).unwrap(), tasks_before);
        assert!(path.is_dir());
        assert!(fixture.storage.lock().is_err());
        drop(held);
    }

    #[test]
    fn completed_at_bottom_groups_existing_tasks_stably_and_is_opt_in() {
        let fixture = crate::storage::tests::Fixture::new();
        let original: Vec<_> = [
            ("done 1", true),
            ("active 1", false),
            ("done 2", true),
            ("active 2", false),
        ]
        .map(|(text, completed)| app::Task {
            carried: false,
            text: text.into(),
            completed,
        })
        .into();
        fixture.storage.save(&original).unwrap();
        let contents = std::fs::read(fixture.task_path()).unwrap();
        assert_eq!(load_tasks(&fixture.storage, false).unwrap(), original);
        assert_eq!(std::fs::read(fixture.task_path()).unwrap(), contents);
        let grouped = load_tasks(&fixture.storage, true).unwrap();
        assert_eq!(
            grouped,
            [
                original[1].clone(),
                original[3].clone(),
                original[0].clone(),
                original[2].clone()
            ]
        );
        assert_eq!(fixture.storage.load().unwrap(), grouped);
        assert_eq!(load_tasks(&fixture.storage, true).unwrap(), grouped);
        assert!(!parse_arguments([]).unwrap().unwrap().completed_at_bottom);
        let options = parse_arguments(
            ["--completed-at-bottom", "--add-to-bottom", "--no-wrap"].map(Into::into),
        )
        .unwrap()
        .unwrap();
        assert!(options.completed_at_bottom);
        assert!(!options.wrap);
    }

    #[test]
    fn terminal_commands_encode_repeat_and_paste_reporting_and_restoration() {
        use ratatui::crossterm::{Command, event::EnableBracketedPaste};

        // Check ANSI encoding without assuming the running terminal supports it.
        let mut enabled = String::new();
        EnableBracketedPaste.write_ansi(&mut enabled).unwrap();
        PushKeyboardEnhancementFlags(KEYBOARD_FLAGS)
            .write_ansi(&mut enabled)
            .unwrap();
        assert!(enabled.contains("\x1b[?2004h"));
        let flags: u8 = enabled
            .split("\x1b[>")
            .nth(1)
            .unwrap()
            .split('u')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        // Protocol bits: disambiguation, event types, shifted keys, and all keys (including Enter).
        assert_eq!(flags & 0b1111, 0b1111);

        let mut restored = String::new();
        PopKeyboardEnhancementFlags
            .write_ansi(&mut restored)
            .unwrap();
        DisableBracketedPaste.write_ansi(&mut restored).unwrap();
        DisableMouseCapture.write_ansi(&mut restored).unwrap();
        Show.write_ansi(&mut restored).unwrap();
        assert!(restored.contains("\x1b[?2004l"));
        assert!(restored.contains("\x1b[<1u"));
        assert!(restored.contains("\x1b[?1006l"));
        assert!(restored.ends_with("\x1b[?25h"));
    }

    #[test]
    fn optional_features_only_fall_back_for_unsupported_modes_or_query_timeouts() {
        assert!(optional_mode(Ok(())).unwrap());
        assert!(!optional_mode(Err(io::ErrorKind::Unsupported.into())).unwrap());
        assert!(keyboard_support(Ok(true)).unwrap());
        assert!(!keyboard_support(Ok(false)).unwrap());
        for kind in [io::ErrorKind::Unsupported, io::ErrorKind::TimedOut] {
            assert!(!keyboard_support(Err(kind.into())).unwrap());
        }
        assert!(
            !keyboard_support(Err(io::Error::other(
                "The keyboard enhancement status could not be read within a normal duration"
            )))
            .unwrap()
        );
        for kind in [
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::Other,
        ] {
            assert_eq!(optional_mode(Err(kind.into())).unwrap_err().kind(), kind);
            assert_eq!(keyboard_support(Err(kind.into())).unwrap_err().kind(), kind);
        }
    }

    #[test]
    fn cleanup_without_enabled_modes_sends_nothing() {
        let modes = TerminalModes::default();
        let mut output = Vec::new();
        modes.restore(&mut output).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    #[cfg(windows)]
    fn windows_backend_does_not_enable_kitty_protocol() {
        assert!(!terminal::supports_keyboard_enhancement().unwrap());
        let modes = TerminalModes::default();
        assert!(!modes.keyboard.load(Ordering::Relaxed));
        assert!(!modes.paste.load(Ordering::Relaxed));
    }

    #[test]
    #[cfg(unix)]
    fn partial_setup_and_cleanup_failures_do_not_leave_other_modes_enabled() {
        struct FailOnce {
            bytes: Vec<u8>,
            remaining: usize,
        }
        impl io::Write for FailOnce {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.remaining == 0 {
                    self.remaining = usize::MAX;
                    return Err(io::ErrorKind::BrokenPipe.into());
                }
                let count = bytes.len().min(self.remaining);
                self.bytes.extend_from_slice(&bytes[..count]);
                self.remaining -= count;
                Ok(count)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut mouse = String::new();
        EnableMouseCapture.write_ansi(&mut mouse).unwrap();
        let modes = TerminalModes::default();
        let mut output = FailOnce {
            bytes: Vec::new(),
            remaining: mouse.len(),
        };
        assert_eq!(
            modes.enable_input(&mut output, true).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        modes.restore(&mut output).unwrap();
        let restored = String::from_utf8(output.bytes).unwrap();
        assert!(restored.contains("\x1b[?1006l"));
        assert!(!restored.contains("\x1b[?2004l"));
        assert!(!restored.contains("\x1b[<1u"));

        let modes = TerminalModes::default();
        modes.enable_input(&mut Vec::new(), true).unwrap();
        let mut output = FailOnce {
            bytes: Vec::new(),
            remaining: 0,
        };
        assert_eq!(
            modes.restore(&mut output).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        let restored = String::from_utf8(output.bytes).unwrap();
        assert!(restored.contains("\x1b[?2004l"));
        assert!(restored.contains("\x1b[?1006l"));
        let mut repeated = Vec::new();
        modes.restore(&mut repeated).unwrap();
        assert!(repeated.is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn input_cleanup_matches_enabled_modes_and_is_not_repeated() {
        for enhanced in [false, true] {
            let modes = TerminalModes::default();
            let mut output = Vec::new();
            modes.enable_input(&mut output, enhanced).unwrap();
            let enabled = String::from_utf8(output).unwrap();
            assert!(enabled.contains("\x1b[?1006h"));
            assert!(enabled.contains("\x1b[?2004h"));
            assert_eq!(enabled.contains("\x1b[>15u"), enhanced);
            let mut restored = Vec::new();
            modes.restore(&mut restored).unwrap();
            let restored = String::from_utf8(restored).unwrap();
            assert!(restored.contains("\x1b[?2004l"));
            assert!(restored.contains("\x1b[?1006l"));
            assert_eq!(restored.contains("\x1b[<1u"), enhanced);
            let mut repeated = Vec::new();
            modes.restore(&mut repeated).unwrap();
            assert!(repeated.is_empty());
        }
    }

    #[test]
    fn wrapping_is_enabled_by_default_and_can_be_disabled() {
        let defaults = parse_arguments([]).unwrap().unwrap();
        assert!(defaults.wrap);
        assert!(defaults.logo);
        assert!(defaults.keybinds);
        assert!(!defaults.rounded);
        assert!(!defaults.ascii_markers);
        assert!(
            parse_arguments(["--ascii-markers".into(), "--no-wrap".into()])
                .unwrap()
                .unwrap()
                .ascii_markers
        );
        assert_eq!(defaults.accent, ui::DEFAULT_ACCENT);
        assert!(!defaults.print_version);
        assert!(!parse_arguments(["--no-wrap".into()]).unwrap().unwrap().wrap);
        assert!(
            !parse_arguments(["--no-keybinds".into()])
                .unwrap()
                .unwrap()
                .keybinds
        );
        let options = parse_arguments(["--no-logo", "--round-corners"].map(Into::into))
            .unwrap()
            .unwrap();
        assert!(!options.logo);
        assert!(options.rounded);
        assert!(options.wrap);
        assert_eq!(
            parse_arguments(["--add-to-bottom".into()])
                .unwrap()
                .unwrap(),
            defaults
        );
        assert_eq!(parse_arguments(["--help".into()]).unwrap(), None);
        assert!(parse_arguments(["--unknown".into()]).is_err());
    }

    #[test]
    fn version_preserves_option_parsing_and_argument_validation() {
        let options =
            parse_arguments(["--version", "--no-wrap", "--accent", "#a9b665"].map(Into::into))
                .unwrap()
                .unwrap();
        assert!(options.print_version);
        assert!(!options.wrap);
        assert_eq!(options.accent, ratatui::style::Color::Rgb(169, 182, 101));
        assert_eq!(
            parse_arguments(["--version", "--help"].map(Into::into)).unwrap(),
            None
        );
        assert!(parse_arguments(["--version", "--unknown"].map(Into::into)).is_err());
        assert!(parse_arguments(["--version", "--accent"].map(Into::into)).is_err());
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
    fn accent_accepts_native_named_and_indexed_colors() {
        use ratatui::style::Color;
        for (text, expected) in [
            ("yellow", Color::Yellow),
            ("lightblue", Color::LightBlue),
            ("3", Color::Indexed(3)),
            ("255", Color::Indexed(255)),
        ] {
            assert_eq!(parse_accent(std::ffi::OsStr::new(text)).unwrap(), expected);
        }
    }

    #[test]
    fn accent_rejects_missing_or_invalid_values() {
        assert!(parse_arguments(["--accent".into()]).is_err());
        for color in [
            "",
            "orange",
            "reset",
            "256",
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
