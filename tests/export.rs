use std::{
    fs::{self, OpenOptions},
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture {
    root: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let root = std::env::temp_dir().join(format!(
            "sati-export-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));

        fs::create_dir(&root).unwrap();

        let data = if cfg!(target_os = "macos") {
            root.join("Library/Application Support/sati")
        } else {
            root.join("sati")
        };

        Self { root, data }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sati"));
        command
            .arg("export")
            .env("HOME", &self.root)
            .env("APPDATA", &self.root)
            .env("XDG_DATA_HOME", &self.root)
            .stdin(Stdio::null());

        command
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn prepare(&self) {
        fs::create_dir_all(&self.data).unwrap();
        fs::write(self.data.join("tasks.json"), tasks_document("")).unwrap();
        fs::write(
            self.data.join("history.json"),
            "deliberately invalid unused history",
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
const MARKDOWN: &str = "- [ ] Work: 日本語\n- [x] done\n";

// A current-format task file with a carried sectioned task and a completed one.
fn tasks_document(extra_fields: &str) -> String {
    format!(
        r#"{{"schema_version":5,"tasks":[{{"id":"00000000-0000-4000-8000-000000000001","created_at":"2026-11-07T19:24:05Z","section":"Work","text":"日本語","status":"carried"}},{{"id":"00000000-0000-4000-8000-000000000002","created_at":"2026-11-07T19:24:06Z","text":"done","status":"completed"}}]{extra_fields},"section_order":["work"]}}"#
    )
}

#[test]
fn export_reads_committed_tasks_while_the_writer_lock_is_held() {
    let fixture = Fixture::new();
    fixture.prepare();

    let held = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(fixture.data.join("tasks.lock"))
        .unwrap();

    held.try_lock().unwrap();
    let before = fs::read(fixture.data.join("tasks.json")).unwrap();
    let output = fixture.run(&[]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, MARKDOWN.as_bytes());
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read(fixture.data.join("tasks.json")).unwrap(), before);

    let document = tasks_document(r#","selected":1"#);
    fs::write(fixture.data.join("tasks.json"), &document).unwrap();
    let output = fixture.run(&[]);

    assert!(output.status.success() && output.stderr.is_empty());
    assert_eq!(output.stdout, MARKDOWN.as_bytes());
    assert_eq!(
        fs::read_to_string(fixture.data.join("tasks.json")).unwrap(),
        document
    );
    assert_eq!(
        fs::read_to_string(fixture.data.join("history.json")).unwrap(),
        "deliberately invalid unused history"
    );
    assert!(
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(fixture.data.join("tasks.lock"))
            .unwrap()
            .try_lock()
            .is_err()
    );
}

#[test]
fn missing_and_invalid_storage_never_get_created_or_repaired() {
    let fixture = Fixture::new();
    let output = fixture.run(&[]);
    assert!(output.status.success());
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);

    fixture.prepare();

    for invalid in [
        "malformed",
        r#"{"schema_version":5,"tasks":[{"id":"00000000-0000-4000-8000-000000000001","created_at":"2026-11-07T19:24:05Z","text":"bad","status":"done"}]}"#,
        r#"{"schema_version":5,"tasks":[{"id":"00000000-0000-4000-8000-000000000001","created_at":"2026-11-07T19:24:05Z","text":"bad\ntext","status":"pending"}]}"#,
    ] {
        fs::write(fixture.data.join("tasks.json"), invalid).unwrap();
        let output = fixture.run(&[]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("invalid task file"));
        assert_eq!(
            fs::read_to_string(fixture.data.join("tasks.json")).unwrap(),
            invalid
        );
    }

    // A file from a newer sati is refused rather than misread.
    let future = r#"{"schema_version":99,"tasks":[]}"#;
    fs::write(fixture.data.join("tasks.json"), future).unwrap();
    let output = fixture.run(&[]);

    assert!(!output.status.success() && output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("schema version 99"));
    assert_eq!(
        fs::read_to_string(fixture.data.join("tasks.json")).unwrap(),
        future
    );
}

#[test]
fn export_rejects_removed_file_options_and_unknown_arguments() {
    let fixture = Fixture::new();
    fixture.prepare();

    for args in [
        vec!["--force"],
        vec!["--output", "tasks.md"],
        vec!["-o", "tasks.md"],
        vec!["--format", "json"],
        vec!["unexpected"],
    ] {
        let output = fixture
            .command()
            .args(&args)
            .current_dir(&fixture.root)
            .output()
            .unwrap();

        assert!(!output.status.success(), "accepted {args:?}");
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument"));
        assert!(!fixture.root.join("tasks.md").exists());
    }

    let help = fixture.run(&["--help"]);
    assert!(help.status.success());

    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("stdout"));
    assert!(!help.contains("--output") && !help.contains("--force"));
}
