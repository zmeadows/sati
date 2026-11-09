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
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("HOME")
        .env_remove("APPDATA")
        .stdin(Stdio::null());

    command
}

// The platform data directory under a HOME (or APPDATA) value.
fn native_data(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/sati")
    } else if cfg!(windows) {
        home.join("sati")
    } else {
        home.join(".local/share/sati")
    }
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

    assert_reuses_data(&mut command, &native_data(&fixture.0));
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

    // An explicit configuration skips configuration discovery, which would otherwise reject
    // the relative directory first. If storage accepted it, the invalid task file at the
    // relative location would stop the run before terminal setup.
    let fixture = Fixture::new();
    let config = fixture.0.join("config.toml");
    fs::write(&config, "").unwrap();
    let relative_data = native_data(&fixture.0.join("relative-home"));
    fs::create_dir_all(&relative_data).unwrap();
    fs::write(relative_data.join("tasks.json"), "invalid json").unwrap();

    let output = command()
        .current_dir(&fixture.0)
        .env(variable, "relative-home")
        .arg("--config")
        .arg(&config)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "sati: data directory must be an absolute path\n"
    );
    assert!(!relative_data.join("tasks.lock").exists());
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

#[test]
fn data_dir_replaces_native_discovery_without_a_sati_subdirectory() {
    let fixture = Fixture::new();
    let work = fixture.0.join("checklists/work");
    let mut command = command();
    command
        .env("HOME", &fixture.0)
        .env("APPDATA", &fixture.0)
        .arg("--data-dir")
        .arg(&work);

    assert_reuses_data(&mut command, &work);

    // The native data directory is never created or read.
    let entries: Vec<_> = fs::read_dir(&fixture.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();

    assert_eq!(entries, ["checklists"]);
}

#[test]
fn data_dir_needs_no_native_directory_and_may_be_relative() {
    let fixture = Fixture::new();
    let absolute = fixture.0.join("absolute");

    assert_reuses_data(command().arg("--data-dir").arg(&absolute), &absolute);

    // A relative directory is resolved against the working directory and reported as given.
    let relative = fixture.0.join("relative");
    fs::create_dir(&relative).unwrap();
    fs::write(relative.join("tasks.json"), "invalid json").unwrap();

    let output = command()
        .current_dir(&fixture.0)
        .args(["--data-dir", "relative"])
        .output()
        .unwrap();
    let shown = Path::new("relative").join("tasks.json");

    assert_discovery_error(output, &format!("invalid task file {}", shown.display()));
    assert!(relative.join("tasks.lock").is_file());
}

#[test]
fn data_dir_that_is_a_regular_file_is_reported_and_left_alone() {
    let fixture = Fixture::new();
    let file = fixture.0.join("not-a-directory");
    fs::write(&file, "keep me").unwrap();

    let output = command().arg("--data-dir").arg(&file).output().unwrap();

    assert_discovery_error(output, "is not a directory");
    assert_eq!(fs::read_to_string(&file).unwrap(), "keep me");

    let output = command()
        .arg("--data-dir")
        .arg(file.join("child"))
        .output()
        .unwrap();

    assert_discovery_error(output, "could not create");
    assert_eq!(fs::read_to_string(&file).unwrap(), "keep me");
}

#[test]
fn separate_data_dirs_have_independent_locks() {
    let fixture = Fixture::new();
    let personal = fixture.0.join("personal");
    let work = fixture.0.join("work");

    // Invalid task data stops each run after locking, before terminal setup.
    for directory in [&personal, &work] {
        fs::create_dir(directory).unwrap();
        fs::write(directory.join("tasks.json"), "invalid json").unwrap();
    }

    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(personal.join("tasks.lock"))
        .unwrap();
    held.try_lock().unwrap();

    let blocked = command().arg("--data-dir").arg(&personal).output().unwrap();
    let independent = command().arg("--data-dir").arg(&work).output().unwrap();

    let expected = format!(
        "sati: already running: another sati holds {}\n",
        personal.join("tasks.lock").display()
    );
    assert_eq!(String::from_utf8_lossy(&blocked.stderr), expected);
    assert_discovery_error(independent, "invalid task file");
    assert!(work.join("tasks.lock").is_file());
}
