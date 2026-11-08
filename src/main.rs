mod app;
mod config;
mod export;
mod input;
mod search;
mod status_message;
mod storage;
mod styles;
mod themes;
mod ui;

use std::{
    io, panic,
    path::PathBuf,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
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

use clap::{
    Args, Parser, Subcommand,
    builder::{
        PossibleValuesParser, TypedValueParser,
        styling::{AnsiColor, Styles},
    },
};
use ratatui::style::Color;

use app::App;
use storage::{Result, Storage};

// Cargo-like help colors; clap drops them when the output is not a color terminal.
const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().bold())
    .usage(AnsiColor::Green.on_default().bold())
    .literal(AnsiColor::Cyan.on_default().bold())
    .placeholder(AnsiColor::Cyan.on_default());

const DISPLAY_HEADING: &str = "Display";

const CONFIG_HELP: &str = concat!(
    "Settings can also come from config.toml; command-line options take precedence.\n",
    "See the README for the configuration file and its location.",
);

// Report repeats for plain characters and Enter too; alternate keys preserve shifted text.
const KEYBOARD_FLAGS: KeyboardEnhancementFlags =
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        .union(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
        .union(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES)
        .union(KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS);

#[derive(Debug, PartialEq)]
struct Options {
    wrap: bool,
    status_messages: bool,
    status_message_duration: std::time::Duration,
    show_help_hint: bool,
    border_style: config::BorderStyle,
    marker_style: config::MarkerStyle,
    styles: styles::Styles,
    theme: themes::BuiltinTheme,
    style_overrides: config::StyleOverrides,
    cli_accent: Option<Color>,
}

#[derive(Debug, Parser)]
#[command(
    version,
    about,
    styles = HELP_STYLES,
    after_help = CONFIG_HELP,
    args_conflicts_with_subcommands = true,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,

    #[command(flatten)]
    options: CliOptions,
}

// Each subcommand runs instead of the interactive checklist.
#[derive(Debug, PartialEq, Subcommand)]
enum CliCommand {
    /// Print the checklist as Markdown
    ///
    /// Writes every task, in list order, to stdout as a Markdown checklist. It works while
    /// sati is open and never changes the task file.
    Export,
}

// Each on/off display pair overrides the other, so the last flag given wins.
#[derive(Debug, Args)]
struct CliOptions {
    /// Read settings from this file instead of the default config.toml
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Use a built-in color theme
    #[arg(
        long,
        value_name = "NAME",
        value_parser = theme_parser(),
        help_heading = DISPLAY_HEADING
    )]
    theme: Option<themes::BuiltinTheme>,

    /// Override the accent color: an ANSI name, #RRGGBB, or index 0-255
    #[arg(
        long,
        value_name = "COLOR",
        value_parser = parse_accent,
        help_heading = DISPLAY_HEADING
    )]
    accent: Option<Color>,

    /// Wrap long tasks onto several rows (default)
    #[arg(long, overrides_with = "no_wrap", help_heading = DISPLAY_HEADING)]
    wrap: bool,

    /// Keep each task on a single row
    #[arg(long, overrides_with = "wrap", help_heading = DISPLAY_HEADING)]
    no_wrap: bool,

    /// Draw the border with square or rounded corners
    #[arg(long, value_name = "STYLE", help_heading = DISPLAY_HEADING)]
    border_style: Option<config::BorderStyle>,

    /// Mark tasks with Unicode or ASCII symbols
    #[arg(long, value_name = "STYLE", help_heading = DISPLAY_HEADING)]
    marker_style: Option<config::MarkerStyle>,

    /// Show brief status messages on the border (default)
    #[arg(long, overrides_with = "no_status_messages", help_heading = DISPLAY_HEADING)]
    status_messages: bool,

    /// Hide brief status messages; errors are still shown
    #[arg(long, overrides_with = "status_messages", help_heading = DISPLAY_HEADING)]
    no_status_messages: bool,
}

impl CliOptions {
    fn resolve(&self, config: config::Config) -> Options {
        let theme = self.theme.unwrap_or(config.theme);

        Options {
            wrap: chosen_flag(self.wrap, self.no_wrap)
                .or(config.display.wrap)
                .unwrap_or(true),
            status_messages: chosen_flag(self.status_messages, self.no_status_messages)
                .or(config.display.status_messages)
                .unwrap_or(true),
            // Duration and help-hint visibility are TOML-only preferences, without CLI flags.
            status_message_duration: config
                .display
                .status_message_duration_ms
                .map_or(status_message::DEFAULT_DURATION, |milliseconds| {
                    std::time::Duration::from_millis(milliseconds.get())
                }),
            show_help_hint: config.display.show_help_hint.unwrap_or(true),
            border_style: self
                .border_style
                .or(config.display.border_style)
                .unwrap_or_default(),
            marker_style: self
                .marker_style
                .or(config.display.marker_style)
                .unwrap_or_default(),
            styles: styles::Styles::resolve(theme, &config.styles, self.accent),
            theme,
            style_overrides: config.styles,
            cli_accent: self.accent,
        }
    }
}

