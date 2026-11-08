use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, Sender, channel},
    time::{Duration, Instant},
};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::config::{ConfigFile, ConfigText};

// Crossterm's input poll cannot be woken from notify's thread, so the event loop wakes
// this often while idle to collect filesystem events.
const EVENT_INTERVAL: Duration = Duration::from_millis(250);

// One save can produce several events (truncate, write, rename). Reading waits until
// events have been quiet this long, so intermediate contents are never applied.
const SETTLE_DELAY: Duration = Duration::from_millis(150);

// The file is also checked this often regardless of events. That covers a parent
// directory that does not exist yet or was recreated, and platforms without notifications.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

type EventResult = notify::Result<Event>;

// Watches the configuration file's directory rather than the file itself, so creation,
// deletion, and editors that save by renaming a replacement over the file are all seen.
pub struct ConfigWatch {
    file: ConfigFile,
    directory: Option<PathBuf>,
    file_name: Option<OsString>,
    // What the most recent check read; only a different state is reported.
    last: ConfigText,
    sender: Sender<EventResult>,
    events: Receiver<EventResult>,
    // Dropping the watcher stops it, so it lives as long as this struct.
    watcher: Option<RecommendedWatcher>,
    settle_at: Option<Instant>,
    next_poll: Instant,
}

impl ConfigWatch {
    pub fn new(file: ConfigFile, initial: ConfigText, now: Instant) -> Self {
        let (sender, events) = channel();
        let directory = file.path.parent().map(PathBuf::from);
        let file_name = file.path.file_name().map(OsString::from);
        let watcher = directory
            .as_ref()
            .and_then(|directory| watch_directory(directory, &sender));

        Self {
            file,
            directory,
            file_name,
            last: initial,
            sender,
            events,
            watcher,
            settle_at: None,
            next_poll: now + POLL_INTERVAL,
        }
    }

    pub fn file(&self) -> &ConfigFile {
        &self.file
    }

    // How long the event loop may wait for input before calling `check` again.
    pub fn next_check(&self, now: Instant) -> Duration {
        let deadline = match self.settle_at {
            Some(settle_at) => settle_at.min(self.next_poll),
            None => self.next_poll,
        };

        deadline.saturating_duration_since(now).min(EVENT_INTERVAL)
    }

    // Returns the file's new state once it has settled and differs from the last one seen.
    pub fn check(&mut self, now: Instant) -> Option<ConfigText> {
        while let Ok(result) = self.events.try_recv() {
            match result {
                Ok(event) if self.names_directory(&event) => {
                    // The directory itself changed and may be gone or moved, taking the
                    // watch with it. The next poll starts a fresh watch.
                    self.watcher = None;
                    self.settle_at = Some(now + SETTLE_DELAY);
                }
                Ok(event) if self.concerns_file(&event) => {
                    self.settle_at = Some(now + SETTLE_DELAY);
                }
                Ok(_) => {}
                Err(_) => {
                    // Notifications failed; polling continues until a new watch succeeds.
                    self.watcher = None;
                    self.settle_at = Some(now + SETTLE_DELAY);
                }
            }
        }

        if now >= self.next_poll {
            self.next_poll = now + POLL_INTERVAL;

            if self.watcher.is_none() {
                self.watcher = self
                    .directory
                    .as_ref()
                    .and_then(|directory| watch_directory(directory, &self.sender));
            }

            self.settle_at.get_or_insert(now + SETTLE_DELAY);
        }

        if self.settle_at.is_none_or(|settle_at| now < settle_at) {
            return None;
        }

        self.settle_at = None;
        let text = self.file.read();

        if text == self.last {
            return None;
        }

        self.last = text.clone();

        Some(text)
    }

    fn names_directory(&self, event: &Event) -> bool {
        event
            .paths
            .iter()
            .any(|path| Some(path) == self.directory.as_ref())
    }

