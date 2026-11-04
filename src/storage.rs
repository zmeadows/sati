use std::{
    env,
    error::Error,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use crate::app::Task;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub struct Storage {
    path: PathBuf,
}

impl Storage {
    pub fn discover() -> Result<Self> {
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
        Ok(Self {
            path: base.join("sati/tasks.json"),
        })
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
        fs::create_dir_all(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
        let mut contents = serde_json::to_vec_pretty(tasks)?;
        contents.push(b'\n');

        let (temporary, mut file) = create_temporary(directory)?;
        // Never truncate the live file. Keep the temporary file on failure for recovery.
        file.write_all(&contents)
            .and_then(|()| file.sync_all())
            .map_err(|error| {
                format!(
                    "could not write {}: {error}. Original tasks are unchanged; temporary data remains at {}",
                    self.path.display(),
                    temporary.display()
                )
            })?;
        drop(file);
        fs::rename(&temporary, &self.path).map_err(|error| {
            format!(
                "could not replace {}: {error}. Original tasks are unchanged; recover the new list from {}",
                self.path.display(),
                temporary.display()
            )
        })?;
        File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(|error| {
                format!(
                    "saved {}, but could not sync its directory: {error}. The save may not survive a power loss",
                    self.path.display()
                )
            })?;
        Ok(())
    }
}

fn create_temporary(directory: &Path) -> Result<(PathBuf, File)> {
    for sequence in 0..1000 {
        let path = directory.join(format!(".tasks.json.{}.{sequence}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
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
        assert!(error.contains("recover the new list"));
        assert!(fixture.task_path().is_dir());
        let temporary = fs::read_dir(fixture.task_path().parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|extension| extension == "tmp"))
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Vec<Task>>(&fs::read(temporary).unwrap()).unwrap(),
            tasks
        );
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
