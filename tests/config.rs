use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let root = std::env::temp_dir().join(format!(
            "sati-config-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));

        fs::create_dir(&root).unwrap();

        let data = if cfg!(target_os = "macos") {
            root.join("Library/Application Support/sati")
        } else {
            root.join("sati")
        };

        Self {
            config: data.join("config.toml"),
            data,
            root,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sati"));
        command
            .current_dir(&self.root)
            .env("HOME", &self.root)
            .env("APPDATA", &self.root)
            .env("XDG_CONFIG_HOME", &self.root)
            .env("XDG_DATA_HOME", &self.root)
            .stdin(Stdio::null());

        command
    }
    fn config(&self, text: &str) {
        fs::create_dir_all(&self.data).unwrap();
        fs::write(&self.config, text).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// Every run here must fail before terminal setup: with stdin redirected, crossterm would
// otherwise take over the developer's terminal. Invalid configuration stops startup before
// storage; with a valid one, an invalid task file stops it before the terminal.
fn write_invalid_tasks(fixture: &Fixture) {
    fs::create_dir_all(&fixture.data).unwrap();
    fs::write(fixture.data.join("tasks.json"), "invalid task data").unwrap();
}

#[test]
fn invalid_config_is_fatal_before_storage_but_help_version_and_export_ignore_it() {
    let fixture = Fixture::new();
    fixture.config("[display]\nwrap = 'invalid'");

    let output = fixture.command().output().unwrap();
    let error = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(
        error.contains("invalid configuration") && error.contains("config.toml"),
        "{error}"
    );
    assert!(!fixture.data.join("tasks.lock").exists());

    for args in [
        vec!["--help"],
        vec!["--version"],
        vec!["export"],
        vec!["--config", "missing.toml", "--help"],
        vec!["--config", "missing.toml", "--version"],
    ] {
        let output = fixture.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
    }

    assert!(!fixture.data.join("tasks.lock").exists());
    assert!(!fixture.data.join("tasks.json").exists());
    assert_eq!(
        fs::read_to_string(&fixture.config).unwrap(),
        "[display]\nwrap = 'invalid'"
    );
}

#[test]
fn missing_explicit_config_fails_clearly_without_creating_storage() {
    let fixture = Fixture::new();

    let output = fixture
        .command()
        .args(["--config", "missing.toml"])
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(error.contains("missing.toml"), "{error}");
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
}

#[test]
fn explicit_config_replaces_an_invalid_default_file() {
    let fixture = Fixture::new();
    fixture.config("invalid default");
    write_invalid_tasks(&fixture);
    let explicit = fixture.root.join("alternate.toml");
    fs::write(&explicit, "[display]\nwrap = false").unwrap();

    let output = fixture
        .command()
        .args(["--config", "./alternate.toml"])
        .output()
        .unwrap();

    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid task file"));
    assert_eq!(
        fs::read_to_string(explicit).unwrap(),
        "[display]\nwrap = false"
    );
}

#[test]
fn config_and_data_dir_select_their_files_independently() {
    let fixture = Fixture::new();
    let work = fixture.root.join("work");

    // `--data-dir` does not move the configuration: the invalid default file is still read,
    // and stops startup before the data directory is created.
    fixture.config("invalid default");

    let output = fixture
        .command()
        .arg("--data-dir")
        .arg(&work)
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&output.stderr);

    assert!(error.contains("invalid configuration"), "{error}");
    assert!(
        error.contains(&fixture.config.display().to_string()),
        "{error}"
    );
    assert!(!work.exists());

    // `--config` does not move the data: tasks come from `--data-dir`, and a config.toml
    // there is never read.
    write_invalid_tasks(&fixture);
    fs::create_dir(&work).unwrap();
    fs::write(work.join("tasks.json"), "invalid work data").unwrap();
    fs::write(work.join("config.toml"), "invalid work config").unwrap();
    fs::write(fixture.root.join("work.toml"), "[display]\nwrap = false").unwrap();

    let output = fixture
        .command()
        .args(["--config", "work.toml", "--data-dir", "work"])
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&output.stderr);
    let work_tasks = std::path::Path::new("work").join("tasks.json");

    assert!(!output.status.success());
    assert!(
        error.contains(&format!("invalid task file {}", work_tasks.display())),
        "{error}"
    );
    assert!(work.join("tasks.lock").is_file());
    assert!(!fixture.data.join("tasks.lock").exists());
}

#[test]
fn missing_default_config_does_not_get_created() {
    let fixture = Fixture::new();
    write_invalid_tasks(&fixture);
    let output = fixture.command().output().unwrap();

    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid task file"));
    assert!(!fixture.config.exists());
}

#[test]
fn invalid_style_theme_and_toml_files_are_reported_before_storage_and_stay_untouched() {
    let fixture = Fixture::new();

    for text in [
        "theme='unknown'",
        "[styles.accent]\nfg='reset'",
        "[styles.normal]\nfg='orange'",
        "[styles.muted]\nmodifiers=['typo']",
        "[styles.border]\nbg='red'",
        "[styles.typo]",
        "[input]\nmouse='off'",
        "[input]\ntypo=true",
        "bad toml",
    ] {
        fixture.config(text);

        let output = fixture.command().output().unwrap();
        let error = String::from_utf8_lossy(&output.stderr);

        assert!(!output.status.success(), "{text}");
        assert!(
            error.contains("invalid configuration") && error.contains("config.toml"),
            "{text}: {error}"
        );
        assert!(!fixture.data.join("tasks.lock").exists());
        assert!(!fixture.data.join("tasks.json").exists());
        assert_eq!(fs::read_to_string(&fixture.config).unwrap(), text);
    }
}

#[test]
fn unknown_theme_errors_point_to_list_themes() {
    let fixture = Fixture::new();
    fixture.config("theme='unknown'");

    let output = fixture.command().output().unwrap();
    let error = String::from_utf8_lossy(&output.stderr);

    assert!(error.contains("sati --list-themes"), "{error}");
}

#[test]
#[cfg(not(any(target_os = "macos", windows)))]
fn relative_config_home_is_still_rejected_before_storage() {
    let fixture = Fixture::new();

    let output = fixture
        .command()
        .env("XDG_CONFIG_HOME", "relative")
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&output.stderr);

    assert!(error.contains("XDG_CONFIG_HOME"), "{error}");
    assert!(!fixture.data.join("tasks.lock").exists());
}