    // Other files can share the directory (tasks.json on macOS and Windows); only events
    // naming the configuration file, or ones that may have lost track of it, matter.
    fn concerns_file(&self, event: &Event) -> bool {
        event.need_rescan()
            || event.paths.is_empty()
            || event
                .paths
                .iter()
                .any(|path| path.file_name() == self.file_name.as_deref())
    }
}

fn watch_directory(directory: &Path, sender: &Sender<EventResult>) -> Option<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(sender.clone()).ok()?;
    watcher.watch(directory, RecursiveMode::NonRecursive).ok()?;

    Some(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::Fixture;
    use notify::EventKind;
    use std::fs;

    fn watch(path: PathBuf, now: Instant) -> ConfigWatch {
        let file = ConfigFile {
            path,
            required: false,
        };
        let initial = file.read();

        ConfigWatch::new(file, initial, now)
    }

    // Steps past several polls and their settle delays, so results never depend on how
    // quickly (or whether) notify delivers events.
    fn changes(watch: &mut ConfigWatch, now: &mut Instant) -> Vec<ConfigText> {
        let mut changes = Vec::new();

        for _ in 0..3 {
            for step in [POLL_INTERVAL, SETTLE_DELAY] {
                *now += step;
                changes.extend(watch.check(*now));
            }
        }

        changes
    }

    #[test]
    fn creation_edits_atomic_replacement_and_deletion_are_each_reported_once() {
        let fixture = Fixture::new();
        let path = fixture.directory.join("config.toml");
        let mut now = Instant::now();
        let mut watch = watch(path.clone(), now);

        fs::write(&path, "theme = 'nord'").unwrap();
        assert_eq!(
            changes(&mut watch, &mut now),
            [ConfigText::Text("theme = 'nord'".into())]
        );

        let replacement = fixture.directory.join("config.toml.tmp");
        fs::write(&replacement, "theme = 'dracula'").unwrap();
        fs::rename(&replacement, &path).unwrap();
        assert_eq!(
            changes(&mut watch, &mut now),
            [ConfigText::Text("theme = 'dracula'".into())]
        );

        fs::write(&path, "theme = 'dracula'").unwrap();
        assert_eq!(changes(&mut watch, &mut now), []);

        fs::remove_file(&path).unwrap();
        assert_eq!(changes(&mut watch, &mut now), [ConfigText::Missing]);
    }

    #[test]
    fn a_missing_parent_directory_is_watched_once_it_exists() {
        let fixture = Fixture::new();
        let directory = fixture.directory.join("later");
        let mut now = Instant::now();
        let mut watch = watch(directory.join("config.toml"), now);

        assert!(watch.watcher.is_none());
        assert_eq!(changes(&mut watch, &mut now), []);

        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("config.toml"), "").unwrap();

        assert_eq!(
            changes(&mut watch, &mut now),
            [ConfigText::Text(String::new())]
        );
        assert!(watch.watcher.is_some());
    }

    #[test]
    fn only_events_that_may_concern_the_config_file_schedule_a_read() {
        let fixture = Fixture::new();
        let watch = watch(fixture.directory.join("config.toml"), Instant::now());
        let modify = || Event::new(EventKind::Modify(notify::event::ModifyKind::Any));

        assert!(watch.concerns_file(&modify().add_path(fixture.directory.join("config.toml"))));
        assert!(watch.concerns_file(&modify()));
        assert!(!watch.concerns_file(&modify().add_path(fixture.directory.join("tasks.json"))));
        assert!(
            !watch.concerns_file(&modify().add_path(fixture.directory.join(".config.toml.swp")))
        );
    }

    #[test]
    fn the_event_loop_never_sleeps_longer_than_the_event_interval() {
        let fixture = Fixture::new();
        let now = Instant::now();
        let mut watch = watch(fixture.directory.join("config.toml"), now);

        assert_eq!(watch.next_check(now), EVENT_INTERVAL);

        watch.settle_at = Some(now + SETTLE_DELAY);
        assert_eq!(watch.next_check(now), SETTLE_DELAY);
        assert_eq!(watch.next_check(now + POLL_INTERVAL), Duration::ZERO);
    }
}
