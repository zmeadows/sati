use std::{
    env,
    error::Error,
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::{Task, settled_section_order};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

// The `tasks.json` format written by this build.
const SCHEMA_VERSION: u64 = 5;

// The oldest `tasks.json` format this build can still upgrade. Older files are refused.
const OLDEST_SCHEMA_VERSION: u64 = 5;

// A migration receives a document without its `schema_version` field, which the loader owns.
type Migration = fn(Value) -> std::result::Result<Value, String>;

// `MIGRATIONS[n]` upgrades a version `OLDEST_SCHEMA_VERSION + n` document to the next version.
// To change the format, bump `SCHEMA_VERSION` and append one migration; the array length
// enforces the pairing. Each step should first parse the previous version strictly into its
// own typed struct with `deny_unknown_fields`, then build the next version, so unexpected or
// malformed data is rejected instead of silently carried along. Migrations run in memory on
// load; the upgraded document is written by the next save.
const MIGRATIONS: [Migration; (SCHEMA_VERSION - OLDEST_SCHEMA_VERSION) as usize] = [];

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDocument {
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub selected: Option<usize>,
    // The active section tab: absent for all tasks, "" for unsectioned, else a lowercase key.
    #[serde(default)]
    pub section: Option<String>,
    // Lowercase keys of the named section tabs, in display order; each task section once.
    #[serde(default)]
    pub section_order: Vec<String>,
    #[serde(skip)]
    pub needs_migration: bool,
}

pub fn valid_selection(tasks: &[Task], selected: Option<usize>) -> Option<usize> {
    selected
        .filter(|&index| index < tasks.len())
        .or_else(|| (!tasks.is_empty()).then_some(0))
}

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
            path: base.join("sati").join("tasks.json"),
        })
    }

    pub fn export_path(&self, output: &Path) -> Result<PathBuf> {
        let name = output.file_name().ok_or("export output must name a file")?;
        let parent = output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));

        let directory = fs::canonicalize(parent).map_err(|error| {
            format!(
                "could not access export directory {}: {error}",
                parent.display()
            )
        })?;

        let destination = directory.join(name);

        let state_directory = match fs::canonicalize(self.path.parent().unwrap()) {
            Ok(path) => Some(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };

        let state_name = name.to_str();
        #[cfg(windows)]
        let state_name =
            state_name.map(|name| name.split(':').next().unwrap().trim_end_matches(['.', ' ']));

        for reserved in ["tasks.json", "history.json", "tasks.lock"] {
            if state_directory.as_ref() == Some(&directory)
                && state_name.is_some_and(|name| {
                    name.eq_ignore_ascii_case(reserved)
                        || name
                            .to_ascii_lowercase()
                            .starts_with(&format!(".{reserved}."))
                })
            {
                return Err("export cannot overwrite sati state files".into());
            }

            // Also reject existing symlink/junction aliases to state files.
            let state = self.path.with_file_name(reserved);

            if let (Ok(state), Ok(target)) =
                (fs::canonicalize(state), fs::canonicalize(&destination))
                && state == target
            {
                return Err("export cannot overwrite sati state files".into());
            }
        }

        Ok(destination)
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
        Ok(self.load_document()?.tasks)
    }

    pub fn load_document(&self) -> Result<TaskDocument> {
        let contents = match fs::read(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(TaskDocument::default());
            }
            Err(error) => {
                return Err(format!("could not read {}: {error}", self.path.display()).into());
            }
        };

        let invalid_file = |problem: String| {
            format!(
                "invalid task file {}: {problem}. The file has been left untouched; repair it or move it aside before restarting",
                self.path.display()
            )
        };

        let value: Value =
            serde_json::from_slice(&contents).map_err(|error| invalid_file(error.to_string()))?;

        let Value::Object(mut fields) = value else {
            return Err(invalid_file("expected an object".to_owned()).into());
        };

        let version = fields
            .remove("schema_version")
            .ok_or_else(|| invalid_file("missing schema_version".to_owned()))?;
        let version = version.as_u64().ok_or_else(|| {
            invalid_file("schema_version must be a non-negative integer".to_owned())
        })?;

        if version > SCHEMA_VERSION {
            return Err(format!(
                "task file {} uses schema version {version}, but this sati supports version {SCHEMA_VERSION}. The file has been left untouched; update sati to open it",
                self.path.display()
            )
            .into());
        }

        if version < OLDEST_SCHEMA_VERSION {
            return Err(format!(
                "task file {} uses schema version {version}, but this sati only opens version {OLDEST_SCHEMA_VERSION} and later. The file has been left untouched; open it once with an older sati release to upgrade it",
                self.path.display()
            )
            .into());
        }

        // Upgrade in memory only; nothing is written until the next save.
        let mut value = Value::Object(fields);
        let applied_from = (version - OLDEST_SCHEMA_VERSION) as usize;

        for (offset, migrate) in MIGRATIONS.iter().enumerate().skip(applied_from) {
            let from = OLDEST_SCHEMA_VERSION + offset as u64;

            value = migrate(value).map_err(|problem| {
                invalid_file(format!("migrating from version {from}: {problem}"))
            })?;
        }

        let mut document: TaskDocument =
            serde_json::from_value(value).map_err(|error| invalid_file(error.to_string()))?;
        document.needs_migration = version < SCHEMA_VERSION;

        let mut first_with_id = std::collections::HashMap::new();

        for (index, task) in document.tasks.iter().enumerate() {
            if let Some(problem) = task.validation_error() {
                return Err(invalid_file(format!("task {} {problem}", index + 1)).into());
            }

            if let Some(first) = first_with_id.insert(task.id, index) {
                return Err(invalid_file(format!(
                    "tasks {} and {} have the same id {}; each task needs a unique id",
                    first + 1,
                    index + 1,
                    task.id
                ))
                .into());
            }
        }

        // The order is part of the data, so a mismatch is reported rather than repaired.
        let expected_order = settled_section_order(&document.section_order, &document.tasks);

        if document.section_order != expected_order {
            return Err(invalid_file(format!(
                "section_order must list each task section exactly once in lowercase, such as {expected_order:?}"
            ))
            .into());
        }

        document.selected = valid_selection(&document.tasks, document.selected);

        Ok(document)
    }

    #[cfg(test)]
    pub fn save(&self, tasks: &[Task]) -> Result<()> {
        self.save_selected(tasks, None)
    }

    #[cfg(test)]
    pub fn save_selected(&self, tasks: &[Task], selected: Option<usize>) -> Result<()> {
        self.save_view(tasks, &settled_section_order(&[], tasks), selected, None)
    }

    pub fn save_view(
        &self,
        tasks: &[Task],
        section_order: &[String],
        selected: Option<usize>,
        section: Option<&str>,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Saved<'a> {
            schema_version: u64,
            tasks: &'a [Task],
            selected: Option<usize>,
            #[serde(skip_serializing_if = "Option::is_none")]
            section: Option<&'a str>,
            section_order: &'a [String],
        }

        atomic_save(
            &self.path,
            &Saved {
                schema_version: SCHEMA_VERSION,
                tasks,
                selected: valid_selection(tasks, selected),
                section,
                section_order,
            },
            "Original tasks are unchanged",
            "The new task list is already visible",
        )
    }
}

