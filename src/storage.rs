use std::{
    env,
    error::Error,
    fs::{self, DirBuilder, File, OpenOptions, TryLockError},
    io::{self, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

use atomic_write_file::AtomicWriteFile;
#[cfg(unix)]
use atomic_write_file::unix::OpenOptionsExt as _;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Result;
use crate::model::{Task, reconcile_section_order};

// The `tasks.json` format written by this build. Version 1 is the first public format.
const SCHEMA_VERSION: u64 = 1;

// The oldest `tasks.json` format this build can still upgrade. Older files are refused.
const OLDEST_SCHEMA_VERSION: u64 = 1;

// A migration receives a document without its `schema_version` field, which the loader owns.
type Migration = fn(Value) -> std::result::Result<Value, String>;

// `MIGRATIONS[n]` upgrades a version `OLDEST_SCHEMA_VERSION + n` document to the next version.
// There are none yet: every released format so far is the current one. Migrations run in
// memory on load (`upgrade`), and the upgraded document is written by the next save or clean
// exit, never by loading itself.
//
// To change the format:
//
// 1. Bump `SCHEMA_VERSION`, write `fn vN_to_vM(Value) -> std::result::Result<Value, String>`,
//    and append it here; the array length enforces the pairing. The step first parses version
//    N strictly into its own structs with `deny_unknown_fields`, never today's `Task`, then
//    builds version M, so unexpected or malformed data is rejected instead of carried along.
// 2. Never edit or reorder a released migration: users may hold files at any supported
//    version, and each step must keep meaning exactly what it did when it shipped.
// 3. Raise `OLDEST_SCHEMA_VERSION` only to drop support deliberately. Remove the leading
//    migrations with it and record the last release that can still upgrade those files.
// 4. Test the step directly (exact input JSON to expected output JSON) and add a
//    `load_checklist` round trip of a real version N file; see the notes in `tests.rs`.
const MIGRATIONS: [Migration; (SCHEMA_VERSION - OLDEST_SCHEMA_VERSION) as usize] = [];

// Runs the migrations that take a version `version` document to the newest version, in order.
// `migrations[n]` upgrades version `oldest + n`. The caller has already checked that
// `oldest <= version <= oldest + migrations.len()`. Taking the chain as arguments lets tests
// run it with stand-in migrations.
fn upgrade(
    mut value: Value,
    version: u64,
    oldest: u64,
    migrations: &[Migration],
) -> std::result::Result<Value, String> {
    let applied_from = (version - oldest) as usize;

    for (offset, migrate) in migrations.iter().enumerate().skip(applied_from) {
        let from = oldest + offset as u64;
        value = migrate(value)
            .map_err(|problem| format!("migrating from version {from}: {problem}"))?;
    }

    Ok(value)
}

// The complete checklist that `tasks.json` holds: the tasks and the navigation saved with them.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChecklistDocument {
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub selected: Option<usize>,
    // The active tab's filter: absent for all tasks, "" for unsectioned, else a lowercase key.
    // Each task's own `section` says which section it is in.
    #[serde(default)]
    pub active_view: Option<String>,
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
    // An explicit `--data-dir` is used as given (relative to the working directory), without
    // a `sati` subdirectory; otherwise the platform data directory is discovered.
    pub fn discover(data_dir: Option<&Path>) -> Result<Self> {
        if let Some(directory) = data_dir {
            return Ok(Self {
                path: directory.join("tasks.json"),
            });
        }

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

    pub fn lock(&self) -> Result<File> {
        let directory = self
            .path
            .parent()
            .expect("task file has a parent directory");
        create_data_directory(directory)?;
        let path = directory.join("tasks.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options
            .open(&path)
            .map_err(|error| format!("could not open {}: {error}", path.display()))?;

        // Keep this file open and never unlink it: every instance must lock the same inode.
        match file.try_lock() {
            Ok(()) => Ok(file),
            Err(TryLockError::WouldBlock) => {
                Err(format!("already running: another sati holds {}", path.display()).into())
            }
            Err(TryLockError::Error(error)) => {
                Err(format!("could not lock {}: {error}", path.display()).into())
            }
        }
    }

    #[cfg(test)]
    pub fn load(&self) -> Result<Vec<Task>> {
        Ok(self.load_checklist()?.tasks)
    }

    pub fn load_checklist(&self) -> Result<ChecklistDocument> {
        let contents = match fs::read(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(ChecklistDocument::default());
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
        let value = upgrade(
            Value::Object(fields),
            version,
            OLDEST_SCHEMA_VERSION,
            &MIGRATIONS,
        )
        .map_err(invalid_file)?;

        let mut document: ChecklistDocument =
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
        let expected_order = reconcile_section_order(&document.section_order, &document.tasks);

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
        self.save_checklist(tasks, &reconcile_section_order(&[], tasks), selected, None)
    }

    // Replaces `tasks.json` with the complete checklist: the tasks, their section order, and the
    // saved navigation. `active_view` is a `SectionView::saved_key`.
    pub fn save_checklist(
        &self,
        tasks: &[Task],
        section_order: &[String],
        selected: Option<usize>,
        active_view: Option<&str>,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Saved<'a> {
            schema_version: u64,
            tasks: &'a [Task],
            selected: Option<usize>,
            #[serde(skip_serializing_if = "Option::is_none")]
            active_view: Option<&'a str>,
            section_order: &'a [String],
        }

        atomic_save(
            &self.path,
            &Saved {
                schema_version: SCHEMA_VERSION,
                tasks,
                selected: valid_selection(tasks, selected),
                active_view,
                section_order,
            },
        )
    }
}

fn atomic_save<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    let directory = path.parent().expect("task file has a parent directory");
    create_data_directory(directory)?;

    let mut contents = serde_json::to_vec_pretty(value)?;
    contents.push(b'\n');

    // The live file is never truncated: the crate writes a temporary file beside it, syncs it,
    // renames it over `path`, and on Unix also syncs the directory. A failure before the rename
    // leaves the previous file in place.
    let options = AtomicWriteFile::options();

    // Publish a private file even when replacing one with looser permissions.
    #[cfg(unix)]
    let options = {
        let mut options = options;
        options.preserve_mode(false).mode(0o600);
        options
    };

    let save_error = |error: io::Error| format!("could not save {}: {error}", path.display());
    let mut file = options.open(path).map_err(save_error)?;
    file.write_all(&contents).map_err(save_error)?;
    file.commit().map_err(save_error)?;

    Ok(())
}

fn create_data_directory(directory: &Path) -> Result<()> {
    // Name the path that actually failed, which may be a parent of `directory`. Creating a
    // directory reports AlreadyExists only when something else occupies that path.
    let failure = |path: &Path, error: io::Error| -> Box<dyn Error> {
        let problem = if error.kind() == io::ErrorKind::AlreadyExists {
            "a file that is not a directory is in the way".to_owned()
        } else {
            error.to_string()
        };

        format!("could not create {}: {problem}", path.display()).into()
    };

    if let Some(parent) = directory.parent() {
        fs::create_dir_all(parent).map_err(|error| failure(parent, error))?;
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
        result => result.map_err(|error| failure(directory, error)),
    }
}

#[cfg(test)]
pub mod tests;
