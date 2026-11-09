//! Runs the interactive checklist in a pseudo-terminal: startup, the first frame, keyboard
//! input, a clean exit that saves the selection, and terminal restoration.
//!
//! The harness never involves the invoking terminal or real Sati data. Sati runs in its own
//! session with the PTY as its controlling terminal, a cleared environment, and temporary
//! configuration and data directories. Linux and macOS only; Windows has no PTY, so
//! tests/interactive_conpty.rs covers it with a pseudoconsole instead.
#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    os::{
        fd::{AsFd, OwnedFd},
        unix::process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

use rustix::{
    fs::{Mode, OFlags},
    termios::{ControlModes, InputModes, LocalModes, OutputModes, Termios, Winsize},
};

const COLUMNS: u16 = 100;
const ROWS: u16 = 30;

// Generous, since an unanswered keyboard probe alone takes crossterm two seconds.
const TIMEOUT: Duration = Duration::from_secs(20);
// How soon a wait notices that Sati has exited; arriving output ends a wait immediately.
const POLL_INTERVAL: Duration = Duration::from_millis(10);
// A session writes tens of kilobytes; more than this is runaway output.
const MAX_OUTPUT: usize = 1 << 20;
const DIAGNOSTIC_BYTES: usize = 2048;

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

// Stable identities, and text that can only reach the terminal in a rendered frame. Each text
// is one word because ratatui skips unchanged blank cells, splitting words apart in the output.
#[rustfmt::skip]
const TASKS: [(&str, &str, &str); 3] = [
    ("00000000-0000-4000-8000-000000000001", "Groceries", "pending"),
    ("00000000-0000-4000-8000-000000000002", "Laundry", "pending"),
    ("00000000-0000-4000-8000-000000000003", "Passport", "completed"),
];

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

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
    let original = session.initial_settings;

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

    session.wait_for("the first frame", |output| {
        TASKS
            .iter()
            .all(|(_, text, _)| find(output, text, 0).is_some())
    });

    let raw = session.current_settings();

    assert!(
        !raw.local.intersects(LocalModes::ICANON | LocalModes::ECHO),
        "Sati should be in raw mode while running: {raw:?}"
    );

    session.send(keys);
    let status = session.wait_for_exit();

    if !status.success() {
        session.fail(&format!("Sati exited with {status}"));
    }

    // The test's PTY outlives Sati, as a real terminal does.
    assert_eq!(
        session.current_settings(),
        original,
        "Sati should restore the terminal modes it found"
    );

    session.read_remaining_output();

    let data = session.data_directory();
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(data.join("tasks.json")).unwrap()).unwrap();

    // Only the selection changes; tasks, identities, statuses, and order are untouched.
    assert_eq!(saved, task_document(1));

    let lock = File::open(data.join("tasks.lock")).unwrap();

    if let Err(error) = lock.try_lock() {
        panic!("Sati should release tasks.lock on exit: {error:?}");
    }

    let mut entries: Vec<_> = fs::read_dir(&data)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    entries.sort();

    assert_eq!(entries, ["tasks.json", "tasks.lock"]);

    std::mem::take(&mut session.output)
}

fn task_document(selected: usize) -> serde_json::Value {
    let tasks: Vec<_> = TASKS
        .iter()
        .map(|(id, text, status)| {
            serde_json::json!({
                "id": id,
                "created_at": "2026-11-07T19:24:05Z",
                "text": text,
                "status": status,
            })
        })
        .collect();

    serde_json::json!({
        "schema_version": 5,
        "tasks": tasks,
        "selected": selected,
        "section_order": [],
    })
}

struct TemporaryDirectory(PathBuf);

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// One Sati process on its own PTY. Dropping it kills and reaps a process still running, for
// instance after a failed assertion, before the temporary directory is removed.
struct Session {
    child: Child,
    master: File,
    // The test's own slave descriptor keeps the PTY open on Linux after Sati exits, until all
    // output has been read.
    slave: Option<OwnedFd>,
    initial_settings: TerminalSettings,
    chunks: Receiver<Vec<u8>>,
    output: Vec<u8>,
    directory: TemporaryDirectory,
}

