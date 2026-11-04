use std::{
    env,
    error::Error,
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

use crate::app::Task;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub struct Storage {
    path: PathBuf,
}

impl Storage {
    pub fn discover() -> Result<Self> {
        #[cfg(not(any(target_os = "macos", windows)))]
        let base = match env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
            Some(path) => {
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    return Err("XDG_DATA_HOME must be an absolute path".into());
                }
                path
            }
            None => PathBuf::from(
                env::var_os("HOME")
                    .filter(|value| !value.is_empty())
                    .ok_or("neither XDG_DATA_HOME nor HOME is set")?,
            )
            .join(".local/share"),
        };
        #[cfg(target_os = "macos")]
        let base = PathBuf::from(
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .ok_or("HOME is not set")?,
        )
        .join("Library/Application Support");
        #[cfg(windows)]
        let base = PathBuf::from(
            env::var_os("APPDATA")
                .filter(|value| !value.is_empty())
                .ok_or("APPDATA is not set")?,
        );
        if !base.is_absolute() {
            return Err("data directory must be an absolute path".into());
        }
        Ok(Self {
            path: base.join("sati/tasks.json"),
        })
    }

    pub fn lock(&self) -> Result<File> {
        let directory = self
            .path
            .parent()
            .expect("task file has a parent directory");
        create_data_directory(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
        let path = directory.join("tasks.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options
            .open(&path)
            .map_err(|error| format!("could not open {}: {error}", path.display()))?;
        // Keep this file open and never unlink it: every instance must lock the same inode.
        match fs4::FileExt::try_lock(&file) {
            Ok(()) => Ok(file),
            Err(fs4::TryLockError::WouldBlock) => Err("already running".into()),
            Err(error) => Err(format!("could not lock {}: {error}", path.display()).into()),
        }
    }

    pub fn load(&self) -> Result<Vec<Task>> {
        let contents = match fs::read(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(format!("could not read {}: {error}", self.path.display()).into());
            }
        };
        serde_json::from_slice(&contents).map_err(|error| {
            format!(
                "invalid task file {}: {error}. The file has been left untouched; repair it or move it aside before restarting",
                self.path.display()
            )
            .into()
        })
    }

    pub fn save(&self, tasks: &[Task]) -> Result<()> {
        let directory = self
            .path
            .parent()
            .expect("task file has a parent directory");
        create_data_directory(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
        let mut contents = serde_json::to_vec_pretty(tasks)?;
        contents.push(b'\n');

        let (temporary, mut file) = create_temporary(directory)?;
        // Never truncate the live file. Failed writes may leave incomplete temporary data.
        file.write_all(&contents)
            .and_then(|()| file.sync_all())
            .map_err(|error| {
                format!(
                    "could not write or sync {}: {error}. Original tasks are unchanged; temporary file at {} may contain incomplete JSON",
                    self.path.display(),
                    temporary.display()
                )
            })?;
        drop(file);
        // Same-directory rename replaces an existing file on Unix and Windows;
        // never delete the destination first or fall back to copying.
        fs::rename(&temporary, &self.path).map_err(|error| {
            format!(
                "could not replace {}: {error}. Original tasks are unchanged; fully written temporary file remains at {}",
                self.path.display(),
                temporary.display()
            )
        })?;
        // Replacement is now visible. A directory-sync failure cannot roll it back.
        #[cfg(unix)]
        File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(|error| {
                format!(
                    "replaced {}, but could not sync directory {}: {error}. The new task list is already visible; its durability after a crash or power loss is uncertain",
                    self.path.display(),
                    directory.display()
                )
            })?;
        // On Windows the temporary file was synced, but there is no portable
        // directory sync. Success means replacement is visible, not that the
        // directory entry is guaranteed to survive a crash or power loss.
        Ok(())
    }
}

fn create_data_directory(directory: &Path) -> io::Result<()> {
    if let Some(parent) = directory.parent() {
        fs::create_dir_all(parent)?;
    }
    // Apply private permissions only when creating sati's own directory.
    let builder = DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    match builder.create(directory) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && directory.is_dir() => Ok(()),
        result => result,
    }
}

