use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    // Unique and stable for the task's lifetime, for cross-referencing tasks.
    pub id: Uuid,
    #[serde(with = "utc_seconds")]
    pub created_at: OffsetDateTime,
    // The task's section, such as "Work". This field is the source of truth: `text` holds only
    // the body, and "Work: " is just the syntax for typing and editing it (`display_text`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    pub text: String,
    pub status: TaskStatus,
}

// Where a task stands. Carried tasks are unfinished ones that `D` keeps; a completed task
// can't also be carried, which a single field makes unrepresentable. Stored in lowercase.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Completed,
    Carried,
}

impl Task {
    // A new, unfinished, uncarried task with a fresh UUID v4 and the current time, from text as
    // typed: "Work: fix bug" becomes section "Work" with the body "fix bug".
    pub fn new(input: impl Into<String>) -> Self {
        let input = input.into();
        let (section, text) = split_section(&input);

        Self {
            id: Uuid::new_v4(),
            created_at: current_time(),
            section,
            text,
            status: TaskStatus::Pending,
        }
    }

    // The task as typed: "Work: fix bug", or just the body without a section.
    pub fn display_text(&self) -> String {
        match &self.section {
            Some(section) => format!("{section}: {}", self.text),
            None => self.text.clone(),
        }
    }

    pub(crate) fn validation_error(&self) -> Option<&'static str> {
        if self.section.as_deref().is_some_and(invalid_section_name) {
            Some("section must be one word without spaces, colons, or control characters")
        } else if self.text.is_empty() {
            Some("text is empty")
        } else if self.text.trim() != self.text {
            Some("text has leading or trailing whitespace")
        } else if self.text.chars().any(|c| single_line_character(c) != c) {
            Some("text contains a control character or Unicode line separator; use a single line")
        } else {
            None
        }
    }
}

// The current UTC time in whole seconds, the precision `created_at` is stored with.
pub(crate) fn current_time() -> OffsetDateTime {
    OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("zero nanoseconds is always valid")
}

// `created_at` is stored as whole-second UTC RFC 3339, such as "2026-11-07T19:24:05Z".
mod utc_seconds {
    use serde::{Deserialize, Deserializer, Serializer, de};
    use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