// Turns an on/off flag pair into an optional setting; `overrides_with` keeps at most one set.
fn chosen_flag(on: bool, off: bool) -> Option<bool> {
    if on {
        Some(true)
    } else if off {
        Some(false)
    } else {
        None
    }
}

// Listing the names lets clap show them in help and suggest close matches for typos.
fn theme_parser() -> impl TypedValueParser<Value = themes::BuiltinTheme> {
    let names = themes::BuiltinTheme::ALL.iter().map(|(_, name, _)| *name);

    PossibleValuesParser::new(names).map(|name| {
        name.parse::<themes::BuiltinTheme>()
            .expect("listed theme names parse")
    })
}

fn parse_accent(value: &str) -> std::result::Result<Color, String> {
    value
        .parse::<Color>()
        .ok()
        .filter(|color| *color != Color::Reset)
        .ok_or_else(|| {
            "expected an ANSI color name, #RRGGBB, or index 0-255; reset is not allowed".to_owned()
        })
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
    // Clap answers --help, --version, and argument errors itself and exits before
    // configuration, storage, or the terminal are touched.
    let cli = Cli::parse();

    if let Some(command) = cli.command {
        return match command {
            CliCommand::Export => export::run(),
        };
    }

    // Configuration is read once, before storage and the terminal are touched, so an
    // invalid file is reported on stderr and changes made later need a restart.
    let config = config::Config::load(cli.options.config.as_deref())?;
    let options = cli.options.resolve(config);

    let storage = Storage::discover()?;
    let _instance_lock = storage.lock()?;

    let mut app = App::from_document(storage.load_document()?);
    app.wrap = options.wrap;
    app.status_messages = options.status_messages;
    app.status_message_duration = options.status_message_duration;
    app.show_help_hint = options.show_help_hint;
    app.border_style = options.border_style;
    app.marker_style = options.marker_style;
    app.styles = options.styles;
    app.theme = options.theme;
    app.style_overrides = options.style_overrides;
    app.cli_accent = options.cli_accent;

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
        modes.cursor.store(true, Ordering::Relaxed);
        // An output failure may occur after the screen-switch command was sent.
        modes.alternate.store(true, Ordering::Relaxed);
        execute!(io::stdout(), EnterAlternateScreen)?;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;

        // The capability probe retries reader errors internally; validate input first.
        event::poll(std::time::Duration::ZERO)?;
        let enhanced = keyboard_support(terminal::supports_keyboard_enhancement())?;
        modes.enable_input(&mut io::stdout(), enhanced)?;

        let mut background = None;
        let mut rendered_theme = app.theme;

        loop {
            let changed = modes.update_background(
                &mut io::stdout(),
                &mut background,
                app.styles.normal.bg,
            )?;

            if changed || rendered_theme != app.theme {
                terminal.clear()?;
                rendered_theme = app.theme;
            }

            let draw_started = Instant::now();

            if app
                .status_message
                .as_ref()
                .is_some_and(|notice| notice.next_redraw(draw_started).is_none())
            {
                app.status_message = None;
            }

            let mut rows = ratatui::layout::Rect::default();
            terminal.draw(|frame| rows = ui::draw(frame, &mut app))?;

            // Wake to animate a status message or to hide an expired `s` hint; otherwise block
            // until the next input event.
            let status_redraw = app
                .status_message
                .as_ref()
                .and_then(|notice| notice.next_redraw(draw_started));
            let section_jump_redraw = app.section_jump_expires_in(draw_started);
            let next_redraw = [status_redraw, section_jump_redraw]
                .into_iter()
                .flatten()
                .min();

            if let Some(wait) = next_redraw
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
                Event::Resize(_, _) => app.status_message = None,
                _ => {}
            }
        }

        Ok(())
    })();

    let restore_result = modes.restore(&mut io::stdout());
    finish_session(result, restore_result)?;

    app.save_selection(&storage)
}

