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
// otherwise take over the developer's terminal. An invalid task file stops startup there.
fn write_invalid_tasks(fixture: &Fixture) {
    fs::create_dir_all(&fixture.data).unwrap();
    fs::write(fixture.data.join("tasks.json"), "invalid task data").unwrap();
}

#[test]
fn invalid_config_no_longer_stops_startup_and_help_version_and_export_ignore_it() {
    let fixture = Fixture::new();
    fixture.config("[display]\nwrap = 'invalid'");

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

    write_invalid_tasks(&fixture);
    let output = fixture.command().output().unwrap();
    let error = String::from_utf8_lossy(&output.stderr);

    // Startup reached storage: the configuration error is left for the in-app overlay.
    assert!(!output.status.success());
    assert!(error.contains("invalid task file"), "{error}");
    assert!(!error.contains("configuration"), "{error}");
    assert!(!fixture.data.join("history.json").exists());
    assert_eq!(
        fs::read_to_string(&fixture.config).unwrap(),
        "[display]\nwrap = 'invalid'"
    );
}

#[test]
fn missing_explicit_paths_no_longer_stop_startup_and_replace_discovery() {
    let fixture = Fixture::new();
    write_invalid_tasks(&fixture);

    let output = fixture
        .command()
        .args(["--config", "missing.toml"])
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&output.stderr);

    assert!(error.contains("invalid task file"), "{error}");
    assert!(!fixture.root.join("missing.toml").exists());

    fixture.config("invalid default");
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
fn missing_default_config_does_not_get_created() {
    let fixture = Fixture::new();
    write_invalid_tasks(&fixture);
    let output = fixture.command().output().unwrap();

    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid task file"));
    assert!(!fixture.config.exists());
}

#[test]
fn invalid_style_and_theme_files_reach_storage_and_stay_untouched() {
    let fixture = Fixture::new();
    write_invalid_tasks(&fixture);

    for text in [
        "theme='unknown'",
        "[styles.accent]\nfg='reset'",
        "[styles.normal]\nfg='orange'",
        "[styles.muted]\nmodifiers=['typo']",
        "[styles.border]\nbg='red'",
        "[styles.typo]",
        "bad toml",
    ] {
        fixture.config(text);
        let output = fixture.command().output().unwrap();
        let error = String::from_utf8_lossy(&output.stderr);

        assert!(error.contains("invalid task file"), "{text}: {error}");
        assert_eq!(fs::read_to_string(&fixture.config).unwrap(), text);
    }
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