fn atomic_save<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
    unchanged: &str,
    _visible: &str,
) -> Result<()> {
    let directory = path.parent().expect("task file has a parent directory");
    create_data_directory(directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let mut contents = serde_json::to_vec_pretty(value)?;
    contents.push(b'\n');

    let (temporary, mut file) =
        create_temporary(directory, path.file_name().unwrap().to_str().unwrap())?;
    // Never truncate the live file. Failed writes may leave incomplete temporary data.
    file.write_all(&contents)
            .and_then(|()| file.sync_all())
            .map_err(|error| {
                format!(
                    "could not write or sync {}: {error}. {unchanged}; temporary file at {} may contain incomplete JSON",
                    path.display(),
                    temporary.display()
                )
            })?;
    drop(file);
    // Same-directory rename replaces an existing file on Unix and Windows;
    // never delete the destination first or fall back to copying.
    fs::rename(&temporary, path).map_err(|error| {
            format!(
                "could not replace {}: {error}. {unchanged}; fully written temporary file remains at {}",
                path.display(),
                temporary.display()
            )
        })?;

    // Replacement is now visible. A directory-sync failure cannot roll it back.
    #[cfg(unix)]
        File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(|error| {
                format!(
                    "replaced {}, but could not sync directory {}: {error}. {_visible}; its durability after a crash or power loss is uncertain",
                    path.display(),
                    directory.display()
                )
            })?;
    // On Windows the temporary file was synced, but there is no portable
    // directory sync. Success means replacement is visible, not that the
    // directory entry is guaranteed to survive a crash or power loss.
    Ok(())
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

pub(crate) fn create_temporary(directory: &Path, name: &str) -> Result<(PathBuf, File)> {
    for sequence in 0..1000 {
        let path = directory.join(format!(".{name}.{}.{sequence}.tmp", std::process::id()));
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
    use crate::app::TaskStatus;
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
    fn task_documents_load_and_normalize_selection_without_writing() {
        let fixture = Fixture::new();
        let absent = fixture.storage.load_document().unwrap();
        assert!(absent.tasks.is_empty() && !absent.needs_migration);
        assert!(!fixture.task_path().exists());

        fs::create_dir_all(fixture.task_path().parent().unwrap()).unwrap();
        let document = |tasks: &str, selected: &str| {
            format!(r#"{{"schema_version":5,"tasks":{tasks}{selected},"section_order":[]}}"#)
        };

        for (json, selected) in [
            (document(IDENTIFIED_TASKS, r#","selected":1"#), Some(1)),
            (document(IDENTIFIED_TASKS, ""), Some(0)),
            (document(IDENTIFIED_TASKS, r#","selected":null"#), Some(0)),
            (document(IDENTIFIED_TASKS, r#","selected":99"#), Some(0)),
            (document("[]", r#","selected":5"#), None),
        ] {
            fs::write(fixture.task_path(), &json).unwrap();
            let document = fixture.storage.load_document().unwrap();

            assert_eq!(
                document.tasks.len(),
                if json.contains("first") { 2 } else { 0 }
            );
            assert_eq!(document.selected, selected, "{json}");
            assert!(!document.needs_migration, "{json}");
            assert_eq!(fs::read_to_string(fixture.task_path()).unwrap(), json);
        }

        for json in [
            r#"{"schema_version":5,"tasks":[],"selected":null,"extra":true}"#,
            r#"{"schema_version":5,"tasks":[{"id":"00000000-0000-4000-8000-000000000001","created_at":"2026-11-07T19:24:05Z","text":"bad\ntext","status":"pending"}],"selected":0}"#,
        ] {
            fs::write(fixture.task_path(), json).unwrap();
            assert!(fixture.storage.load_document().is_err());
            assert_eq!(fs::read_to_string(fixture.task_path()).unwrap(), json);
        }

        // Completion state never reorders tasks or rewrites the file while loading.
        let interleaved: Vec<_> = [
            ("done 1", true),
            ("active 1", false),
            ("done 2", true),
            ("active 2", false),
        ]
        .map(|(text, completed)| Task {
            status: if completed {
                TaskStatus::Completed
            } else {
                TaskStatus::Pending
            },
            ..Task::new(text)
        })
        .into();

        fixture.storage.save(&interleaved).unwrap();
        let contents = fs::read(fixture.task_path()).unwrap();

        assert_eq!(fixture.storage.load_document().unwrap().tasks, interleaved);
        assert_eq!(fs::read(fixture.task_path()).unwrap(), contents);

        fixture.storage.save_selected(&[], Some(99)).unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.task_path()).unwrap()).unwrap();

        assert_eq!(
            json,
            serde_json::json!({"schema_version": 5, "tasks":[], "selected":null, "section_order":[]})
        );

        // The version comes first so it is the first thing a reader sees.
        let saved = fs::read_to_string(fixture.task_path()).unwrap();
        assert!(saved.starts_with("{\n  \"schema_version\": 5,"), "{saved}");
        assert!(!fixture.storage.load_document().unwrap().needs_migration);
    }

    fn saved_tasks(path: &Path) -> Vec<Task> {
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(saved["schema_version"], SCHEMA_VERSION);

        serde_json::from_value(saved["tasks"].clone()).unwrap()
    }

    // Two pending tasks with valid ids and creation times and no section.
    const IDENTIFIED_TASKS: &str = r#"[
        {"id":"00000000-0000-4000-8000-000000000001","created_at":"2026-11-07T19:24:05Z","text":"first","status":"pending"},
        {"id":"00000000-0000-4000-8000-000000000002","created_at":"2026-11-07T19:24:06Z","text":"second","status":"pending"}
    ]"#;

    #[test]
    fn version_5_files_need_one_known_status_per_task() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.task_path().parent().unwrap()).unwrap();
        let file = |task_fields: &str| {
            format!(
                r#"{{"schema_version":5,"tasks":[{{"id":"00000000-0000-4000-8000-000000000001","created_at":"2026-11-07T19:24:05Z","text":"t"{task_fields}}}],"section_order":[]}}"#
            )
        };

        for (fields, problem) in [
            (r#","status":"done""#, "unknown variant `done`"),
            (r#","status":"Completed""#, "unknown variant `Completed`"),
            ("", "missing field `status`"),
            (
                r#","status":"pending","completed":false"#,
                "unknown field `completed`",
            ),
            (r#","completed":true"#, "unknown field `completed`"),
        ] {
            let json = file(fields);
            fs::write(fixture.task_path(), &json).unwrap();

            let error = fixture
                .storage
                .load_document()
                .err()
                .expect("rejected")
                .to_string();

            assert!(error.contains("invalid task file"), "{fields}: {error}");
            assert!(error.contains(problem), "{fields}: {error}");
            assert_eq!(fs::read_to_string(fixture.task_path()).unwrap(), json);
        }

        for status in ["pending", "completed", "carried"] {
            let json = file(&format!(r#","status":"{status}""#));
            fs::write(fixture.task_path(), &json).unwrap();

            let document = fixture.storage.load_document().unwrap();
            assert!(!document.needs_migration);

            fixture.storage.save(&document.tasks).unwrap();
            let saved: serde_json::Value =
                serde_json::from_slice(&fs::read(fixture.task_path()).unwrap()).unwrap();
            assert_eq!(saved["tasks"][0]["status"], status);
        }
    }

    #[test]
    fn stored_sections_are_validated_and_bodies_are_never_reparsed() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.task_path().parent().unwrap()).unwrap();
        let task = |section: &str, text: &str| {
            format!(
                r#"{{"schema_version":5,"tasks":[{{"id":"00000000-0000-4000-8000-000000000001","created_at":"2026-11-07T19:24:05Z","section":{section},"text":"{text}","status":"pending"}}],"section_order":["work"]}}"#
            )
        };

        for section in [r#""""#, r#""Side project""#, r#""a:b""#, r#""tab\t""#, "5"] {
            let json = task(section, "body");
            fs::write(fixture.task_path(), &json).unwrap();
            let error = fixture
                .storage
                .load_document()
                .err()
                .expect("rejected")
                .to_string();

            assert!(error.contains("invalid task file"), "{section}: {error}");
            assert_eq!(fs::read_to_string(fixture.task_path()).unwrap(), json);
        }

        // A body that looks like a prefix is just text in its stored section.
        fs::write(fixture.task_path(), task(r#""Work""#, "Home: x")).unwrap();
        let document = fixture.storage.load_document().unwrap();

        assert_eq!(document.tasks[0].section.as_deref(), Some("Work"));
        assert_eq!(document.tasks[0].text, "Home: x");
        assert!(!document.needs_migration);

        // Saving omits `section` for tasks without one.
        fixture.storage.save(&[Task::new("plain")]).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.task_path()).unwrap()).unwrap();
        assert!(saved["tasks"][0].get("section").is_none());
    }

    #[test]
    fn duplicate_ids_and_malformed_identities_are_rejected_without_writing() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.task_path().parent().unwrap()).unwrap();
        let id = "00000000-0000-4000-8000-000000000001";
        let task = |id: &str, created_at: &str| {
            format!(
                r#"{{"id":"{id}","created_at":"{created_at}","text":"task","status":"pending"}}"#
            )
        };
        let time = "2026-11-07T19:24:05Z";

        let duplicate = format!(
            r#"{{"schema_version":5,"tasks":[{},{},{}]}}"#,
            task(id, time),
            task("00000000-0000-4000-8000-000000000002", time),
            task(id, time),
        );
        fs::write(fixture.task_path(), &duplicate).unwrap();
        let error = fixture
            .storage
            .load_document()
            .err()
            .expect("rejected")
            .to_string();

        assert!(error.contains("tasks 1 and 3 have the same id"), "{error}");
        assert!(
            error.contains(id) && error.contains("left untouched"),
            "{error}"
        );
        assert_eq!(fs::read_to_string(fixture.task_path()).unwrap(), duplicate);

        for json in [
            format!(
                r#"{{"schema_version":5,"tasks":[{}]}}"#,
                task("not-a-uuid", time)
            ),
            format!(
                r#"{{"schema_version":5,"tasks":[{}]}}"#,
                task(id, "2026-11-07")
            ),
            format!(
                r#"{{"schema_version":5,"tasks":[{}]}}"#,
                task(id, "2026-02-30T00:00:00Z")
            ),
            r#"{"schema_version":5,"tasks":[{"text":"no id","status":"pending"}]}"#.to_owned(),
            r#"{"schema_version":5,"tasks":["not an object"]}"#.to_owned(),
            r#"{"schema_version":5}"#.to_owned(),
        ] {
            fs::write(fixture.task_path(), &json).unwrap();
            let error = fixture
                .storage
                .load_document()
                .err()
                .expect("rejected")
                .to_string();

            assert!(error.contains("invalid task file"), "{json}: {error}");
            assert_eq!(fs::read_to_string(fixture.task_path()).unwrap(), json);
        }
    }

    #[test]
    fn section_order_must_list_each_task_section_once_in_lowercase() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.task_path().parent().unwrap()).unwrap();
        let document = |order: &str| {
            format!(
                r#"{{"schema_version":5,"tasks":[{{"id":"00000000-0000-4000-8000-000000000001","created_at":"2026-11-07T19:24:05Z","section":"Work","text":"a","status":"pending"}},{{"id":"00000000-0000-4000-8000-000000000002","created_at":"2026-11-07T19:24:05Z","section":"Home","text":"b","status":"pending"}}],"section_order":{order}}}"#
            )
        };

        for order in [
            r#"[]"#,
            r#"["work"]"#,
            r#"["work","home","work"]"#,
            r#"["work","home","gone"]"#,
            r#"["Work","home"]"#,
        ] {
            let json = document(order);
            fs::write(fixture.task_path(), &json).unwrap();

            let error = fixture
                .storage
                .load_document()
                .err()
                .expect("rejected")
                .to_string();

            assert!(error.contains("section_order"), "{order}: {error}");
            assert_eq!(fs::read_to_string(fixture.task_path()).unwrap(), json);
        }

        // Any order of the right sections is kept as saved.
        fs::write(fixture.task_path(), document(r#"["work","home"]"#)).unwrap();
        let loaded = fixture.storage.load_document().unwrap();

        assert_eq!(loaded.section_order, ["work", "home"]);
        assert!(!loaded.needs_migration);
    }

    #[test]
    fn future_and_malformed_schema_versions_are_rejected_without_writing() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.task_path().parent().unwrap()).unwrap();

        let future = r#"{"schema_version":6,"tasks":[],"future":"field"}"#;
        fs::write(fixture.task_path(), future).unwrap();
        let error = fixture
            .storage
            .load_document()
            .err()
            .expect("rejected")
            .to_string();

        assert!(error.contains("schema version 6"), "{error}");
        assert!(error.contains("supports version 5"), "{error}");
        assert!(error.contains("left untouched"), "{error}");
        assert_eq!(fs::read_to_string(fixture.task_path()).unwrap(), future);

        for json in [
            r#"{"schema_version":"1","tasks":[]}"#,
            r#"{"schema_version":-1,"tasks":[]}"#,
            r#"{"schema_version":1.5,"tasks":[]}"#,
            r#"{"schema_version":null,"tasks":[]}"#,
            r#""text""#,
            "42",
        ] {
            fs::write(fixture.task_path(), json).unwrap();
            let error = fixture
                .storage
                .load_document()
                .err()
                .expect("rejected")
                .to_string();

            assert!(error.contains("invalid task file"), "{json}: {error}");
            assert_eq!(fs::read_to_string(fixture.task_path()).unwrap(), json);
        }
    }

    #[test]
    fn older_and_unversioned_files_are_refused_without_writing() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.task_path().parent().unwrap()).unwrap();

        for version in 0..OLDEST_SCHEMA_VERSION {
            let json = format!(r#"{{"schema_version":{version},"tasks":[]}}"#);
            fs::write(fixture.task_path(), &json).unwrap();

            let error = fixture
                .storage
                .load_document()
                .err()
                .expect("rejected")
                .to_string();

            assert!(
                error.contains(&format!("schema version {version}")),
                "{error}"
            );
            assert!(error.contains("only opens version 5 and later"), "{error}");
            assert!(error.contains("left untouched"), "{error}");
            assert_eq!(fs::read_to_string(fixture.task_path()).unwrap(), json);
        }

        for (json, problem) in [
            (r#"{"tasks":[]}"#, "missing schema_version"),
            (r#"[{"text":"a","completed":false}]"#, "expected an object"),
        ] {
            fs::write(fixture.task_path(), json).unwrap();

            let error = fixture
                .storage
                .load_document()
                .err()
                .expect("rejected")
                .to_string();

            assert!(error.contains("invalid task file"), "{json}: {error}");
            assert!(error.contains(problem), "{json}: {error}");
            assert_eq!(fs::read_to_string(fixture.task_path()).unwrap(), json);
        }
    }

    #[test]
    #[cfg(unix)]
    fn new_application_data_is_private_without_changing_parent_permissions() {
        for with_lock in [false, true] {
            let fixture = Fixture::new();
            fs::set_permissions(&fixture.directory, fs::Permissions::from_mode(0o755)).unwrap();
            let _held = with_lock.then(|| fixture.storage.lock().unwrap());
            fixture.storage.save(&[Task::new("Private task")]).unwrap();
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

            let (path, file) = create_temporary(directory, "tasks.json").unwrap();
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
        fs::write(fixture.task_path(), r#"{"schema_version":5,"tasks":[]}"#).unwrap();
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

        let tasks = vec![Task::new("Keep me")];

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
            Task::new("Read 日本語 🪷"),
            Task {
                status: TaskStatus::Completed,
                ..Task::new("Keep this position")
            },
        ];

        fixture.storage.save(&tasks).unwrap();
        assert_eq!(fixture.storage.load().unwrap(), tasks);

        let json = fs::read_to_string(fixture.task_path()).unwrap();
        assert!(json.starts_with("{\n"));
        assert!(json.ends_with('\n'));

        let mut replacement = tasks.clone();
        replacement.reverse();
        replacement[0].status = TaskStatus::Pending;
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
            status: TaskStatus::Completed,
            ..Task::new("Keep the original")
        }];

        fixture.storage.save(&tasks).unwrap();
        let original = fs::read(fixture.task_path()).unwrap();
        let directory = fixture.task_path().parent().unwrap();
        let collision = directory.join(format!(".tasks.json.{}.0.tmp", std::process::id()));
        fs::write(&collision, "old recovery data").unwrap();
        let (temporary, file) = create_temporary(directory, "tasks.json").unwrap();

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
            r#"{"schema_version":5,"tasks":[{"text":"missing identity and status"}]}"#,
            r#"{"schema_version":5,"tasks":[],"extra":1}"#,
        ] {
            fs::write(fixture.task_path(), invalid).unwrap();
            assert!(fixture.storage.load().is_err());
            assert_eq!(fs::read_to_string(fixture.task_path()).unwrap(), invalid);
        }
    }

    #[test]
    fn invalid_task_text_is_rejected_without_changing_the_file() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.task_path().parent().unwrap()).unwrap();

        let invalid = [
            "",
            " ",
            " padded",
            "padded ",
            "\u{2003}padded",
            "padded\u{a0}",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain(
            (0..=0x1f)
                .chain(0x7f..=0x9f)
                .chain([0x2028, 0x2029])
                .map(|code| format!("before{}after", char::from_u32(code).unwrap())),
        );

        for text in invalid {
            let tasks = vec![
                Task::new("valid"),
                Task {
                    status: TaskStatus::Completed,
                    ..Task::new(text)
                },
            ];

            let original = serde_json::to_vec_pretty(
                &serde_json::json!({"schema_version": SCHEMA_VERSION, "tasks": tasks}),
            )
            .unwrap();
            fs::write(fixture.task_path(), &original).unwrap();
            let error = fixture.storage.load().unwrap_err().to_string();

            assert!(error.contains("invalid task file"));
            assert!(error.contains("task 2 text"), "{error}");
            assert!(error.contains("left untouched"));

            if tasks[1].text.contains(['\u{2028}', '\u{2029}']) {
                assert!(error.contains("Unicode line separator"));
            }

            assert_eq!(fs::read(fixture.task_path()).unwrap(), original);
            assert_eq!(
                fs::read_dir(fixture.task_path().parent().unwrap())
                    .unwrap()
                    .count(),
                1
            );
        }
    }

    #[test]
    fn valid_unicode_and_long_task_text_loads_unchanged() {
        let fixture = Fixture::new();

        let tasks = vec![
            Task {
                status: TaskStatus::Completed,
                ..Task::new(
                    "日本語 e\u{301} 👩‍💻 👍🏽 — [punctuation!]  spaces\u{a0}inside\u{2003}text\u{2009}and\u{202f}text",
                )
            },
            Task::new("long🪷".repeat(100_000)),
        ];

        fixture.storage.save(&tasks).unwrap();
        let original = fs::read(fixture.task_path()).unwrap();
        assert_eq!(fixture.storage.load().unwrap(), tasks);
        assert_eq!(fs::read(fixture.task_path()).unwrap(), original);
    }

    #[test]
    fn replacement_failure_keeps_recovery_file() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.task_path()).unwrap();

        let tasks = vec![Task::new("Recover me")];

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
        assert_eq!(saved_tasks(&temporary), tasks);
    }

    #[test]
    #[cfg(windows)]
    fn replacement_denied_by_open_file_preserves_original_and_recovery_data() {
        use std::os::windows::fs::OpenOptionsExt;

        let fixture = Fixture::new();
        fixture
            .storage
            .save(&[Task {
                status: TaskStatus::Completed,
                ..Task::new("Original tasks")
            }])
            .unwrap();
        let original = fs::read(fixture.task_path()).unwrap();

        // Denying delete sharing prevents replacement even on privileged runners.
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(fixture.task_path())
            .unwrap();

        let replacement = vec![Task::new("Recover me")];

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

        assert_eq!(saved_tasks(&temporary), replacement);
    }

    #[test]
    #[cfg(unix)]
    fn directory_sync_failure_reports_visible_replacement_and_uncertain_durability() {
        let fixture = Fixture::new();
        fixture.storage.save(&[Task::new("Old list")]).unwrap();
        let directory = fixture.task_path().parent().unwrap();
        // Write/search permission permits replacement, but opening the directory for sync fails.
        fs::set_permissions(directory, fs::Permissions::from_mode(0o300)).unwrap();

        if File::open(directory).is_ok() {
            // Root/capability-enabled test runners can bypass these permissions.
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
            return;
        }

        let replacement = vec![Task {
            status: TaskStatus::Completed,
            ..Task::new("New list")
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
