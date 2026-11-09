mod app;
mod config;
mod export;
mod input;
mod model;
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
    Args, CommandFactory, FromArgMatches, Parser, Subcommand,
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
const INPUT_HEADING: &str = "Input";

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
    mouse: bool,
}

#[derive(Debug, Parser)]
#[command(
    version,
    about,
    styles = HELP_STYLES,
    after_help = CONFIG_HELP,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,

    /// Use the checklist in this directory instead of the default data directory
    ///
    /// The directory holds tasks.json and tasks.lock; each directory is an independent
    /// checklist. sati creates the directory if needed; export only reads it. It does not
    /// change where config.toml is read from.
    #[arg(long, value_name = "PATH", global = true)]
    data_dir: Option<PathBuf>,

    #[command(flatten)]
    options: CliOptions,
}

impl Cli {
    // Like `try_parse_from`, but a subcommand rejects the interactive options. Clap's
    // `args_conflicts_with_subcommands` would also reject the global `--data-dir` before a
    // subcommand, so the conflict is checked here, and reported as Clap's own usage error.
    fn try_parse_arguments<I, T>(arguments: I) -> std::result::Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let mut command = Self::command();
        let matches = command.try_get_matches_from_mut(arguments)?;

        if let Some((subcommand, _)) = matches.subcommand() {
            let interactive_option = command.get_arguments().find(|argument| {
                !argument.is_global_set()
                    && matches.value_source(argument.get_id().as_str())
                        == Some(clap::parser::ValueSource::CommandLine)
            });

            if let Some(option) = interactive_option {
                let message = format!(
                    "the subcommand '{subcommand}' cannot be used with '--{}'",
                    option.get_long().unwrap_or_default()
                );

                return Err(command.error(clap::error::ErrorKind::ArgumentConflict, message));
            }
        }

        Self::from_arg_matches(&matches).map_err(|error| error.format(&mut command))
    }
}

// Each subcommand runs instead of the interactive checklist.
#[derive(Debug, PartialEq, Subcommand)]
enum CliCommand {
    /// Print the checklist, or one section of it, as Markdown
    ///
    /// Writes every task, in list order, to stdout as a Markdown checklist. With a SECTION,
    /// writes only that section's tasks, without their "Name: " prefix. It works while sati
    /// is open and never changes the task file.
    Export {
        /// Export only this section; names match regardless of case
        #[arg(value_name = "SECTION")]
        section: Option<String>,

        /// Export only the tasks without a section
        #[arg(long, conflicts_with = "section")]
        unsectioned: bool,
    },
}

// Each on/off pair overrides the other, so the last flag given wins.
#[derive(Debug, Args)]
struct CliOptions {
    /// Read settings from this file instead of the default config.toml
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Use a built-in color theme; see --list-themes
    #[arg(
        long,
        value_name = "NAME",
        value_parser = theme_parser(),
        hide_possible_values = true,
        help_heading = DISPLAY_HEADING
    )]
    theme: Option<themes::BuiltinTheme>,

    /// List the built-in color themes and exit
    #[arg(long, help_heading = DISPLAY_HEADING)]
    list_themes: bool,

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

    /// Capture the mouse to click tasks and tabs and scroll with the wheel (default)
    #[arg(long, overrides_with = "no_mouse", help_heading = INPUT_HEADING)]
    mouse: bool,

    /// Leave the mouse to the terminal, e.g. for selecting text; keys work as usual
    #[arg(long, overrides_with = "mouse", help_heading = INPUT_HEADING)]
    no_mouse: bool,
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
            mouse: chosen_flag(self.mouse, self.no_mouse)
                .or(config.input.mouse)
                .unwrap_or(true),
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

// Listing the names lets clap suggest close matches for typos; help omits them for brevity.
fn theme_parser() -> impl TypedValueParser<Value = themes::BuiltinTheme> {
    let names = themes::BuiltinTheme::ALL.iter().map(|(_, name)| *name);

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
    // configuration, storage, or the terminal are touched; --list-themes returns early too.
    let cli = Cli::try_parse_arguments(std::env::args_os()).unwrap_or_else(|error| error.exit());

    let data_dir = cli.data_dir.as_deref();

    if let Some(command) = cli.command {
        return match command {
            CliCommand::Export {
                section,
                unsectioned,
            } => export::run(data_dir, section.as_deref(), unsectioned),
        };
    }

    if cli.options.list_themes {
        return themes::print_list();
    }

    // Configuration is read once, before storage and the terminal are touched, so an
    // invalid file is reported on stderr and changes made later need a restart.
    let config = config::Config::load(cli.options.config.as_deref())?;
    let options = cli.options.resolve(config);

    let storage = Storage::discover(data_dir)?;
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
        // Setup errors are bare OS errors (such as "No such device or address" without a
        // terminal), so label them as terminal setup failures.
        terminal::enable_raw_mode().map_err(terminal_setup_error)?;
        modes.raw.store(true, Ordering::Relaxed);
        modes.cursor.store(true, Ordering::Relaxed);
        // An output failure may occur after the screen-switch command was sent.
        modes.alternate.store(true, Ordering::Relaxed);
        execute!(io::stdout(), EnterAlternateScreen).map_err(terminal_setup_error)?;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))
                .map_err(terminal_setup_error)?;

        // The capability probe retries reader errors internally; validate input first.
        event::poll(std::time::Duration::ZERO).map_err(terminal_setup_error)?;
        let enhanced = keyboard_support(terminal::supports_keyboard_enhancement())
            .map_err(terminal_setup_error)?;
        modes
            .enable_input(&mut io::stdout(), enhanced, options.mouse)
            .map_err(terminal_setup_error)?;

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

fn terminal_setup_error(error: io::Error) -> Box<dyn std::error::Error> {
    format!("could not set up the terminal: {error}").into()
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

    fn enable_input(
        &self,
        output: &mut impl io::Write,
        enhanced: bool,
        mouse: bool,
    ) -> io::Result<()> {
        // With mouse capture off, `self.mouse` stays false, so cleanup never disables it either.
        if mouse {
            enable_optional(output, &self.mouse, EnableMouseCapture)?;
        }

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
#[path = "main/tests.rs"]
mod tests;