fn create_temporary(directory: &Path) -> Result<(PathBuf, File)> {
    for sequence in 0..1000 {
        let path = directory.join(format!(".tasks.json.{}.{sequence}.tmp", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("could not create {}: {error}", path.display()).into());
            }
        }
    }
    Err(format!("too many temporary files in {}", directory.display()).into())
}

#[cfg(test)]
pub mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    pub struct Fixture {
        pub directory: PathBuf,
        pub storage: Storage,
    }

    impl Fixture {
        pub fn new() -> Self {
            let directory = env::temp_dir().join(format!(
                "sati-test-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&directory).unwrap();
            Self {
                storage: Storage {
                    path: directory.join("data/tasks.json"),
                },
                directory,
            }
        }

        pub fn task_path(&self) -> &Path {
            &self.storage.path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    #[cfg(unix)]
    fn new_application_data_is_private_without_changing_parent_permissions() {
        for with_lock in [false, true] {
            let fixture = Fixture::new();
            fs::set_permissions(&fixture.directory, fs::Permissions::from_mode(0o755)).unwrap();
            let _held = with_lock.then(|| fixture.storage.lock().unwrap());
            fixture
                .storage
                .save(&[Task {
                    text: "Private task".into(),
                    completed: false,
                }])
                .unwrap();
            let directory = fixture.task_path().parent().unwrap();
            assert_eq!(
                fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(fixture.task_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&fixture.directory)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
            if with_lock {
                assert_eq!(
                    fs::metadata(directory.join("tasks.lock"))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
            let (path, file) = create_temporary(directory).unwrap();
            assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
            drop(file);
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    #[cfg(unix)]
    fn existing_directories_and_files_are_not_chmodded() {
        let fixture = Fixture::new();
        let directory = fixture.task_path().parent().unwrap();
        fs::create_dir(directory).unwrap();
        fs::set_permissions(directory, fs::Permissions::from_mode(0o750)).unwrap();
        let lock_path = directory.join("tasks.lock");
        fs::write(&lock_path, "").unwrap();
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o640)).unwrap();
        fs::write(fixture.task_path(), "[]\n").unwrap();
        fs::set_permissions(fixture.task_path(), fs::Permissions::from_mode(0o640)).unwrap();
        let unrelated = fixture.directory.join("other-user-file");
        fs::write(&unrelated, "Keep this file unchanged").unwrap();
        fs::set_permissions(&unrelated, fs::Permissions::from_mode(0o644)).unwrap();
        let _held = fixture.storage.lock().unwrap();
        assert!(fixture.storage.load().unwrap().is_empty());
        assert_eq!(
            fs::metadata(fixture.task_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        fixture.storage.save(&[]).unwrap();
        // A save publishes the private replacement; it does not chmod other existing paths.
        assert_eq!(
            fs::metadata(fixture.task_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o750
        );
        assert_eq!(
            fs::metadata(lock_path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(
            fs::metadata(&unrelated).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(
            fs::read_to_string(unrelated).unwrap(),
            "Keep this file unchanged"
        );
    }

    #[test]
    fn lock_stays_exclusive_across_saves_and_is_released_without_removing_file() {
        let fixture = Fixture::new();
        let held = fixture.storage.lock().unwrap();
        let path = fixture.task_path().parent().unwrap().join("tasks.lock");
        assert!(path.is_file());
        assert!(fixture.storage.load().unwrap().is_empty());
        assert!(
            fixture
                .storage
                .lock()
                .unwrap_err()
                .to_string()
                .contains("already running")
        );
        let tasks = vec![Task {
            text: "Keep me".into(),
            completed: false,
        }];
        fixture.storage.save(&tasks).unwrap();
        fixture.storage.save(&[]).unwrap();
        assert!(
            fixture
                .storage
                .lock()
                .unwrap_err()
                .to_string()
                .contains("already running")
        );
        drop(held);
        assert!(path.is_file());
        let _reacquired = fixture.storage.lock().unwrap();
    }

    #[test]
    fn independent_directories_can_be_locked_but_aliases_cannot() {
        let first = Fixture::new();
        let second = Fixture::new();
        let _first_lock = first.storage.lock().unwrap();
        let _second_lock = second.storage.lock().unwrap();
        let alias = second.directory.join("alias");
        #[cfg(unix)]
        std::os::unix::fs::symlink(first.task_path().parent().unwrap(), &alias).unwrap();
        #[cfg(windows)]
        {
            fs::create_dir(&alias).unwrap();
            // A hard link tests file identity without requiring symlink privileges.
            fs::hard_link(
                first.task_path().parent().unwrap().join("tasks.lock"),
                alias.join("tasks.lock"),
            )
            .unwrap();
        }
        let same_directory = Storage {
            path: alias.join("tasks.json"),
        };
        assert!(
            same_directory
                .lock()
                .unwrap_err()
                .to_string()
                .contains("already running")
        );
    }

    #[test]
    fn lock_setup_failures_are_reported() {
        let fixture = Fixture::new();
        let directory = fixture.task_path().parent().unwrap();
        fs::write(directory, "blocking file").unwrap();
        assert!(
            fixture
                .storage
                .lock()
                .unwrap_err()
                .to_string()
                .contains("could not create")
        );
        fs::remove_file(directory).unwrap();
        fs::create_dir_all(directory.join("tasks.lock")).unwrap();
        assert!(
            fixture
                .storage
                .lock()
                .unwrap_err()
                .to_string()
                .contains("could not open")
        );
    }

    #[test]
    fn saves_order_unicode_and_completion_and_replaces_atomically() {
        let fixture = Fixture::new();
        assert!(fixture.storage.load().unwrap().is_empty());
        let tasks = vec![
            Task {
                text: "Read 日本語 🪷".into(),
                completed: false,
            },
            Task {
                text: "Keep this position".into(),
                completed: true,
            },
        ];
        fixture.storage.save(&tasks).unwrap();
        assert_eq!(fixture.storage.load().unwrap(), tasks);
        let json = fs::read_to_string(fixture.task_path()).unwrap();
        assert!(json.starts_with("[\n"));
        assert!(json.ends_with('\n'));
        let mut replacement = tasks.clone();
        replacement.reverse();
        replacement[0].completed = false;
        replacement[1].text.push_str(" updated");
        for list in [&replacement, &tasks, &replacement] {
            fixture.storage.save(list).unwrap();
            assert_eq!(fixture.storage.load().unwrap(), *list);
        }
        fixture.storage.save(&[]).unwrap();
        assert!(fixture.storage.load().unwrap().is_empty());
        assert_eq!(
            fs::read_dir(fixture.task_path().parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn temporary_collisions_preserve_existing_files_and_live_data() {
        let fixture = Fixture::new();
        let tasks = vec![Task {
            text: "Keep the original".into(),
            completed: true,
        }];
        fixture.storage.save(&tasks).unwrap();
        let original = fs::read(fixture.task_path()).unwrap();
        let directory = fixture.task_path().parent().unwrap();
        let collision = directory.join(format!(".tasks.json.{}.0.tmp", std::process::id()));
        fs::write(&collision, "old recovery data").unwrap();
        let (temporary, file) = create_temporary(directory).unwrap();
        assert_eq!(temporary.parent(), Some(directory));
        assert_ne!(temporary, collision);
        assert_eq!(fs::read_to_string(&collision).unwrap(), "old recovery data");
        drop(file);
        fs::remove_file(temporary).unwrap();

        // Exhaust the bounded namespace: a real creation failure independent
        // of runner privileges, permissions, or available disk space.
        for sequence in 1..1000 {
            fs::write(
                directory.join(format!(".tasks.json.{}.{sequence}.tmp", std::process::id())),
                "old recovery data",
            )
            .unwrap();
        }
        let error = fixture.storage.save(&[]).unwrap_err().to_string();
        assert!(error.contains("too many temporary files"), "{error}");
        assert_eq!(fs::read(fixture.task_path()).unwrap(), original);
        assert_eq!(fixture.storage.load().unwrap(), tasks);
        assert_eq!(fs::read_to_string(collision).unwrap(), "old recovery data");
    }

    #[test]
    fn corrupt_or_incompatible_data_is_never_replaced_at_startup() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.task_path().parent().unwrap()).unwrap();
        for invalid in [
            "not json",
            "{}",
            "null",
            r#"[{"text":"missing completion"}]"#,
            r#"[{"text":"future data","completed":false,"extra":1}]"#,
        ] {
            fs::write(fixture.task_path(), invalid).unwrap();
            assert!(fixture.storage.load().is_err());
            assert_eq!(fs::read_to_string(fixture.task_path()).unwrap(), invalid);
        }
    }

    #[test]
    fn replacement_failure_keeps_recovery_file() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.task_path()).unwrap();
        let tasks = vec![Task {
            text: "Recover me".into(),
            completed: false,
        }];
        let error = fixture.storage.save(&tasks).unwrap_err().to_string();
        assert!(error.contains("fully written temporary file remains"));
        assert!(fixture.task_path().is_dir());
        let temporary = fs::read_dir(fixture.task_path().parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|extension| extension == "tmp"))
            .unwrap();
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&temporary).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            serde_json::from_slice::<Vec<Task>>(&fs::read(temporary).unwrap()).unwrap(),
            tasks
        );
    }

    #[test]
    #[cfg(windows)]
    fn replacement_denied_by_open_file_preserves_original_and_recovery_data() {
        use std::os::windows::fs::OpenOptionsExt;

        let fixture = Fixture::new();
        fixture
            .storage
            .save(&[Task {
                text: "Original tasks".into(),
                completed: true,
            }])
            .unwrap();
        let original = fs::read(fixture.task_path()).unwrap();
        // Denying delete sharing prevents replacement even on privileged runners.
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(fixture.task_path())
            .unwrap();
        let replacement = vec![Task {
            text: "Recover me".into(),
            completed: false,
        }];
        let error = fixture.storage.save(&replacement).unwrap_err().to_string();
        drop(held);
        assert!(error.contains("could not replace"), "{error}");
        assert!(error.contains("Original tasks are unchanged"), "{error}");
        assert!(
            error.contains("fully written temporary file remains"),
            "{error}"
        );
        assert_eq!(fs::read(fixture.task_path()).unwrap(), original);
        let temporary = fs::read_dir(fixture.task_path().parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|extension| extension == "tmp"))
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Vec<Task>>(&fs::read(temporary).unwrap()).unwrap(),
            replacement
        );
    }

    #[test]
    #[cfg(unix)]
    fn directory_sync_failure_reports_visible_replacement_and_uncertain_durability() {
        let fixture = Fixture::new();
        fixture
            .storage
            .save(&[Task {
                text: "Old list".into(),
                completed: false,
            }])
            .unwrap();
        let directory = fixture.task_path().parent().unwrap();
        // Write/search permission permits replacement, but opening the directory for sync fails.
        fs::set_permissions(directory, fs::Permissions::from_mode(0o300)).unwrap();
        if File::open(directory).is_ok() {
            // Root/capability-enabled test runners can bypass these permissions.
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
            return;
        }
        let replacement = vec![Task {
            text: "New list".into(),
            completed: true,
        }];
        let result = fixture.storage.save(&replacement);
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        let error = result.unwrap_err().to_string();
        assert!(error.contains("replaced "));
        assert!(error.contains("already visible"));
        assert!(error.contains("uncertain"));
        assert!(!error.contains("unchanged"));
        assert_eq!(fixture.storage.load().unwrap(), replacement);
        assert_eq!(fs::read_dir(directory).unwrap().count(), 1);
    }

    #[test]
    fn directory_creation_failure_is_reported_without_changing_existing_data() {
        let fixture = Fixture::new();
        fs::write(fixture.directory.join("data"), "existing file").unwrap();
        assert!(
            fixture
                .storage
                .save(&[])
                .unwrap_err()
                .to_string()
                .contains("could not create")
        );
        assert_eq!(
            fs::read_to_string(fixture.directory.join("data")).unwrap(),
            "existing file"
        );
    }
}
