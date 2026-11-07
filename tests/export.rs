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
        fs::write(self.data.join("tasks.json"), r#"[{"text":"Work: 日本語","completed":false,"carried":true},{"text":"done","completed":true}]"#).unwrap();
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

    fs4::FileExt::try_lock(&held).unwrap();
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

    let document = format!(
        r#"{{"tasks":{},"selected":1}}"#,
        String::from_utf8(before).unwrap()
    );
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
        fs4::FileExt::try_lock(
            &OpenOptions::new()
                .read(true)
                .write(true)
                .open(fixture.data.join("tasks.lock"))
                .unwrap()
        )
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
        r#"[{"text":"bad","completed":true,"carried":true}]"#,
        r#"[{"text":"bad\ntext","completed":false}]"#,
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
fn export_files_require_force_and_do_not_leave_partial_outputs() {
    let fixture = Fixture::new();
    fixture.prepare();
    let path = fixture.root.join("tasks.md");

    let first = fixture
        .command()
        .arg("--output")
        .arg(&path)
        .output()
        .unwrap();

    assert!(first.status.success());
    assert!(first.stdout.is_empty() && first.stderr.is_empty());
    assert_eq!(fs::read(&path).unwrap(), MARKDOWN.as_bytes());

    fs::write(&path, "original\n").unwrap();
    let refused = fixture.command().arg("-o").arg(&path).output().unwrap();
    assert!(!refused.status.success());
    assert_eq!(fs::read(&path).unwrap(), b"original\n");

    let replaced = fixture
        .command()
        .arg("--force")
        .arg("-o")
        .arg(&path)
        .output()
        .unwrap();

    assert!(replaced.status.success());
    assert_eq!(fs::read(&path).unwrap(), MARKDOWN.as_bytes());

    let missing = fixture.root.join("missing/tasks.md");
    assert!(
        !fixture
            .command()
            .arg("-o")
            .arg(&missing)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!missing.exists());
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 2);
}

#[test]
fn concurrent_exports_cannot_overwrite_each_other_without_force() {
    let fixture = Fixture::new();
    fixture.prepare();
    let path = fixture.root.join("tasks.md");

    let mut first = fixture
        .command()
        .arg("-o")
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let second = fixture.command().arg("-o").arg(&path).output().unwrap();
    let status = first.wait().unwrap();
    assert_ne!(status.success(), second.status.success());
    assert_eq!(fs::read(&path).unwrap(), MARKDOWN.as_bytes());
}

#[test]
fn export_protects_state_files_and_rejects_invalid_arguments() {
    let fixture = Fixture::new();
    fixture.prepare();

    for name in [
        "tasks.json",
        "history.json",
        "tasks.lock",
        ".tasks.json.writer.tmp",
        ".history.json.writer.tmp",
    ] {
        let path = fixture.data.join(name);

        if name != "tasks.json" && name != "history.json" {
            fs::write(&path, "protected").unwrap();
        }

        let bytes = fs::read(&path).unwrap();

        for force in [false, true] {
            let mut command = fixture.command();
            command.arg("-o").arg(&path);

            if force {
                command.arg("--force");
            }

            let output = command.output().unwrap();
            assert!(!output.status.success(), "accepted {name}");
            assert!(output.stdout.is_empty());
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("cannot overwrite sati state")
            );
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    for args in [
        vec!["--force"],
        vec!["--output"],
        vec!["--output", "--force"],
        vec!["--format", "json"],
        vec!["unexpected"],
        vec!["-o", "one", "-o", "two"],
    ] {
        let output = fixture.run(&args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }

    let help = fixture.run(&["--help"]);
    assert!(help.status.success());

    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("--output") && help.contains("--force") && help.contains("-o"));
}

#[cfg(unix)]
#[test]
fn export_refuses_symlink_aliases_of_state_files() {
    let fixture = Fixture::new();
    fixture.prepare();

    for name in ["tasks.json", "history.json"] {
        let alias = fixture.root.join(format!("{name}.md"));
        std::os::unix::fs::symlink(fixture.data.join(name), &alias).unwrap();
        let bytes = fs::read(&alias).unwrap();

        assert!(
            !fixture
                .command()
                .arg("-o")
                .arg(&alias)
                .arg("--force")
                .output()
                .unwrap()
                .status
                .success()
        );
        assert_eq!(fs::read(alias).unwrap(), bytes);
    }
}

#[cfg(windows)]
#[test]
fn windows_export_refuses_normalized_state_names_even_when_missing() {
    let fixture = Fixture::new();
    fixture.prepare();
    fs::remove_file(fixture.data.join("history.json")).unwrap();

    for name in ["history.json.", "HISTORY.JSON ", "history.json:stream"] {
        assert!(
            !fixture
                .command()
                .arg("-o")
                .arg(fixture.data.join(name))
                .arg("--force")
                .output()
                .unwrap()
                .status
                .success()
        );
    }

    assert!(!fixture.data.join("history.json").exists());
}
