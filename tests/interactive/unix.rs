//! Native PTY ownership, terminal inspection, and deeper Linux/macOS verification.
use std::{
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsFd, OwnedFd},
        unix::process::CommandExt,
    },
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::Instant,
};

use rustix::{
    fs::{Mode, OFlags},
    termios::{ControlModes, InputModes, LocalModes, OutputModes, Termios, Winsize},
};

use super::{COLUMNS, ROWS, Session, TASKS, TIMEOUT, excerpt, find};

// Terminal commands as crossterm 0.29 and `TerminalModes` encode them.
const ENTER_ALTERNATE_SCREEN: &str = "\x1b[?1049h";
const LEAVE_ALTERNATE_SCREEN: &str = "\x1b[?1049l";
const SHOW_CURSOR: &str = "\x1b[?25h";
const ENABLE_PASTE: &str = "\x1b[?2004h";
const DISABLE_PASTE: &str = "\x1b[?2004l";
const ENABLE_MOUSE: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h";
const DISABLE_MOUSE: &str = "\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l";
const KEYBOARD_PROBE: &str = "\x1b[?u\x1b[c";
const CURSOR_POSITION_QUERY: &str = "\x1b[6n";
const PUSH_KEYBOARD: &str = "\x1b[>15u";
const POP_KEYBOARD: &str = "\x1b[<1u";
// The start of OSC 11 with any color.
const SET_BACKGROUND: &str = "\x1b]11;";
const RESET_BACKGROUND: &str = "\x1b]111\x1b\\";

#[test]
fn themed_session_with_mouse_and_keyboard_enhancement_restores_the_terminal() {
    let output = run_session(
        "theme = \"gruvbox-material\"\n",
        &[],
        &[
            // A kitty-protocol terminal reports its current flags, then its primary attributes.
            (KEYBOARD_PROBE, "\x1b[?0u\x1b[?62c"),
            // Ratatui's clear, which redraws for the new background, asks where the cursor is.
            // Any position will do: the frame is redrawn completely with the cursor hidden.
            (CURSOR_POSITION_QUERY, "\x1b[1;1R"),
        ],
        // Kitty-encoded presses of j and q, and the release of j, which Sati must ignore.
        "\x1b[106u\x1b[106;1:3u\x1b[113u",
    );

    assert_in_order(
        &output,
        &[
            ENTER_ALTERNATE_SCREEN,
            KEYBOARD_PROBE,
            ENABLE_MOUSE,
            ENABLE_PASTE,
            PUSH_KEYBOARD,
            // Gruvbox Material's background.
            "\x1b]11;#282828\x1b\\",
            TASKS[0].1,
        ],
    );

    assert_counts(
        &output,
        &[
            (ENTER_ALTERNATE_SCREEN, 1),
            (LEAVE_ALTERNATE_SCREEN, 1),
            (KEYBOARD_PROBE, 1),
            (ENABLE_MOUSE, 1),
            (DISABLE_MOUSE, 1),
            (ENABLE_PASTE, 1),
            (DISABLE_PASTE, 1),
            (PUSH_KEYBOARD, 1),
            (POP_KEYBOARD, 1),
            (SET_BACKGROUND, 1),
            (RESET_BACKGROUND, 1),
        ],
    );

    // Restoration is the last thing written, after the final frame.
    assert_ends_with(
        &output,
        &[
            POP_KEYBOARD,
            DISABLE_PASTE,
            DISABLE_MOUSE,
            SHOW_CURSOR,
            LEAVE_ALTERNATE_SCREEN,
            RESET_BACKGROUND,
        ],
    );
}

#[test]
fn primary_attributes_without_keyboard_enhancement_restores_the_terminal() {
    let output = run_session(
        "",
        &["--no-mouse"],
        // A terminal can reject the kitty query simply by answering primary attributes.
        &[(KEYBOARD_PROBE, "\x1b[?62c")],
        "jq",
    );

    assert_counts(
        &output,
        &[
            (KEYBOARD_PROBE, 1),
            (PUSH_KEYBOARD, 0),
            (POP_KEYBOARD, 0),
            (ENABLE_PASTE, 1),
            (DISABLE_PASTE, 1),
            (ENABLE_MOUSE, 0),
            (DISABLE_MOUSE, 0),
        ],
    );
    assert_ends_with(
        &output,
        &[DISABLE_PASTE, SHOW_CURSOR, LEAVE_ALTERNATE_SCREEN],
    );
}