    pub fn serialize<S: Serializer>(
        time: &OffsetDateTime,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        time::serde::rfc3339::serialize(time, serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<OffsetDateTime, D::Error> {
        let text = String::deserialize(deserializer)?;
        let time = OffsetDateTime::parse(&text, &Rfc3339).map_err(de::Error::custom)?;

        // Accept only the canonical form: UTC, whole seconds, and spelled exactly as it would
        // be written back (which rules out "+00:00", lowercase "t"/"z", and similar variants).
        let canonical = time.offset() == UtcOffset::UTC
            && time.nanosecond() == 0
            && time.format(&Rfc3339).ok().as_deref() == Some(text.as_str());

        if !canonical {
            return Err(de::Error::custom(
                "expected a UTC time in whole seconds like 2026-11-07T19:24:05Z",
            ));
        }

        Ok(time)
    }
}

// Stored sections must round-trip through the "Name: " typing syntax: one word without
// spaces, colons, or control characters. Renaming checks typed names with the same rule.
pub(crate) fn invalid_section_name(section: &str) -> bool {
    section.is_empty()
        || section
            .chars()
            .any(|c| c == ':' || c.is_whitespace() || single_line_character(c) != c)
}

pub(crate) fn single_line_character(character: char) -> char {
    if character.is_control() || matches!(character, '\u{2028}' | '\u{2029}') {
        ' '
    } else {
        character
    }
}

// A section is a leading word followed by ": ", as in "Work: Fix bug". Requiring the
// space keeps URLs ("https://...") and times ("10:30 call") from becoming sections.
pub(crate) fn task_prefix(text: &str) -> Option<&str> {
    let end = text.find(':')?;
    let prefix = &text[..end];

    let is_word = end > 0 && !prefix.chars().any(char::is_whitespace);
    (is_word && text[end + 1..].starts_with(' ')).then_some(prefix)
}

// The task text shown inside its own section tab, without the "Work: " prefix.
pub(crate) fn section_body(text: &str) -> &str {
    task_prefix(text).map_or(text, |prefix| text[prefix.len() + 1..].trim_start())
}

// Split typed text into its section and body: "Work: fix bug" → (Some("Work"), "fix bug").
pub(crate) fn split_section(input: &str) -> (Option<String>, String) {
    let section = task_prefix(input).map(str::to_owned);

    (section, section_body(input).to_owned())
}

impl Task {
    // Sections match case-insensitively.
    pub(crate) fn section_key(&self) -> Option<String> {
        self.section.as_deref().map(str::to_lowercase)
    }
}

// The section tab order for `tasks`, starting from `order`: keys that some task still uses
// keep their places, and sections not listed yet follow in list order. This keeps the saved
// order matching the tasks: a section's last task takes it out, and a new one goes last.
pub(crate) fn settled_section_order(order: &[String], tasks: &[Task]) -> Vec<String> {
    let used: Vec<String> = tasks.iter().filter_map(Task::section_key).collect();
    let in_use: HashSet<&String> = used.iter().collect();

    // The sets only answer membership; `settled` keeps the order.
    let mut seen: HashSet<&String> = HashSet::new();
    let mut settled: Vec<String> = Vec::new();

    for key in order.iter().chain(&used) {
        if in_use.contains(key) && seen.insert(key) {
            settled.push(key.clone());
        }
    }

    settled
}

// Tabs in display order for `tasks`: [*], [-] when some tasks have no section, then sections
// in `section_order`. Empty when no task has a section, which hides the tab bar.
pub(crate) fn section_tabs_of(
    tasks: &[Task],
    section_order: &[String],
) -> Vec<(SectionView, String)> {
    // One pass over the tasks: the first spelling of each key in list order labels its
    // tab, and any task without a section brings in [-].
    let mut labels: HashMap<String, &str> = HashMap::new();
    let mut has_unsectioned = false;

    for task in tasks {
        match &task.section {
            Some(section) => {
                labels.entry(section.to_lowercase()).or_insert(section);
            }
            None => has_unsectioned = true,
        }
    }

    let named: Vec<(SectionView, String)> = section_order
        .iter()
        .filter_map(|key| {
            let label = labels.get(key)?;

            Some((SectionView::Named(key.clone()), (*label).to_owned()))
        })
        .collect();

    if named.is_empty() {
        return Vec::new();
    }

    let mut tabs = vec![(SectionView::All, "[*]".to_owned())];

    if has_unsectioned {
        tabs.push((SectionView::Unsectioned, "[-]".to_owned()));
    }

    tabs.extend(named);

    tabs
}

pub(crate) fn view_shows(view: &SectionView, task: &Task) -> bool {
    match view {
        SectionView::All => true,
        SectionView::Named(key) => task.section_key().as_ref() == Some(key),
        SectionView::Unsectioned => task.section.is_none(),
    }
}

// The view that fits `tasks` when `view` is proposed with `selected`. A selection that left
// the proposed tab pulls the view along to the task's own tab (an edit or undo moved it); a
// tab that is gone otherwise falls back to [*]. Removals that empty a tab propose [*]
// themselves. Mutations resolve this before saving, so the saved tab is the one shown.
pub(crate) fn settled_view(
    tasks: &[Task],
    section_order: &[String],
    view: &SectionView,
    selected: Option<usize>,
) -> SectionView {
    let tabs = section_tabs_of(tasks, section_order);
    let mut view = view.clone();

    if !tabs.is_empty()
        && let Some(task) = selected.and_then(|index| tasks.get(index))
        && !view_shows(&view, task)
    {
        view = match task.section_key() {
            Some(key) => SectionView::Named(key),
            None => SectionView::Unsectioned,
        };
    }

    if !tabs.iter().any(|(tab, _)| *tab == view) {
        view = SectionView::All;
    }

    view
}

// Which tasks are shown: all of them, one section (by lowercase key), or unsectioned ones.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum SectionView {
    #[default]
    All,
    Named(String),
    Unsectioned,
}

impl SectionView {
    // Saved in tasks.json: absent for all tasks, "" for unsectioned, else the key.
    pub(crate) fn saved_key(&self) -> Option<&str> {
        match self {
            SectionView::All => None,
            SectionView::Named(key) => Some(key),
            SectionView::Unsectioned => Some(""),
        }
    }

    pub(crate) fn from_saved_key(key: Option<String>) -> Self {
        match key {
            None => SectionView::All,
            Some(key) if key.is_empty() => SectionView::Unsectioned,
            Some(key) => SectionView::Named(key),
        }
    }
}
