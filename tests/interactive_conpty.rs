//! Runs the interactive checklist in a Windows pseudoconsole (ConPTY): startup, the first frame,
//! keyboard input, and a clean exit that saves the selection and releases the lock.
//!
//! ConPTY is a console of its own: it interprets what Sati writes and sends its own rendering of
//! the screen. Unlike tests/interactive_pty.rs, this cannot observe Sati's escape sequences or
//! the console modes it restores, so it checks rendered text only; Sati still reports a failed
//! restoration through its exit status. Windows Terminal, paste, and the clipboard are out of
//! scope.
//!
//! The harness never involves the invoking console or real Sati data. Sati is attached only to
//! the pseudoconsole and runs with a minimal environment and temporary data and configuration
//! directories.
#![cfg(windows)]

use std::{
    ffi::{OsStr, OsString, c_void},
    fs::{self, File},
    io::{self, Read, Write},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    ptr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

use windows_sys::{
    Win32::{
        Foundation::{INVALID_HANDLE_VALUE, S_OK, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::{
            Console::{COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON},
            Pipes::CreatePipe,
            Threading::{
                CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
                EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
                InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
                UpdateProcThreadAttribute, WaitForSingleObject,
            },
        },
    },
    core::BOOL,
};

const COLUMNS: i16 = 100;
const ROWS: i16 = 30;

// Generous, since starting the pseudoconsole's host can be slow on a busy CI runner.
const TIMEOUT: Duration = Duration::from_secs(20);
// How soon a wait notices that Sati has exited; arriving output ends a wait immediately.
const POLL_INTERVAL: Duration = Duration::from_millis(10);
// A session renders tens of kilobytes; more than this is runaway output.
const MAX_OUTPUT: usize = 1 << 20;
const DIAGNOSTIC_BYTES: usize = 2048;

// Stable identities, and one-word texts that appear in the output only once a frame is rendered.
#[rustfmt::skip]
const TASKS: [(&str, &str, &str); 3] = [
    ("00000000-0000-4000-8000-000000000001", "Groceries", "pending"),
    ("00000000-0000-4000-8000-000000000002", "Laundry", "pending"),
    ("00000000-0000-4000-8000-000000000003", "Passport", "completed"),
];

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

#[test]
fn session_in_a_pseudoconsole_moves_the_selection_and_exits_cleanly() {
    let mut session = Session::start();

    session.wait_for("the first frame", |output| {
        TASKS.iter().all(|(_, text, _)| find(output, text))
    });

    // ConPTY turns each character into a key press and a release; Sati ignores releases.
    session.send("jq");
    let code = session.wait_for_exit();

    // Sati also exits with failure when restoring the console fails.
    if code != 0 {
        session.fail(&format!("Sati exited with code {code}"));
    }

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

// Closing the pseudoconsole ends Sati's connection to it and, once ConPTY has written everything
// out, closes the output pipe.
struct PseudoConsole(HPCON);

impl Drop for PseudoConsole {
    fn drop(&mut self) {
        // SAFETY: the value came from a successful CreatePseudoConsole and is closed only here.
        unsafe { ClosePseudoConsole(self.0) };
    }
}

// One Sati process in its own pseudoconsole. Dropping it ends a process still running, for
// instance after a failed assertion, then closes the pseudoconsole and removes the directory.
struct Session {
    process: OwnedHandle,
    // Held only to be closed, after `process` has ended.
    _console: PseudoConsole,
    input: File,
    chunks: Receiver<Vec<u8>>,
    output: Vec<u8>,
    directory: TemporaryDirectory,
}

impl Session {
    fn start() -> Self {
        let directory = TemporaryDirectory(std::env::temp_dir().join(format!(
            "sati-conpty-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        )));
        let root = &directory.0;
        fs::create_dir(root).unwrap();
        fs::create_dir(root.join("data")).unwrap();
        fs::write(root.join("data/tasks.json"), task_document(0).to_string()).unwrap();

        // ConPTY reads keys from one pipe and writes its rendering of the screen to the other.
        let (input_read, input_write) = pipe();
        let (output_read, output_write) = pipe();
        let size = COORD {
            X: COLUMNS,
            Y: ROWS,
        };
        let mut console = 0;
        // SAFETY: both handles are open pipe ends, which the pseudoconsole duplicates, and
        // `console` is valid for writes.
        let result = unsafe {
            CreatePseudoConsole(
                size,
                input_read.as_raw_handle(),
                output_write.as_raw_handle(),
                0,
                &mut console,
            )
        };
        assert!(
            result == S_OK,
            "could not create a pseudoconsole: HRESULT {:#010x}",
            result as u32
        );
        let console = PseudoConsole(console);

        // Reading starts first so that ConPTY never blocks on a full pipe.
        let (sender, chunks) = mpsc::channel();
        let reader = File::from(output_read);
        thread::spawn(move || read_continuously(reader, sender));

        let process = spawn_sati(root, &console);

        // The pseudoconsole holds its own copies. Without ours, the output pipe reports its end
        // once the pseudoconsole closes.
        drop(input_read);
        drop(output_write);

        Self {
            process,
            _console: console,
            input: File::from(input_write),
            chunks,
            output: Vec::new(),
            directory,
        }
    }

    fn data_directory(&self) -> PathBuf {
        self.directory.0.join("data")
    }

    fn send(&mut self, input: &str) {
        self.input.write_all(input.as_bytes()).unwrap();
    }

    // Sati's exit code once it has exited.
    fn exit_code(&self) -> Option<u32> {
        let process = self.process.as_raw_handle();

        // SAFETY: `process` is an open process handle; a zero timeout only checks its state.
        match unsafe { WaitForSingleObject(process, 0) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => return None,
            _ => panic!(
                "could not check Sati's state: {}",
                io::Error::last_os_error()
            ),
        }

        let mut code = 0;
        // SAFETY: `process` is an open process handle and `code` is valid for writes.
        check(
            unsafe { GetExitCodeProcess(process, &mut code) },
            "read Sati's exit code",
        );

        Some(code)
    }

    // Waits up to `timeout` for output; false once the reader has stopped.
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
            if self.exit_code().is_some() {
                self.fail(&format!("Sati exited before {milestone}"));
            }

            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                self.fail(&format!("timed out waiting for {milestone}"));
            };

            if !self.receive(remaining.min(POLL_INTERVAL)) {
                self.fail(&format!("the console output closed before {milestone}"));
            }
        }
    }

    fn wait_for_exit(&mut self) -> u32 {
        let deadline = Instant::now() + TIMEOUT;

        loop {
            if let Some(code) = self.exit_code() {
                return code;
            }

            if Instant::now() >= deadline {
                self.fail("timed out waiting for Sati to exit");
            }

            if !self.receive(POLL_INTERVAL) {
                self.fail("the console output closed while Sati was running");
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

        let status = match self.exit_code() {
            Some(code) => format!("exited with code {code}"),
            None => "still running".to_owned(),
        };

        panic!(
            "{problem}\nSati status: {status}\n{}",
            excerpt(&self.output)
        );
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let process = self.process.as_raw_handle();

        // SAFETY: `process` is an open process handle. It refers to this process even after it
        // has exited, when terminating it simply fails, so no other process can be affected.
        unsafe {
            TerminateProcess(process, 1);
            WaitForSingleObject(process, TIMEOUT.as_millis() as u32);
        }

        // The fields then close the pseudoconsole and the pipes, and remove the directory.
    }
}

// Starts Sati attached to `console`. Its data directory and a minimal environment keep the
// user's configuration and checklist out of reach.
fn spawn_sati(root: &Path, console: &PseudoConsole) -> OwnedHandle {
    // Windows paths cannot contain quotation marks, so quoting each one is enough.
    let mut command_line = OsString::from("\"");
    command_line.push(env!("CARGO_BIN_EXE_sati"));
    command_line.push("\" --data-dir \"");
    command_line.push(root.join("data"));
    command_line.push("\"");
    let mut command_line = wide(&command_line);

    // NAME=value entries sorted by name, each ending in a nul, then a final nul. APPDATA is where
    // Sati looks for config.toml; Windows itself expects SystemRoot.
    let mut environment = OsString::from("APPDATA=");
    environment.push(root.join("appdata"));
    environment.push("\0");

    if let Some(system_root) = std::env::var_os("SystemRoot") {
        environment.push("SystemRoot=");
        environment.push(system_root);
        environment.push("\0");
    }

    let environment = wide(&environment);
    let current_directory = wide(root.as_os_str());

    let mut size = 0;
    // SAFETY: with no list, the call only reports the size a one-attribute list needs, by
    // failing with ERROR_INSUFFICIENT_BUFFER.
    unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size) };
    // Elements of usize keep the opaque list pointer-aligned.
    let mut list_buffer = vec![0_usize; size.div_ceil(size_of::<usize>())];
    let list = list_buffer.as_mut_ptr().cast();

    // SAFETY: `list` has the size requested above and outlives every use. The pseudoconsole
    // attribute takes the handle value itself, not a pointer to it.
    unsafe {
        check(
            InitializeProcThreadAttributeList(list, 1, 0, &mut size),
            "initialize the process attributes",
        );
        check(
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                console.0 as *const c_void,
                size_of::<HPCON>(),
                ptr::null_mut(),
                ptr::null(),
            ),
            "attach the pseudoconsole",
        );
    }

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.lpAttributeList = list;
    // Otherwise Sati may inherit the test runner's redirected output and write outside the
    // pseudoconsole. With invalid standard handles, it uses the pseudoconsole's instead.
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;

    let mut information = PROCESS_INFORMATION::default();
    // SAFETY: every pointer refers to a live, nul-terminated string or an initialized structure
    // above, and the command line is mutable, as CreateProcessW requires. No handles are
    // inherited.
    let created = unsafe {
        CreateProcessW(
            ptr::null(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            current_directory.as_ptr(),
            ptr::from_ref(&startup).cast(),
            &mut information,
        )
    };
    let error = io::Error::last_os_error();

    // SAFETY: the list was initialized above and is not used again.
    unsafe { DeleteProcThreadAttributeList(list) };

    if created == 0 {
        panic!("could not start Sati: {error}");
    }

    // SAFETY: CreateProcessW returned new handles that nothing else owns. The thread handle is
    // not needed and closes immediately.
    unsafe {
        drop(OwnedHandle::from_raw_handle(information.hThread));
        OwnedHandle::from_raw_handle(information.hProcess)
    }
}

// The read and write ends of a new anonymous pipe; neither is inheritable.
fn pipe() -> (OwnedHandle, OwnedHandle) {
    let mut read = ptr::null_mut();
    let mut write = ptr::null_mut();

    // SAFETY: both pointers are valid for writes. On success, each holds a new handle that
    // nothing else owns.
    unsafe {
        check(
            CreatePipe(&mut read, &mut write, ptr::null(), 0),
            "create a pipe",
        );

        (
            OwnedHandle::from_raw_handle(read),
            OwnedHandle::from_raw_handle(write),
        )
    }
}

fn check(result: BOOL, action: &str) {
    if result == 0 {
        panic!("could not {action}: {}", io::Error::last_os_error());
    }
}

// A nul-terminated UTF-16 copy, as Win32 expects.
fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain([0]).collect()
}

// Forwards console output until the pseudoconsole closes. It keeps draining after the test stops
// listening, since ClosePseudoConsole can wait until all output has been read.
fn read_continuously(mut output: File, chunks: mpsc::Sender<Vec<u8>>) {
    let mut buffer = [0; 4096];

    loop {
        match output.read(&mut buffer) {
            // A pipe whose write ends have all closed reads as end of file.
            Ok(0) => return,
            Ok(count) => {
                let _ = chunks.send(buffer[..count].to_vec());
            }
            Err(error) => panic!("could not read the console output: {error}"),
        }
    }
}

fn find(output: &[u8], text: &str) -> bool {
    output
        .windows(text.len())
        .any(|window| window == text.as_bytes())
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