#[test]
fn plain_session_without_mouse_or_keyboard_enhancement_restores_the_terminal() {
    let output = run_session(
        "",
        &["--no-mouse"],
        // Staying silent lets the keyboard probe time out, which Sati treats as unsupported.
        &[],
        "jq",
    );

    assert_in_order(
        &output,
        &[
            ENTER_ALTERNATE_SCREEN,
            KEYBOARD_PROBE,
            ENABLE_PASTE,
            TASKS[0].1,
        ],
    );

    // Crossterm writes the keyboard probe to /dev/tty, so finding it here also shows that
    // /dev/tty is the PTY. The default theme keeps the terminal's own background, so OSC 11
    // and 111 are never sent.
    assert_counts(
        &output,
        &[
            (ENTER_ALTERNATE_SCREEN, 1),
            (LEAVE_ALTERNATE_SCREEN, 1),
            (KEYBOARD_PROBE, 1),
            (ENABLE_MOUSE, 0),
            (DISABLE_MOUSE, 0),
            (ENABLE_PASTE, 1),
            (DISABLE_PASTE, 1),
            (PUSH_KEYBOARD, 0),
            (POP_KEYBOARD, 0),
            (SET_BACKGROUND, 0),
            (RESET_BACKGROUND, 0),
        ],
    );

    assert_ends_with(
        &output,
        &[DISABLE_PASTE, SHOW_CURSOR, LEAVE_ALTERNATE_SCREEN],
    );
}

// Runs Sati over the fixture checklist with `config` as its default config.toml. Answers each
// terminal query in `answers` in turn, waits for the first frame, then types `keys`, which must
// move down one task and quit. Checks the exit, raw-mode restoration, saved selection, and
// lock, and returns everything Sati wrote to the terminal.
fn run_session(config: &str, arguments: &[&str], answers: &[(&str, &str)], keys: &str) -> Vec<u8> {
    let mut session = Session::start(config, arguments);
    let original = session.terminal.initial_settings;

    assert!(
        original
            .local
            .contains(LocalModes::ICANON | LocalModes::ECHO),
        "a new PTY should start in canonical mode with echo: {original:?}"
    );

    // Raw mode precedes every query, so answers are not echoed. Sati waits for each answer
    // before writing more, so the next query follows all output received so far.
    let mut searched = 0;

    for &(query, answer) in answers {
        session.wait_for(&format!("the query {query:?}"), |output| {
            find(output, query, searched).is_some()
        });
        searched = session.output.len();
        session.send(answer);
    }

    session.wait_for_first_frame();

    let raw = session.terminal.current_settings();

    assert!(
        !raw.local.intersects(LocalModes::ICANON | LocalModes::ECHO),
        "Sati should be in raw mode while running: {raw:?}"
    );

    session.send(keys);
    session.wait_for_successful_exit();

    // The test's PTY outlives Sati, as a real terminal does.
    assert_eq!(
        session.terminal.current_settings(),
        original,
        "Sati should restore the terminal modes it found"
    );

    session.read_remaining_output();

    session.verify_saved_checklist(&super::task_document(1));

    std::mem::take(&mut session.output)
}

// One Sati process on its own PTY. Dropping it kills and reaps a process still running, for
// instance after a failed assertion. It closes before Session removes the temporary directory.
pub(super) struct Terminal {
    child: Child,
    master: File,
    // The test's own slave descriptor keeps the PTY open on Linux after Sati exits, until all
    // output has been read.
    slave: Option<OwnedFd>,
    initial_settings: TerminalSettings,
}

