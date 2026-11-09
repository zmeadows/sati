//! Shared interactive scenarios over the native Unix PTY and Windows ConPTY backends.
//! Every session uses isolated data and configuration, never the invoking terminal or console.
#![cfg(any(target_os = "linux", target_os = "macos", windows))]

use std::{
    fs::{self, File},
    path::PathBuf,
    process::ExitStatus,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "interactive/unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "interactive/windows.rs"]
mod platform;

use platform::Terminal;

const COLUMNS: u16 = 100;
const ROWS: u16 = 30;
// Generous for both an unanswered keyboard probe and ConPTY startup on busy CI runners.
const TIMEOUT: Duration = Duration::from_secs(20);
// How soon a wait notices that Sati has exited; arriving output ends a wait immediately.
const POLL_INTERVAL: Duration = Duration::from_millis(10);
// A session writes tens of kilobytes; more than this is runaway output.
const MAX_OUTPUT: usize = 1 << 20;
const DIAGNOSTIC_BYTES: usize = 2048;

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
fn session_moves_the_selection_and_exits_cleanly() {
    let mut session = Session::start("", &[]);
    session.wait_for_first_frame();

    // ConPTY turns each character into a press and a release; Sati ignores releases.
    session.send("jq");
    session.wait_for_successful_exit();
    session.verify_saved_checklist();
}

#[test]
fn failed_task_save_restores_the_terminal() {
    let mut session = Session::start("", &[]);
    session.wait_for_first_frame();

    // A directory at the destination makes atomic replacement fail on every platform,
    // including when tests run with permission to write read-only files.
    let data = session.directory.0.join("data");
    let original = fs::read(data.join("tasks.json")).unwrap();
    fs::rename(data.join("tasks.json"), data.join("original.json")).unwrap();
    fs::create_dir(data.join("tasks.json")).unwrap();
    session.send(" ");

    if session.wait_for_exit().success() {
        session.fail("a failed save should report failure after restoring the terminal");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    session.verify_error_cleanup();

    assert_eq!(fs::read(data.join("original.json")).unwrap(), original);
    let lock = File::open(data.join("tasks.lock")).unwrap();
    assert!(lock.try_lock().is_ok(), "Sati should release tasks.lock");
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

impl TemporaryDirectory {
    fn new(config: &str) -> Self {
        let directory = Self(std::env::temp_dir().join(format!(
            "sati-interactive-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        )));
        let root = &directory.0;
        fs::create_dir(root).unwrap();
        // These match the isolated environment set by each native spawn function.
        let config_directory = if cfg!(windows) {
            root.join("appdata/sati")
        } else if cfg!(target_os = "macos") {
            root.join("home/Library/Application Support/sati")
        } else {
            root.join("config/sati")
        };
        fs::create_dir_all(&config_directory).unwrap();
        fs::write(config_directory.join("config.toml"), config).unwrap();
        fs::create_dir(root.join("data")).unwrap();
        fs::write(root.join("data/tasks.json"), task_document(0).to_string()).unwrap();
        directory
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Session {
    // Field order matters: native Drop kills/reaps Sati and closes the terminal before the
    // receiver and temporary directory disappear. ConPTY's reader keeps draining on shutdown.
    terminal: Terminal,
    chunks: Receiver<Vec<u8>>,
    output: Vec<u8>,
    directory: TemporaryDirectory,
}

impl Session {
    fn start(config: &str, arguments: &[&str]) -> Self {
        let directory = TemporaryDirectory::new(config);
        let (terminal, chunks) = Terminal::start(&directory.0, arguments);
        Self {
            terminal,
            chunks,
            output: Vec::new(),
            directory,
        }
    }

    fn send(&mut self, input: &str) {
        self.terminal.send(input);
    }

    fn wait_for_first_frame(&mut self) {
        self.wait_for("the first frame", |output| {
            TASKS
                .iter()
                .all(|(_, text, _)| find(output, text, 0).is_some())
        });
    }

    // Waits up to `timeout` for output; false once the native reader has stopped.
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
            if self.terminal.exit_status().unwrap().is_some() {
                self.fail(&format!("Sati exited before {milestone}"));
            }

            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                self.fail(&format!("timed out waiting for {milestone}"));
            };

            if !self.receive(remaining.min(POLL_INTERVAL)) {
                self.fail(&format!("the terminal output closed before {milestone}"));
            }
        }
    }

    fn wait_for_successful_exit(&mut self) {
        let status = self.wait_for_exit();
        // Sati also reports terminal restoration failures through its exit status.
        if !status.success() {
            self.fail(&format!("Sati exited with {status}"));
        }
    }

    fn wait_for_exit(&mut self) -> ExitStatus {
        let deadline = Instant::now() + TIMEOUT;

        loop {
            if let Some(status) = self.terminal.exit_status().unwrap() {
                return status;
            }

            if Instant::now() >= deadline {
                self.fail("timed out waiting for Sati to exit");
            }

            if !self.receive(POLL_INTERVAL) {
                // macOS can hang up the PTY just before Sati can be reaped. Keep polling,
                // with the original deadline, rather than spinning on the closed receiver.
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                std::thread::sleep(POLL_INTERVAL);
                #[cfg(windows)]
                self.fail("the console output closed while Sati was running");
            }
        }
    }

    fn verify_saved_checklist(&self) {
        let data = self.directory.0.join("data");
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
    }

    fn fail(&mut self, problem: &str) -> ! {
        // Include output that has already arrived, such as an error message from Sati.
        while self.output.len() <= MAX_OUTPUT
            && let Ok(chunk) = self.chunks.try_recv()
        {
            self.output.extend_from_slice(&chunk);
        }

        let status = match self.terminal.exit_status() {
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

fn find(output: &[u8], sequence: &str, from: usize) -> Option<usize> {
    output[from..]
        .windows(sequence.len())
        .position(|window| window == sequence.as_bytes())
        .map(|offset| from + offset)
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