impl Session {
    fn start(config: &str, arguments: &[&str]) -> Self {
        let directory = TemporaryDirectory(std::env::temp_dir().join(format!(
            "sati-pty-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        )));
        let root = &directory.0;
        fs::create_dir(root).unwrap();
        // Where Sati looks for config.toml with the environment that `spawn_sati` sets.
        let config_directory = if cfg!(target_os = "macos") {
            root.join("home/Library/Application Support/sati")
        } else {
            root.join("config/sati")
        };
        fs::create_dir_all(&config_directory).unwrap();
        fs::write(config_directory.join("config.toml"), config).unwrap();
        fs::create_dir(root.join("data")).unwrap();
        fs::write(root.join("data/tasks.json"), task_document(0).to_string()).unwrap();

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

        Self {
            child,
            master,
            slave: Some(slave),
            initial_settings,
            chunks,
            output: Vec::new(),
            directory,
        }
    }

    fn data_directory(&self) -> PathBuf {
        self.directory.0.join("data")
    }

    fn send(&mut self, input: &str) {
        self.master.write_all(input.as_bytes()).unwrap();
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

    // Waits up to `timeout` for output; false once the PTY has closed.
    fn receive(&mut self, timeout: Duration) -> bool {
        match self.chunks.recv_timeout(timeout) {
            Ok(chunk) => {
                self.output.extend_from_slice(&chunk);

                if self.output.len() > MAX_OUTPUT {
                    self.fail("Sati wrote unexpectedly much output");
                }

                true
            }
            Err(RecvTimeoutError::Timeout) => true,
            Err(RecvTimeoutError::Disconnected) => false,
        }
    }

    fn wait_for(&mut self, milestone: &str, reached: impl Fn(&[u8]) -> bool) {
        let deadline = Instant::now() + TIMEOUT;

        while !reached(&self.output) {
            if self.child.try_wait().unwrap().is_some() {
                self.fail(&format!("Sati exited before {milestone}"));
            }

            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                self.fail(&format!("timed out waiting for {milestone}"));
            };

            if !self.receive(remaining.min(POLL_INTERVAL)) {
                self.fail(&format!("the PTY closed before {milestone}"));
            }
        }
    }

    fn wait_for_exit(&mut self) -> ExitStatus {
        let deadline = Instant::now() + TIMEOUT;

        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }

            if Instant::now() >= deadline {
                self.fail("timed out waiting for Sati to exit");
            }

            // The macOS hangup as Sati exits can close the PTY just before Sati can be reaped.
            if !self.receive(POLL_INTERVAL) {
                thread::sleep(POLL_INTERVAL);
            }
        }
    }

    // Closes the test's slave descriptor, then collects output until the PTY reports its end,
    // which it does only after everything Sati wrote. On macOS, that end may already have come.
    fn read_remaining_output(&mut self) {
        drop(self.slave.take());
        let deadline = Instant::now() + TIMEOUT;

        while self.receive(deadline.saturating_duration_since(Instant::now())) {
            if Instant::now() >= deadline {
                self.fail("the PTY did not close after Sati exited");
            }
        }
    }

    fn fail(&mut self, problem: &str) -> ! {
        // Include output that has already arrived, such as an error message from Sati.
        while self.output.len() <= MAX_OUTPUT
            && let Ok(chunk) = self.chunks.try_recv()
        {
            self.output.extend_from_slice(&chunk);
        }

        let status = match self.child.try_wait() {
            Ok(Some(status)) => status.to_string(),
            Ok(None) => "still running".to_owned(),
            Err(error) => format!("unknown ({error})"),
        };

        panic!(
            "{problem}\nSati status: {status}\n{}",
            excerpt(&self.output)
        );
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Does nothing once the process has been reaped, so an unrelated process is never hit.
        let _ = self.child.kill();
        let _ = self.child.wait();
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

fn find(output: &[u8], sequence: &str, from: usize) -> Option<usize> {
    output[from..]
        .windows(sequence.len())
        .position(|window| window == sequence.as_bytes())
        .map(|offset| from + offset)
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

// The end of the output, escaped, for failure messages.
fn excerpt(output: &[u8]) -> String {
    let start = output.len().saturating_sub(DIAGNOSTIC_BYTES);

    format!(
        "last {} of {} output bytes:\n{}",
        output.len() - start,
        output.len(),
        output[start..].escape_ascii()
    )
}
