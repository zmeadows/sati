use std::{
    fs::{self, File},
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[test]
fn second_process_cannot_read_tasks_while_the_directory_is_locked() {
    let directory = std::env::temp_dir().join(format!(
        "sati-process-lock-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let data = directory.join("sati");
    fs::create_dir_all(&data).unwrap();
    // Invalid data distinguishes lock contention from reaching the load step.
    fs::write(data.join("tasks.json"), "invalid json").unwrap();
    let held = File::create(data.join("tasks.lock")).unwrap();
    rustix::fs::flock(&held, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    let blocked = run_sati(&directory);
    drop(held);
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
