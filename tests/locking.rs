use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[test]
fn second_process_cannot_read_tasks_while_the_directory_is_locked() {
    // The same test executable supplies a real lock-owning child process.
    if let Some(data) = std::env::var_os("SATI_TEST_LOCK_HOLDER") {
        let data = Path::new(&data);
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(data.join("tasks.lock"))
            .unwrap();
        fs4::FileExt::try_lock(&held).unwrap();
        fs::write(data.join("ready"), "").unwrap();
        std::io::stdin().read_line(&mut String::new()).unwrap();
        return;
    }
    for terminate in [false, true] {
        check_process_lock(terminate);
    }
}

fn check_process_lock(terminate: bool) {
    let directory = std::env::temp_dir().join(format!(
        "sati-process-lock-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let data = if cfg!(target_os = "macos") {
        directory.join("Library/Application Support/sati")
    } else {
        directory.join("sati")
    };
    fs::create_dir_all(&data).unwrap();
    // Invalid data distinguishes lock contention from reaching the load step.
    fs::write(data.join("tasks.json"), "invalid json").unwrap();
    let mut holder = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "second_process_cannot_read_tasks_while_the_directory_is_locked",
        ])
        .env("SATI_TEST_LOCK_HOLDER", &data)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !data.join("ready").exists() {
        assert!(holder.try_wait().unwrap().is_none(), "lock holder exited");
        if Instant::now() >= deadline {
            holder.kill().unwrap();
            holder.wait().unwrap();
            panic!("lock holder did not acquire the lock promptly");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let blocked = run_sati(&directory);
    assert_eq!(
        fs::read_to_string(data.join("tasks.json")).unwrap(),
        "invalid json"
    );
    if terminate {
        holder.kill().unwrap();
    } else {
        holder.stdin.take().unwrap().write_all(b"\n").unwrap();
    }
    let status = holder.wait().unwrap();
    if !terminate {
        assert!(status.success());
    }
    assert!(data.join("tasks.lock").is_file());
    let unlocked = run_sati(&directory);
    let contents = fs::read_to_string(data.join("tasks.json")).unwrap();
    fs::remove_dir_all(&directory).unwrap();

    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(blocked.stderr, b"sati: already running\n");
    assert!(blocked.stdout.is_empty());
    assert_eq!(unlocked.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unlocked.stderr).contains("invalid task file"));
    assert_eq!(contents, "invalid json");
}

fn run_sati(data_home: &Path) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sati"))
        .env("XDG_DATA_HOME", data_home)
        .env("HOME", data_home)
        .env("APPDATA", data_home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "sati did not exit promptly: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}