impl Terminal {
    pub(super) fn start(root: &Path, arguments: &[&str]) -> (Self, Receiver<Vec<u8>>) {
        // Every descriptor is close-on-exec, so concurrently spawned processes never keep the
        // PTY open; Sati receives only the standard streams below. Opening /dev/ptmx is what
        // `posix_openpt` does on both platforms, but only `open` takes O_CLOEXEC on macOS.
        let cloexec = OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC;
        let master = rustix::fs::open("/dev/ptmx", cloexec, Mode::empty()).unwrap();
        rustix::pty::grantpt(&master).unwrap();
        rustix::pty::unlockpt(&master).unwrap();
        let slave_path = rustix::pty::ptsname(&master, Vec::new()).unwrap();
        let slave = rustix::fs::open(slave_path.as_c_str(), cloexec, Mode::empty()).unwrap();
        let size = Winsize {
            ws_row: ROWS,
            ws_col: COLUMNS,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        rustix::termios::tcsetwinsize(&slave, size).unwrap();
        // Recorded before Sati starts, since it may switch to raw mode at any moment after.
        let initial_settings = terminal_settings(&slave);

        let master = File::from(master);
        let reader = master.try_clone().unwrap();
        let (sender, chunks) = mpsc::channel();
        thread::spawn(move || read_continuously(reader, sender));

        let child = spawn_sati(root, arguments, &slave);

        (
            Self {
                child,
                master,
                slave: Some(slave),
                initial_settings,
            },
            chunks,
        )
    }

    pub(super) fn send(&mut self, input: &str) {
        self.master.write_all(input.as_bytes()).unwrap();
    }

    pub(super) fn resize(&self, columns: u16, rows: u16) {
        let size = Winsize {
            ws_row: rows,
            ws_col: columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        rustix::termios::tcsetwinsize(&self.master, size).unwrap();
    }

    // Linux keeps separate settings for the master, so they are read through the test's slave
    // descriptor. macOS hangs up the terminal when Sati, its session leader, exits, so that
    // descriptor cannot be relied on afterwards; there, both sides share one set of settings.
    fn current_settings(&self) -> TerminalSettings {
        #[cfg(target_os = "linux")]
        let side = self.slave.as_ref().expect("slave is open").as_fd();
        #[cfg(target_os = "macos")]
        let side = self.master.as_fd();

        terminal_settings(side)
    }

    pub(super) fn exit_status(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Does nothing once the process has been reaped, so an unrelated process is never hit.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Session {
    pub(super) fn verify_error_cleanup(&mut self) {
        assert_eq!(
            self.terminal.current_settings(),
            self.terminal.initial_settings,
            "Sati should restore raw mode after a save error"
        );
        self.read_remaining_output();
        assert_in_order(
            &self.output,
            &[
                ENABLE_MOUSE,
                ENABLE_PASTE,
                TASKS[0].1,
                DISABLE_PASTE,
                DISABLE_MOUSE,
                SHOW_CURSOR,
                LEAVE_ALTERNATE_SCREEN,
                "sati: could not save",
            ],
        );
        assert!(find(&self.output, "could not restore terminal", 0).is_none());
    }

    // Closes the test's slave descriptor, then collects output until the PTY reports its end,
    // which it does only after everything Sati wrote. On macOS, that end may already have come.
    fn read_remaining_output(&mut self) {
        drop(self.terminal.slave.take());
        let deadline = Instant::now() + TIMEOUT;

        while self.receive(deadline.saturating_duration_since(Instant::now())) {
            if Instant::now() >= deadline {
                self.fail("the PTY did not close after Sati exited");
            }
        }
    }
}

// The termios flags that raw mode changes and restoring it must put back.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TerminalSettings {
    input: InputModes,
    output: OutputModes,
    control: ControlModes,
    local: LocalModes,
}

fn terminal_settings(terminal: impl AsFd) -> TerminalSettings {
    let termios: Termios = rustix::termios::tcgetattr(terminal).unwrap();

    TerminalSettings {
        input: termios.input_modes,
        output: termios.output_modes,
        control: termios.control_modes,
        local: termios.local_modes,
    }
}

fn spawn_sati(root: &Path, arguments: &[&str], slave: &OwnedFd) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sati"));
    command
        .args(arguments)
        .arg("--data-dir")
        .arg(root.join("data"))
        .current_dir(root)
        // Only temporary locations, so the user's configuration and checklist are unreachable.
        .env_clear()
        .env("TERM", "xterm-256color")
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data-home"))
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave.try_clone().unwrap()));

    // Coverage tools such as cargo-llvm-cov collect subprocess profiles through this variable.
    // Sati runs in the temporary directory, so a relative path is anchored to ours; joining
    // keeps an absolute path as is, and LLVM still expands patterns such as %p and %m.
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        let profile = std::env::current_dir().unwrap().join(profile);
        command.env("LLVM_PROFILE_FILE", profile);
    }

    // SAFETY: the closure runs between fork and exec, where only async-signal-safe operations
    // are allowed. It makes two system calls and allocates nothing.
    unsafe {
        command.pre_exec(|| {
            // A new session detaches Sati from the terminal running the tests. The PTY then
            // becomes its controlling terminal, so /dev/tty, which crossterm opens for the
            // window size and the keyboard probe, is the PTY as well.
            rustix::process::setsid()?;
            rustix::process::ioctl_tiocsctty(rustix::stdio::stdin())?;

            Ok(())
        });
    }

    // The command's copies of the slave descriptor close when it is dropped on return.
    command.spawn().unwrap()
}

// Forwards PTY output until the PTY closes, so Sati never blocks on a full PTY buffer.
fn read_continuously(mut master: File, chunks: mpsc::Sender<Vec<u8>>) {
    let mut buffer = [0; 4096];

    loop {
        match master.read(&mut buffer) {
            Ok(0) => return,
            Ok(count) => {
                if chunks.send(buffer[..count].to_vec()).is_err() {
                    return;
                }
            }
            // Linux reports EIO, rather than end of file, once no slave descriptor is open;
            // macOS may report either once the terminal is hung up.
            Err(error) if error.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) => {
                return;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => panic!("could not read the PTY: {error}"),
        }
    }
}

fn assert_in_order(output: &[u8], sequences: &[&str]) {
    let mut from = 0;

    for sequence in sequences {
        let Some(start) = find(output, sequence, from) else {
            panic!(
                "{sequence:?} is missing or out of order\n{}",
                excerpt(output)
            );
        };

        from = start + sequence.len();
    }
}

fn assert_counts(output: &[u8], expected: &[(&str, usize)]) {
    for &(sequence, count) in expected {
        let found = output
            .windows(sequence.len())
            .filter(|window| *window == sequence.as_bytes())
            .count();

        assert_eq!(found, count, "occurrences of {sequence:?}");
    }
}

fn assert_ends_with(output: &[u8], sequences: &[&str]) {
    let ending = sequences.concat();

    assert!(
        output.ends_with(ending.as_bytes()),
        "output should end with {ending:?}\n{}",
        excerpt(output)
    );
}