fn finish_session(result: Result<()>, restore_result: io::Result<()>) -> Result<()> {
    match (result, restore_result) {
        (Err(error), Err(restore)) => {
            Err(format!("{error}; could not restore terminal: {restore}").into())
        }
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(format!("could not restore terminal: {error}").into()),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[derive(Default)]
struct TerminalModes {
    raw: AtomicBool,
    alternate: AtomicBool,
    // Showing the cursor may still be pending after raw/screen cleanup succeeds.
    cursor: AtomicBool,
    mouse: AtomicBool,
    paste: AtomicBool,
    keyboard: AtomicBool,
    background: AtomicBool,
}

impl TerminalModes {
    fn update_background(
        &self,
        output: &mut impl io::Write,
        current: &mut Option<ratatui::style::Color>,
        background: Option<ratatui::style::Color>,
    ) -> io::Result<bool> {
        use ratatui::style::Color;
        let target = background.filter(|color| matches!(color, Color::Rgb(..)));

        if *current == target {
            return Ok(false);
        }

        if let Some(Color::Rgb(r, g, b)) = target {
            // Remember attempts too: a failed write may already have changed the terminal.
            self.background.store(true, Ordering::Relaxed);
            write!(output, "\x1b]11;#{r:02x}{g:02x}{b:02x}\x1b\\")?;
            output.flush()?;
        } else {
            self.restore_background(output)?;
        }

        *current = target;

        Ok(true)
    }

    fn restore_background(&self, output: &mut impl io::Write) -> io::Result<()> {
        if self.background.load(Ordering::Relaxed) {
            output.write_all(b"\x1b]111\x1b\\")?;
            output.flush()?;
            self.background.store(false, Ordering::Relaxed);
        }

        Ok(())
    }

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
        // A pop may have reached the terminal even if flushing failed. Never pop twice.
        let keyboard = if self.keyboard.swap(false, Ordering::Relaxed) {
            execute!(output, PopKeyboardEnhancementFlags)
        } else {
            Ok(())
        };

        let paste = restore_mode(&self.paste, || execute!(output, DisableBracketedPaste));

        // Windows restores a raw-mode console snapshot; retrying after raw cleanup
        // could re-enable raw mode. Keep this operation one-shot on that backend.
        #[cfg(windows)]
        let mouse = if self.mouse.swap(false, Ordering::Relaxed) {
            execute!(output, DisableMouseCapture)
        } else {
            Ok(())
        };

        #[cfg(not(windows))]
        let mouse = restore_mode(&self.mouse, || execute!(output, DisableMouseCapture));
        let cursor = restore_mode(&self.cursor, || execute!(output, Show));
        let raw = restore_mode(&self.raw, terminal::disable_raw_mode);
        let alternate = restore_mode(&self.alternate, || execute!(output, LeaveAlternateScreen));
        let background = self.restore_background(output);

        // Evaluate every cleanup step before returning the first failure.
        keyboard
            .and(paste)
            .and(mouse)
            .and(cursor)
            .and(raw)
            .and(alternate)
            .and(background)
    }
}

// Only for idempotent resets: an error leaves completion uncertain, so retain
// the flag for a later cleanup attempt, including when writing succeeded but flushing failed.
fn restore_mode(enabled: &AtomicBool, restore: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    if enabled.load(Ordering::Relaxed) {
        restore()?;
        enabled.store(false, Ordering::Relaxed);
    }

    Ok(())
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
        // Crossterm 0.29's Unix query returns Other + this message, with no typed timeout.
        // Match both narrowly so genuine I/O errors propagate; revisit on dependency upgrades.
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

#[cfg(test)]
mod tests {
    use super::*;

    // Parses arguments as if typed after `sati`, keeping only the interactive options.
    fn parse_cli(arguments: &[&str]) -> std::result::Result<CliOptions, clap::Error> {
        let arguments = std::iter::once("sati").chain(arguments.iter().copied());

        Cli::try_parse_from(arguments).map(|cli| cli.options)
    }

    fn resolve_defaults(arguments: &[&str]) -> Options {
        parse_cli(arguments)
            .unwrap()
            .resolve(config::Config::default())
    }

    // Records bytes even when flushing fails, making ambiguous command delivery explicit.
    #[derive(Default)]
    struct FailOnce {
        bytes: Vec<u8>,
        write_remaining: Option<usize>,
        flush_remaining: Option<usize>,
    }

    impl io::Write for FailOnce {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = match self.write_remaining {
                Some(0) => {
                    self.write_remaining = None;
                    return Err(io::ErrorKind::BrokenPipe.into());
                }
                Some(remaining) => {
                    let count = bytes.len().min(remaining);
                    self.write_remaining = Some(remaining - count);
                    count
                }
                None => bytes.len(),
            };

            self.bytes.extend_from_slice(&bytes[..count]);

            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            if let Some(remaining) = self.flush_remaining.as_mut() {
                if *remaining == 0 {
                    self.flush_remaining = None;
                    return Err(io::ErrorKind::BrokenPipe.into());
                }
                *remaining -= 1;
            }

            Ok(())
        }
    }

    #[test]
    fn finishing_preserves_application_and_terminal_error_reporting() {
        assert!(finish_session(Ok(()), Ok(())).is_ok());
        assert_eq!(
            finish_session(Err("task save failed".into()), Ok(()))
                .unwrap_err()
                .to_string(),
            "task save failed"
        );

        let error = finish_session(Ok(()), Err(io::ErrorKind::BrokenPipe.into()))
            .unwrap_err()
            .to_string();

        assert!(error.contains("could not restore terminal"));

        let both = finish_session(
            Err("task save failed".into()),
            Err(io::ErrorKind::BrokenPipe.into()),
        )
        .unwrap_err()
        .to_string();

        assert!(both.contains("task save failed") && both.contains("could not restore terminal"));
    }

    #[test]
    fn background_output_tracks_changes_and_restores_once_even_after_write_failure() {
        use ratatui::style::Color;
        let modes = TerminalModes::default();
        let mut output = Vec::new();
        let mut current = None;

        assert!(
            !modes
                .update_background(&mut output, &mut current, None)
                .unwrap()
        );
        assert!(output.is_empty());

        let color = Some(Color::Rgb(40, 40, 40));
        assert!(
            modes
                .update_background(&mut output, &mut current, color)
                .unwrap()
        );
        assert!(
            !modes
                .update_background(&mut output, &mut current, color)
                .unwrap()
        );
        assert_eq!(output, b"\x1b]11;#282828\x1b\\");
        assert!(
            modes
                .update_background(&mut output, &mut current, Some(Color::Reset))
                .unwrap()
        );
        assert_eq!(output, b"\x1b]11;#282828\x1b\\\x1b]111\x1b\\");

        output.clear();
        modes.restore(&mut output).unwrap();
        assert!(output.is_empty());

        modes
            .update_background(&mut output, &mut current, Some(Color::Rgb(1, 2, 3)))
            .unwrap();
        output.clear();
        modes.restore(&mut output).unwrap();
        modes.restore(&mut output).unwrap();

        assert_eq!(output, b"\x1b]111\x1b\\");

        struct Broken;
        impl io::Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let modes = TerminalModes::default();
        let mut current = None;

        assert!(
            modes
                .update_background(&mut Broken, &mut current, color)
                .is_err()
        );
        assert!(current.is_none());

        let mut recovery = Vec::new();
        modes.restore(&mut recovery).unwrap();
        assert_eq!(recovery, b"\x1b]111\x1b\\");
    }

    #[test]
    fn runtime_themes_synchronize_resolved_backgrounds_and_leave_config_unchanged() {
        use ratatui::{backend::TestBackend, crossterm::event::KeyCode, style::Color};
        use themes::BuiltinTheme::*;

        let fixture = storage::tests::Fixture::new();
        let path = fixture.directory.join("config.toml");

        for (configured, cli_theme, initial, changes) in [
            (
                "gruvbox-material",
                Some("onedark"),
                OneDark,
                [('t', GithubDark), ('T', OneDark), ('T', Nightfox)],
            ),
            (
                "solarized-dark",
                None,
                SolarizedDark,
                [('t', Default), ('T', SolarizedDark), ('t', Default)],
            ),
        ] {
            for custom_background in [None, Some(Color::Rgb(32, 36, 45))] {
                let mut text = format!(
                    "theme='{configured}'\n[styles.accent]\nfg='red'\nmodifiers=['bold']\n"
                );

                if custom_background.is_some() {
                    text.push_str("[styles.normal]\nbg='#20242d'\n");
                }

                std::fs::write(&path, &text).unwrap();
                let mut args = vec!["--accent", "blue"];

                if let Some(name) = cli_theme {
                    args.extend(["--theme", name]);
                }

                let options = parse_cli(&args)
                    .unwrap()
                    .resolve(config::Config::load(Some(&path)).unwrap());
                let mut app = App::new(Vec::new());
                app.styles = options.styles;
                app.theme = options.theme;
                app.style_overrides = options.style_overrides;
                app.cli_accent = options.cli_accent;
                let modes = TerminalModes::default();
                let mut current = None;
                let mut output = Vec::new();
                let mut terminal = ratatui::Terminal::new(TestBackend::new(40, 10)).unwrap();

                for (key, expected) in std::iter::once((None, initial))
                    .chain(changes.map(|(key, theme)| (Some(key), theme)))
                {
                    if let Some(key) = key {
                        app.key(KeyCode::Char(key).into(), &fixture.storage)
                            .unwrap();
                        assert_eq!(
                            app.status_message.as_ref().unwrap().message(),
                            format!("Theme: {} ({})", expected.display_name(), expected.id())
                        );
                    }

                    assert_eq!(app.theme, expected);
                    assert_eq!(app.styles.normal.fg, expected.base_styles().normal.fg);
                    assert_eq!(app.styles.accent.fg, Some(Color::Blue));
                    assert_eq!(
                        app.styles.accent.add_modifier,
                        ratatui::style::Modifier::BOLD
                    );

                    let background = custom_background.or(expected.base_styles().normal.bg);
                    assert_eq!(app.styles.normal.bg, background);

                    let previous = current;
                    output.clear();
                    let changed = modes
                        .update_background(&mut output, &mut current, app.styles.normal.bg)
                        .unwrap();

                    let expected_output = if previous == background {
                        String::new()
                    } else if let Some(Color::Rgb(r, g, b)) = background {
                        format!("\x1b]11;#{r:02x}{g:02x}{b:02x}\x1b\\")
                    } else {
                        "\x1b]111\x1b\\".into()
                    };

                    assert_eq!(changed, previous != background);
                    assert_eq!(output, expected_output.as_bytes());

                    terminal.clear().unwrap();
                    terminal
                        .draw(|frame| {
                            ui::draw(frame, &mut app);
                        })
                        .unwrap();

                    assert!(
                        terminal
                            .backend()
                            .buffer()
                            .content
                            .iter()
                            .all(|cell| cell.bg == background.unwrap_or(Color::Reset))
                    );
                }

                output.clear();
                modes.restore(&mut output).unwrap();
                assert_eq!(
                    output,
                    if current.is_some() {
                        b"\x1b]111\x1b\\".as_slice()
                    } else {
                        b""
                    }
                );
                assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
                assert!(!fixture.task_path().exists());
                assert!(!app.can_undo() && !app.can_redo());
            }
        }
    }

    #[test]
    fn every_theme_is_selectable_from_cli_over_toml() {
        for name in themes::BuiltinTheme::names().split(", ") {
            let config: config::Config = toml::from_str("theme='gruvbox-material'").unwrap();
            let cli = parse_cli(&["--theme", name]).unwrap();
            let selected = name.parse::<themes::BuiltinTheme>().unwrap();

            assert_eq!(
                cli.resolve(config).styles,
                styles::Styles::resolve(selected, &config::StyleOverrides::default(), None)
            );
        }
    }

    #[test]
    fn cli_theme_overrides_toml_in_both_directions() {
        for (configured, cli, expected) in [
            (
                "default",
                Some("gruvbox-material"),
                ratatui::style::Color::Rgb(212, 190, 152),
            ),
            (
                "gruvbox-material",
                Some("default"),
                ratatui::style::Color::Reset,
            ),
            (
                "gruvbox-material",
                None,
                ratatui::style::Color::Rgb(212, 190, 152),
            ),
        ] {
            let config = toml::from_str(&format!("theme='{configured}'")).unwrap();
            let args = cli.map_or_else(Vec::new, |name| vec!["--theme", name]);
            let cli = parse_cli(&args).unwrap();

            assert_eq!(cli.resolve(config).styles.normal.fg, Some(expected));
        }
    }

    #[test]
    fn theme_selection_keeps_cascading_and_final_accent_precedence() {
        let config: config::Config = toml::from_str("theme='default'\n[styles.normal]\nfg='lightblue'\nmodifiers=['bold']\n[styles.muted]\nmodifiers=[]\n[styles.accent]\nfg='red'\nmodifiers=['bold']").unwrap();
        let cli = parse_cli(&["--theme", "default", "--accent", "blue"]).unwrap();

        assert_eq!(cli.theme, Some(themes::BuiltinTheme::Default));

        let styles = cli.resolve(config).styles;
        assert_eq!(styles.normal.fg, Some(ratatui::style::Color::LightBlue));
        assert_eq!(styles.muted.fg, styles.normal.fg);
        assert!(styles.muted.add_modifier.is_empty());
        assert_eq!(styles.completed, styles.muted);
        assert_eq!(styles.border, styles.muted);
        assert_eq!(styles.accent.fg, Some(ratatui::style::Color::Blue));
        assert_eq!(styles.accent.add_modifier, ratatui::style::Modifier::BOLD);

        // Clap lists the available themes whenever a name is missing or unknown.
        for args in [
            vec!["--theme"],
            vec!["--theme", "unknown"],
            vec!["--theme", "Default"],
        ] {
            let error = parse_cli(&args).unwrap_err().to_string();

            assert!(error.contains("gruvbox-material"), "{error}");
        }

        assert!(parse_cli(&["--theme", "default", "--theme", "default"]).is_err());
    }

    #[test]
    fn cli_accent_overrides_only_the_resolved_foreground() {
        let config = || {
            toml::from_str::<config::Config>("[styles.accent]\nfg='blue'\nmodifiers=['bold']")
                .unwrap()
        };

        assert!(parse_cli(&[]).unwrap().accent.is_none());

        let options = parse_cli(&[]).unwrap().resolve(config());

        assert_eq!(options.styles.accent.fg, Some(ratatui::style::Color::Blue));

        let options = parse_cli(&["--accent", "red"]).unwrap().resolve(config());

        assert_eq!(options.styles.accent.fg, Some(ratatui::style::Color::Red));
        assert_eq!(
            options.styles.accent.add_modifier,
            ratatui::style::Modifier::BOLD
        );
    }

    #[test]
    fn display_precedence_and_last_flag_of_a_pair_cover_both_boolean_directions() {
        let fields = ["wrap", "status_messages"];

        let flags = [
            ("--wrap", "--no-wrap"),
            ("--status-messages", "--no-status-messages"),
        ];

        let values = |options: Options| [options.wrap, options.status_messages];

        for (index, (yes, no)) in flags.into_iter().enumerate() {
            for configured in [true, false] {
                let text = format!("[display]\n{} = {configured}", fields[index]);
                let config = || toml::from_str::<config::Config>(&text).unwrap();
                assert_eq!(
                    values(parse_cli(&[]).unwrap().resolve(config()))[index],
                    configured
                );

                for (flag, expected) in [(yes, true), (no, false)] {
                    let options = parse_cli(&[flag]).unwrap().resolve(config());

                    assert_eq!(values(options)[index], expected);
                }
            }

            // Each flag of a pair overrides the other, so the last one given wins.
            for (pair, expected) in [([yes, no], false), ([no, yes], true)] {
                assert_eq!(values(resolve_defaults(&pair))[index], expected);
            }
        }

        assert!(parse_cli(&["--config", "one", "--config", "two"]).is_err());
        assert!(parse_cli(&["--config"]).is_err());
    }

    #[test]
    fn style_options_take_precedence_over_config_which_takes_precedence_over_defaults() {
        use config::{BorderStyle, MarkerStyle};

        let config = |text: &str| toml::from_str::<config::Config>(text).unwrap();
        let styles = |options: Options| (options.border_style, options.marker_style);

        assert_eq!(
            styles(resolve_defaults(&[])),
            (BorderStyle::Square, MarkerStyle::Unicode)
        );

        let configured = "[display]\nborder_style = 'rounded'\nmarker_style = 'ascii'";

        assert_eq!(
            styles(parse_cli(&[]).unwrap().resolve(config(configured))),
            (BorderStyle::Rounded, MarkerStyle::Ascii)
        );

        let overridden = parse_cli(&["--border-style", "square", "--marker-style", "unicode"])
            .unwrap()
            .resolve(config(configured));

        assert_eq!(
            styles(overridden),
            (BorderStyle::Square, MarkerStyle::Unicode)
        );

        let from_defaults = resolve_defaults(&["--border-style=rounded", "--marker-style=ascii"]);

        assert_eq!(
            styles(from_defaults),
            (BorderStyle::Rounded, MarkerStyle::Ascii)
        );
    }

    #[test]
    fn style_options_reject_unknown_missing_and_removed_values() {
        for args in [
            vec!["--border-style", "round"],
            vec!["--border-style", "Rounded"],
            vec!["--border-style"],
            vec!["--marker-style", "emoji"],
            vec!["--marker-style"],
            vec!["--round-corners"],
            vec!["--square-corners"],
            vec!["--ascii-markers"],
            vec!["--unicode-markers"],
            vec!["--notifications"],
            vec!["--no-notifications"],
        ] {
            assert!(parse_cli(&args).is_err(), "accepted {args:?}");
        }
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

        for (kind, message) in [
            (io::ErrorKind::Other, "keyboard query reader failed"),
            (
                io::ErrorKind::Other,
                "The keyboard enhancement status could not be read within a normal duration: reader failed",
            ),
            (
                io::ErrorKind::Other,
                "The keyboard enhancement status could not be read within a normal duration ",
            ),
            (
                io::ErrorKind::PermissionDenied,
                "The keyboard enhancement status could not be read within a normal duration",
            ),
        ] {
            let error = keyboard_support(Err(io::Error::new(kind, message))).unwrap_err();
            assert_eq!(error.kind(), kind);
            assert_eq!(error.to_string(), message);
        }
    }

    #[test]
    #[cfg(unix)]
    fn session_cleanup_orders_cursor_screen_and_background_for_success_and_error() {
        for themed in [false, true] {
            for failed in [false, true] {
                let modes = TerminalModes::default();
                let mut output = Vec::new();
                modes
                    .update_background(
                        &mut output,
                        &mut None,
                        themed.then_some(ratatui::style::Color::Rgb(40, 40, 40)),
                    )
                    .unwrap();
                // Capture cleanup of an initialized screen without changing a real TTY.
                modes.alternate.store(true, Ordering::Relaxed);
                modes.cursor.store(true, Ordering::Relaxed);
                output.clear();

                let result = if failed {
                    Err("task save failed".into())
                } else {
                    Ok(())
                };

                let finished = finish_session(result, modes.restore(&mut output));
                assert_eq!(finished.is_err(), failed);

                let mut expected = b"\x1b[?25h\x1b[?1049l".to_vec();

                if themed {
                    expected.extend_from_slice(b"\x1b]111\x1b\\");
                }

                assert_eq!(output, expected);

                output.clear();
                modes.restore(&mut output).unwrap();
                assert!(output.is_empty());
            }
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
        let mut mouse = String::new();
        EnableMouseCapture.write_ansi(&mut mouse).unwrap();
        let modes = TerminalModes::default();
        let mut output = FailOnce {
            write_remaining: Some(mouse.len()),
            ..FailOnce::default()
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
            write_remaining: Some(0),
            ..FailOnce::default()
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
    fn cleanup_retries_only_safe_resets_after_partial_writes_and_flush_failures() {
        let mut mouse = String::new();
        DisableMouseCapture.write_ansi(&mut mouse).unwrap();

        let commands = [
            "\x1b[<1u",
            "\x1b[?2004l",
            &mouse,
            "\x1b[?25h",
            "\x1b[?1049l",
            "\x1b]111\x1b\\",
        ];

        for (failed, command) in commands.iter().enumerate() {
            let preceding: usize = commands[..failed].iter().map(|command| command.len()).sum();

            for (partial, flush) in [(0, false), (command.len() / 2, false), (0, true)] {
                let modes = TerminalModes::default();

                for enabled in [
                    &modes.keyboard,
                    &modes.paste,
                    &modes.mouse,
                    &modes.cursor,
                    &modes.alternate,
                    &modes.background,
                ] {
                    enabled.store(true, Ordering::Relaxed);
                }

                let mut output = FailOnce {
                    write_remaining: (!flush).then_some(preceding + partial),
                    flush_remaining: flush.then_some(failed),
                    ..FailOnce::default()
                };

                assert_eq!(
                    modes.restore(&mut output).unwrap_err().kind(),
                    io::ErrorKind::BrokenPipe
                );

                let mut expected = String::new();

                for (index, command) in commands.iter().enumerate() {
                    expected.push_str(if index == failed && !flush {
                        &command[..partial]
                    } else {
                        command
                    });
                }

                // Every later cleanup step still runs, including OSC 111.
                assert_eq!(
                    output.bytes,
                    expected.as_bytes(),
                    "step {failed}, flush {flush}"
                );

                output.bytes.clear();
                modes.restore(&mut output).unwrap();

                // A failed keyboard pop is deliberately consumed, even after a partial write.
                let retry = if failed == 0 {
                    b"".as_slice()
                } else {
                    command.as_bytes()
                };

                assert_eq!(output.bytes, retry, "step {failed}, flush {flush}");

                output.bytes.clear();
                modes.restore(&mut output).unwrap();
                assert!(output.bytes.is_empty());
            }
        }
    }

    #[test]
    #[cfg(windows)]
    fn failed_windows_mouse_cleanup_never_retries_the_raw_console_snapshot() {
        let modes = TerminalModes::default();
        modes.mouse.store(true, Ordering::Relaxed);
        // Crossterm flushes before its Windows API call. Fail there without touching a console.
        let mut output = FailOnce {
            flush_remaining: Some(0),
            ..FailOnce::default()
        };

        assert_eq!(
            modes.restore(&mut output).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );

        modes.restore(&mut output).unwrap();
        assert!(output.bytes.is_empty());
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
    fn duration_and_help_hint_come_from_config_or_defaults() {
        let defaults = resolve_defaults(&[]);

        assert_eq!(
            defaults.status_message_duration,
            std::time::Duration::from_millis(3560)
        );
        assert!(defaults.show_help_hint);

        let text = "[display]\nstatus_message_duration_ms = 1500\nshow_help_hint = false";
        let config = toml::from_str::<config::Config>(text).unwrap();
        let options = parse_cli(&["--no-wrap"]).unwrap().resolve(config);

        assert_eq!(
            options.status_message_duration,
            std::time::Duration::from_millis(1500)
        );
        assert!(!options.show_help_hint);
        assert!(!options.wrap);

        for flag in [
            "--status-message-duration-ms",
            "--show-help-hint",
            "--no-help-hint",
        ] {
            assert!(parse_cli(&[flag]).is_err(), "accepted {flag}");
        }
    }

    #[test]
    fn wrapping_is_enabled_by_default_and_can_be_disabled() {
        let defaults = resolve_defaults(&[]);
        assert!(defaults.wrap);
        assert_eq!(defaults.border_style, config::BorderStyle::Square);
        assert_eq!(defaults.marker_style, config::MarkerStyle::Unicode);
        assert!(defaults.status_messages);
        assert_eq!(defaults.styles.accent.fg, Some(styles::DEFAULT_ACCENT));

        let options = resolve_defaults(&["--marker-style", "ascii", "--no-wrap"]);

        assert_eq!(options.marker_style, config::MarkerStyle::Ascii);
        assert!(!options.wrap);

        for removed in ["--keybinds", "--no-keybinds", "--unknown"] {
            assert!(parse_cli(&[removed]).is_err());
        }

        let options = resolve_defaults(&["--border-style", "rounded"]);

        assert_eq!(options.border_style, config::BorderStyle::Rounded);
        assert!(options.wrap);
    }

    #[test]
    fn help_and_version_requests_stop_parsing_before_startup() {
        use clap::error::ErrorKind;

        for (args, kind) in [
            (vec!["--help"], ErrorKind::DisplayHelp),
            (vec!["-h"], ErrorKind::DisplayHelp),
            (vec!["export", "--help"], ErrorKind::DisplayHelp),
            (vec!["--version"], ErrorKind::DisplayVersion),
            (vec!["--no-wrap", "--version"], ErrorKind::DisplayVersion),
        ] {
            assert_eq!(parse_cli(&args).unwrap_err().kind(), kind, "{args:?}");
        }
    }

    #[test]
    fn export_is_a_subcommand_that_takes_no_display_options() {
        let cli = Cli::try_parse_from(["sati", "export"]).unwrap();
        assert_eq!(cli.command, Some(CliCommand::Export));

        assert!(Cli::try_parse_from(["sati"]).unwrap().command.is_none());

        for args in [
            vec!["sati", "--no-wrap", "export"],
            vec!["sati", "export", "--no-wrap"],
            vec!["sati", "export", "unexpected"],
            vec!["sati", "help"],
        ] {
            assert!(Cli::try_parse_from(&args).is_err(), "accepted {args:?}");
        }
    }

    #[test]
    fn cli_definition_passes_clap_consistency_checks() {
        use clap::CommandFactory;

        Cli::command().debug_assert();
    }

    #[test]
    fn accent_accepts_hex_colors_and_combines_with_no_wrap() {
        for values in [
            ["--accent", "#a9B665", "--no-wrap"],
            ["--no-wrap", "--accent", "#A9b665"],
        ] {
            let options = resolve_defaults(&values);
            assert!(!options.wrap);
            assert_eq!(
                options.styles.accent.fg,
                Some(ratatui::style::Color::Rgb(169, 182, 101))
            );
        }

        for color in ["#000000", "#ffffff"] {
            assert!(parse_accent(color).is_ok());
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
            assert_eq!(parse_accent(text).unwrap(), expected);
        }
    }

    #[test]
    fn accent_rejects_missing_or_invalid_values() {
        assert!(parse_cli(&["--accent"]).is_err());

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
            assert!(parse_cli(&["--accent", color]).is_err(), "accepted {color}");
        }
    }
}
