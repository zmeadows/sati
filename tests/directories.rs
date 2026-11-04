use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sati-directories-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sati"));
    command
        .env_remove("XDG_DATA_HOME")
        .env_remove("HOME")
        .env_remove("APPDATA")
        .stdin(Stdio::null());
    command
}

fn assert_reuses_data(command: &mut Command, data: &Path) {
    fs::create_dir_all(data).unwrap();
    let tasks = data.join("tasks.json");
    fs::write(&tasks, "invalid json").unwrap();
    let output = command.output().unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("invalid task file"), "{error}");
    assert!(error.contains(&tasks.display().to_string()), "{error}");
    assert_eq!(fs::read_to_string(tasks).unwrap(), "invalid json");
    assert!(data.join("tasks.lock").is_file());
    assert_eq!(fs::read_dir(data).unwrap().count(), 2);
}

fn assert_discovery_error(output: Output, message: &str) {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains(message), "{error}");
}

#[test]
fn native_directory_reuses_existing_data() {
    let fixture = Fixture::new();
    let mut command = command();
    command.env("HOME", &fixture.0).env("APPDATA", &fixture.0);
    if cfg!(any(target_os = "macos", windows)) {
        command.env("XDG_DATA_HOME", "ignored-relative-xdg");
    }
    let data = if cfg!(target_os = "macos") {
        fixture.0.join("Library/Application Support/sati")
    } else if cfg!(windows) {
        fixture.0.join("sati")
    } else {
        fixture.0.join(".local/share/sati")
    };
    assert_reuses_data(&mut command, &data);
}

#[test]
fn missing_empty_or_relative_native_directory_is_an_error() {
    let variable = if cfg!(windows) { "APPDATA" } else { "HOME" };
    let missing = if cfg!(windows) {
        "APPDATA is not set"
    } else if cfg!(target_os = "macos") {
        "HOME is not set"
    } else {
        "neither XDG_DATA_HOME nor HOME is set"
    };
    assert_discovery_error(command().output().unwrap(), missing);
    assert_discovery_error(command().env(variable, "").output().unwrap(), missing);
    assert_discovery_error(
        command().env(variable, "relative-home").output().unwrap(),
        "must be an absolute path",
    );
}

#[test]
#[cfg(target_os = "linux")]
fn xdg_directory_takes_precedence_and_empty_xdg_uses_home() {
    for xdg in [None, Some("")] {
        let fixture = Fixture::new();
        let mut command = command();
        command.env("HOME", &fixture.0);
        if let Some(value) = xdg {
            command.env("XDG_DATA_HOME", value);
        }
        assert_reuses_data(&mut command, &fixture.0.join(".local/share/sati"));
    }
    let fixture = Fixture::new();
    let home = fixture.0.join("home");
    assert_reuses_data(
        command()
            .env("XDG_DATA_HOME", &fixture.0)
            .env("HOME", &home),
        &fixture.0.join("sati"),
    );
    assert!(!home.exists());
    // XDG also works without HOME, and relative XDG never falls back to HOME.
    assert_reuses_data(
        command().env("XDG_DATA_HOME", &fixture.0),
        &fixture.0.join("sati"),
    );
    assert_discovery_error(
        command()
            .env("XDG_DATA_HOME", "relative-data")
            .env("HOME", &home)
            .output()
            .unwrap(),
        "XDG_DATA_HOME must be an absolute path",
    );
    assert!(!home.exists());
}
