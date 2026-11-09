//! Native ConPTY ownership, process handling, and output draining.
//!
//! ConPTY re-renders Sati's output, so shared scenarios check text rather than Unix escape
//! sequences or console modes. A failed restoration is still reported by Sati's exit status.
use std::{
    ffi::{OsStr, OsString, c_void},
    fs::File,
    io::{self, Read, Write},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::ExitStatusExt,
    },
    path::Path,
    process::ExitStatus,
    ptr,
    sync::mpsc::{self, Receiver},
    thread,
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

use super::{COLUMNS, ROWS, TIMEOUT};

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
// instance after a failed assertion, then closes the pseudoconsole before fixture cleanup.
pub(super) struct Terminal {
    process: OwnedHandle,
    // Held only to be closed, after `process` has ended.
    _console: PseudoConsole,
    input: File,
}

impl Terminal {
    pub(super) fn start(root: &Path, arguments: &[&str]) -> (Self, Receiver<Vec<u8>>) {
        // ConPTY reads keys from one pipe and writes its rendering of the screen to the other.
        let (input_read, input_write) = pipe();
        let (output_read, output_write) = pipe();
        let size = COORD {
            X: COLUMNS as i16,
            Y: ROWS as i16,
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

        let process = spawn_sati(root, arguments, &console);

        // The pseudoconsole holds its own copies. Without ours, the output pipe reports its end
        // once the pseudoconsole closes.
        drop(input_read);
        drop(output_write);

        (
            Self {
                process,
                _console: console,
                input: File::from(input_write),
            },
            chunks,
        )
    }

    pub(super) fn send(&mut self, input: &str) {
        self.input.write_all(input.as_bytes()).unwrap();
    }

    // Sati's exit code once it has exited.
    pub(super) fn exit_status(&mut self) -> io::Result<Option<ExitStatus>> {
        let process = self.process.as_raw_handle();

        // SAFETY: `process` is an open process handle; a zero timeout only checks its state.
        match unsafe { WaitForSingleObject(process, 0) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => return Ok(None),
            _ => return Err(io::Error::last_os_error()),
        }

        let mut code = 0;
        // SAFETY: `process` is an open process handle and `code` is valid for writes.
        if unsafe { GetExitCodeProcess(process, &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Some(ExitStatus::from_raw(code)))
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let process = self.process.as_raw_handle();

        // SAFETY: `process` is an open process handle. It refers to this process even after it
        // has exited, when terminating it simply fails, so no other process can be affected.
        unsafe {
            TerminateProcess(process, 1);
            WaitForSingleObject(process, TIMEOUT.as_millis() as u32);
        }

        // The fields then close the pseudoconsole and pipes, before Session removes its directory.
    }
}

// Starts Sati attached to `console`. Its data directory and a minimal environment keep the
// user's configuration and checklist out of reach.
fn spawn_sati(root: &Path, arguments: &[&str], console: &PseudoConsole) -> OwnedHandle {
    // Windows paths cannot contain quotation marks, so quoting each one is enough.
    let mut command_line = OsString::from("\"");
    command_line.push(env!("CARGO_BIN_EXE_sati"));
    command_line.push("\"");
    for argument in arguments {
        // These are literal test switches, not arbitrary command-line strings.
        assert!(!argument.contains(['"', ' ', '\t']));
        command_line.push(" ");
        command_line.push(argument);
    }
    command_line.push(" --data-dir \"");
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
