use std::collections::VecDeque;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{layout::Rect, widgets::ListState};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::input::TextInput;
use crate::notification::Notification;
use crate::storage::{Result, Storage};

const HISTORY_LIMIT: usize = 256;

// The name the section picker matches [-] by.
const UNSECTIONED_NAME: &str = "unsectioned";

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
    pub completed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub carried: bool,
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
            completed: false,
            carried: false,
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
        } else if self.completed && self.carried {
            Some("cannot be both completed and carried")
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
fn invalid_section_name(section: &str) -> bool {
    section.is_empty()
        || section
            .chars()
            .any(|c| c == ':' || c.is_whitespace() || single_line_character(c) != c)
}

fn single_line_character(character: char) -> char {
    if character.is_control() || matches!(character, '\u{2028}' | '\u{2029}') {
        ' '
    } else {
        character
    }
}

fn task_noun(count: usize) -> &'static str {
    if count == 1 { "task" } else { "tasks" }
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
    fn section_key(&self) -> Option<String> {
        self.section.as_deref().map(str::to_lowercase)
    }
}

// The section tab order for `tasks`, starting from `order`: keys that some task still uses
// keep their places, and sections not listed yet follow in list order. This keeps the saved
// order matching the tasks: a section's last task takes it out, and a new one goes last.
pub(crate) fn settled_section_order(order: &[String], tasks: &[Task]) -> Vec<String> {
    let used: Vec<String> = tasks.iter().filter_map(Task::section_key).collect();
    let mut settled: Vec<String> = Vec::new();

    for key in order.iter().chain(&used) {
        if used.contains(key) && !settled.contains(key) {
            settled.push(key.clone());
        }
    }

    settled
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
    fn saved_key(&self) -> Option<&str> {
        match self {
            SectionView::All => None,
            SectionView::Named(key) => Some(key),
            SectionView::Unsectioned => Some(""),
        }
    }

    fn from_saved_key(key: Option<String>) -> Self {
        match key {
            None => SectionView::All,
            Some(key) if key.is_empty() => SectionView::Unsectioned,
            Some(key) => SectionView::Named(key),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AddPlacement {
    End,
    Below,
    Above,
}

#[derive(Debug, PartialEq)]
pub enum Mode {
    Normal,
    Adding(TextInput, AddPlacement),
    Editing {
        index: usize,
        input: TextInput,
    },
    Searching {
        input: TextInput,
        original: Option<usize>,
    },
    SearchResults {
        input: TextInput,
        original: Option<usize>,
    },
    // Typing a new name for the current section tab, inside the tab bar.
    RenamingSection {
        input: TextInput,
    },
    // Picking a section tab by fuzzy name, typed into the tab bar in place of [*]. The view
    // previews the focused match; `original` is where Esc returns to.
    FuzzySection {
        input: TextInput,
        // Matching tabs, best first: [-] and named sections, never [*].
        matches: Vec<SectionView>,
        // The query `matches` were found for, which tells edits apart from cursor moves.
        matched_query: String,
        original: NavigationCheckpoint,
    },
    ConfirmClear,
}

// Where navigation stood when the section picker opened. Previews change the view,
// selection, and scrolling freely; cancelling puts all of them back.
#[derive(Debug, PartialEq)]
pub struct NavigationCheckpoint {
    view: SectionView,
    selected: Option<usize>,
    scroll: usize,
    tab_scroll: usize,
    reveal_selection: bool,
}

// A pending section suggestion while adding or editing, shown as dim text after the
// cursor. It only exists while the input is a single partial word with the cursor at its end.
struct Completion {
    labels: Vec<String>,
    current: usize,
}

#[cfg_attr(test, derive(Clone, Debug, PartialEq))]
struct UndoState {
    tasks: Vec<Task>,
    section_order: Vec<String>,
    selected: Option<usize>,
    // Describes the change from this snapshot to the next state, e.g. "delete task".
    label: String,
}

pub struct App {
    pub tasks: Vec<Task>,
    // Lowercase keys of the named section tabs in display order; see `settled_section_order`.
    pub section_order: Vec<String>,
    pub list: ListState,
    pub mode: Mode,
    pub wrap: bool,
    pub notifications: bool,
    pub keybinds: bool,
    pub rounded: bool,
    pub unicode_markers: bool,
    pub styles: crate::styles::Styles,
    pub theme: crate::themes::BuiltinTheme,
    pub style_overrides: crate::config::StyleOverrides,
    pub cli_accent: Option<ratatui::style::Color>,
    pub scroll: usize,
    // A temporary inline add row has no task until it is confirmed.
    pub row_tasks: Vec<Option<usize>>,
    pub viewport_height: u16,
    pub reveal_selection: bool,
    pub notification: Option<Notification>,
    pub view: SectionView,
    // Clickable tab regions from the last draw.
    pub tab_targets: Vec<(Rect, SectionView)>,
    // The first visible tab when they overflow; kept between draws so tabs don't slide.
    pub tab_scroll: usize,
    // A Vim-style count typed before a motion; transient and never saved.
    pending_count: Option<usize>,
    // Set by `s` / Alt+S: the next key picks a tab by number. Transient and never saved.
    pending_section_jump: bool,
    completion: Option<Completion>,
    task_register: Option<String>,
    last_search_query: Option<String>,
    history: VecDeque<UndoState>,
    redo_history: VecDeque<UndoState>,
    persisted_selection: Option<usize>,
    persisted_view: SectionView,
    needs_migration: bool,
}

impl App {
    pub fn new(tasks: Vec<Task>) -> Self {
        let selected = (!tasks.is_empty()).then_some(0);
        // Without a saved order, tabs start alphabetically, as they were before orders were
        // saved (and as `v3_to_v4` migrates them).
        let mut section_order = settled_section_order(&[], &tasks);
        section_order.sort();

        Self {
            tasks,
            section_order,
            list: ListState::default().with_selected(selected),
            mode: Mode::Normal,
            wrap: true,
            notifications: true,
            keybinds: true,
            rounded: false,
            unicode_markers: true,
            styles: crate::styles::Styles::default(),
            theme: crate::themes::BuiltinTheme::default(),
            style_overrides: crate::config::StyleOverrides::default(),
            cli_accent: None,
            scroll: 0,
            row_tasks: Vec::new(),
            viewport_height: 0,
            reveal_selection: true,
            notification: None,
            view: SectionView::All,
            tab_targets: Vec::new(),
            tab_scroll: 0,
            pending_count: None,
            pending_section_jump: false,
            completion: None,
            task_register: None,
            last_search_query: None,
            history: VecDeque::new(),
            redo_history: VecDeque::new(),
            persisted_selection: selected,
            persisted_view: SectionView::All,
            needs_migration: false,
        }
    }

    pub fn from_document(document: crate::storage::TaskDocument) -> Self {
        let mut app = Self::new(document.tasks);
        app.section_order = document.section_order;
        app.list.select(document.selected);
        app.persisted_selection = document.selected;
        app.view = SectionView::from_saved_key(document.section);
        app.persisted_view = app.view.clone();
        app.needs_migration = document.needs_migration;

        if !app.section_tabs().iter().any(|(view, _)| *view == app.view) {
            app.view = SectionView::All;
        }

        app.settle_view();

        app
    }

    pub fn save_selection(&mut self, storage: &Storage) -> Result<()> {
        // Temporary search and section-picker focus is never saved.
        let (selected, view) = match &self.mode {
            Mode::Searching { original, .. } | Mode::SearchResults { original, .. } => {
                (*original, self.view.clone())
            }
            Mode::FuzzySection { original, .. } => (original.selected, original.view.clone()),
            _ => (self.list.selected(), self.view.clone()),
        };

        let selected = crate::storage::valid_selection(&self.tasks, selected);

        if self.needs_migration
            || selected != self.persisted_selection
            || view != self.persisted_view
        {
            storage.save_view(&self.tasks, &self.section_order, selected, view.saved_key())?;
            self.persisted_selection = selected;
            self.persisted_view = view;
            self.needs_migration = false;
        }

        Ok(())
    }

    // Tabs in display order: [*], [-] when some tasks have no section, then sections in
    // `section_order`. Empty when no task has a section, which hides the tab bar.
    pub fn section_tabs(&self) -> Vec<(SectionView, String)> {
        let named: Vec<(SectionView, String)> = self
            .section_order
            .iter()
            .filter_map(|key| {
                // The first spelling in list order labels the tab.
                let label = self
                    .tasks
                    .iter()
                    .filter_map(|task| task.section.as_deref())
                    .find(|section| section.to_lowercase() == *key)?;

                Some((SectionView::Named(key.clone()), label.to_owned()))
            })
            .collect();

        if named.is_empty() {
            return Vec::new();
        }

        let mut tabs = vec![(SectionView::All, "[*]".to_owned())];

        if self.tasks.iter().any(|task| task.section.is_none()) {
            tabs.push((SectionView::Unsectioned, "[-]".to_owned()));
        }

        tabs.extend(named);

        tabs
    }

    pub fn is_visible(&self, index: usize) -> bool {
        let task = &self.tasks[index];

        match &self.view {
            SectionView::All => true,
            SectionView::Named(key) => task.section_key().as_ref() == Some(key),
            SectionView::Unsectioned => task.section.is_none(),
        }
    }

    fn visible_indices(&self) -> Vec<usize> {
        (0..self.tasks.len())
            .filter(|&index| self.is_visible(index))
            .collect()
    }

    // Search matches what a tab shows, so a section tab ignores its own prefix.
    fn search_text(&self, index: usize) -> std::borrow::Cow<'_, str> {
        let task = &self.tasks[index];

        match self.view {
            SectionView::Named(_) => task.text.as_str().into(),
            _ => task.display_text().into(),
        }
    }

    // The tab a task belongs to, given that sections exist.
    fn home_view(&self, index: usize) -> SectionView {
        match self.tasks[index].section_key() {
            Some(key) => SectionView::Named(key),
            None => SectionView::Unsectioned,
        }
    }

    // Keep the view valid after tasks change. A selection that left the current tab pulls
    // the view along to the task's own tab (an edit or undo moved it); a tab that is gone
    // otherwise falls back to [*]. Removals that empty a tab switch to [*] themselves.
    fn settle_view(&mut self) {
        let tabs = self.section_tabs();

        if !tabs.is_empty()
            && let Some(index) = self.list.selected()
            && index < self.tasks.len()
            && !self.is_visible(index)
        {
            self.view = self.home_view(index);
        }

        if !tabs.iter().any(|(view, _)| *view == self.view) {
            self.view = SectionView::All;
        }
    }

    pub fn show_section(&mut self, view: SectionView) {
        self.view = view;

        let selection_visible = self
            .list
            .selected()
            .is_some_and(|index| index < self.tasks.len() && self.is_visible(index));

        if !selection_visible {
            self.list.select(self.visible_indices().first().copied());
        }

        self.reveal_selection = true;
    }

    fn cycle_section(&mut self, forward: bool) {
        let tabs = self.section_tabs();

        if tabs.is_empty() {
            self.notify("No sections yet (start a task with \"Name: \")");

            return;
        }

        let current = tabs
            .iter()
            .position(|(view, _)| *view == self.view)
            .unwrap_or(0);

        let next = if forward {
            (current + 1) % tabs.len()
        } else {
            (current + tabs.len() - 1) % tabs.len()
        };

        self.show_section(tabs[next].0.clone());
    }

    // `s1` through `s9` pick the first nine tabs in display order and `s0` the tenth.
    fn jump_to_section(&mut self, digit: char) {
        let position = match digit.to_digit(10) {
            Some(0) => 9,
            Some(number) => number as usize - 1,
            None => return,
        };

        if let Some((view, _)) = self.section_tabs().get(position) {
            self.show_section(view.clone());
        }
    }

    // Section tabs whose names fuzzy-match `query`, best first. Equal scores, and an empty
    // query, keep the tab order. [-] is found as "unsectioned", and [*] is never a match.
    fn section_matches(&self, query: &str) -> Vec<SectionView> {
        let candidates: Vec<(SectionView, String)> = self
            .section_tabs()
            .into_iter()
            .filter_map(|(view, label)| match view {
                SectionView::All => None,
                SectionView::Unsectioned => Some((view, UNSECTIONED_NAME.to_owned())),
                SectionView::Named(_) => Some((view, label)),
            })
            .collect();

        if query.is_empty() {
            return candidates.into_iter().map(|(view, _)| view).collect();
        }

        // `Pattern::new` takes the query literally: no `^`, `$`, `!`, or `'` syntax.
        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut buffer = Vec::new();

        let mut scored: Vec<(SectionView, u32)> = candidates
            .into_iter()
            .filter_map(|(view, name)| {
                let score = pattern.score(Utf32Str::new(&name, &mut buffer), &mut matcher)?;
                Some((view, score))
            })
            .collect();

        // A stable sort, so equal scores stay in tab order.
        scored.sort_by_key(|&(_, score)| std::cmp::Reverse(score));

        scored.into_iter().map(|(view, _)| view).collect()
    }

    // Open the section picker without moving: the current tab and selection stay as they are
    // until the query or Tab picks another tab.
    fn start_section_picker(&mut self) {
        if self.section_tabs().is_empty() {
            self.notify("No sections yet (start a task with \"Name: \")");

            return;
        }

        let original = NavigationCheckpoint {
            view: self.view.clone(),
            selected: self.list.selected(),
            scroll: self.scroll,
            tab_scroll: self.tab_scroll,
            reveal_selection: self.reveal_selection,
        };

        self.mode = Mode::FuzzySection {
            input: TextInput::default(),
            matches: self.section_matches(""),
            matched_query: String::new(),
            original,
        };
    }

    // After the picker's query changes, rank the sections again and preview the best match.
    // Cursor moves leave the query, and so the focused match, alone. With no matches, the
    // last preview stays on screen.
    fn refresh_section_picker(&mut self) {
        let Mode::FuzzySection {
            input,
            matched_query,
            ..
        } = &self.mode
        else {
            return;
        };

        if input.text() == matched_query {
            return;
        }

        let query = input.text().to_owned();
        let found = self.section_matches(&query);
        let best = found.first().cloned();

        if let Mode::FuzzySection {
            matches,
            matched_query,
            ..
        } = &mut self.mode
        {
            *matches = found;
            *matched_query = query;
        }

        if let Some(best) = best {
            self.show_section(best);
        }
    }

    // The focused match is the previewed tab. When that isn't a match (the picker opened on
    // [*]), stepping starts from the first or last match.
    fn cycle_section_match(&mut self, forward: bool) {
        let Mode::FuzzySection { matches, .. } = &self.mode else {
            return;
        };

        if matches.is_empty() {
            return;
        }

        let count = matches.len();
        let current = matches.iter().position(|view| *view == self.view);

        let next = match (current, forward) {
            (Some(current), true) => (current + 1) % count,
            (Some(current), false) => (current + count - 1) % count,
            (None, true) => 0,
            (None, false) => count - 1,
        };

        let view = matches[next].clone();
        self.show_section(view);
    }

    // Accepting keeps the previewed tab, saved on a clean exit like any other navigation.
    // Cancelling restores the view, selection, and scrolling from before the picker opened.
    fn finish_section_picker(&mut self, accept: bool) {
        let Mode::FuzzySection { original, .. } = std::mem::replace(&mut self.mode, Mode::Normal)
        else {
            return;
        };

        if accept {
            return;
        }

        self.view = original.view;
        self.list.select(original.selected);
        self.scroll = original.scroll;
        self.tab_scroll = original.tab_scroll;
        self.reveal_selection = original.reveal_selection;
    }

    // Swap the current section tab with its neighbor. [*] and [-] stay where they are, and the
    // ends do not wrap around.
    fn move_section(&mut self, right: bool, storage: &Storage) -> Result<()> {
        let SectionView::Named(key) = &self.view else {
            return Ok(());
        };

        let Some(position) = self.section_order.iter().position(|named| named == key) else {
            return Ok(());
        };

        let target = if right {
            position + 1
        } else {
            match position.checked_sub(1) {
                Some(target) => target,
                None => return Ok(()),
            }
        };

        if target >= self.section_order.len() {
            return Ok(());
        }

        let mut section_order = self.section_order.clone();
        section_order.swap(position, target);

        // Only the tab order changes; the tasks and their order in [*] stay as they are.
        self.replace_with_section_order(
            self.tasks.clone(),
            section_order,
            self.list.selected(),
            "move section",
            storage,
        )
    }

    fn section_labels(&self) -> Vec<String> {
        self.section_tabs()
            .into_iter()
            .filter(|(view, _)| matches!(view, SectionView::Named(_)))
            .map(|(_, label)| label)
            .collect()
    }

    // The input typed so far when it can still become a section: one word, cursor at the end.
    fn completion_partial(&self) -> Option<&str> {
        let (Mode::Adding(input, _) | Mode::Editing { input, .. }) = &self.mode else {
            return None;
        };

        let text = input.text();
        let one_word = !text.contains(char::is_whitespace) && !text.contains(':');

        (one_word && input.cursor() == text.len()).then_some(text)
    }

    fn cycle_completion(&mut self, forward: bool) {
        if let Some(completion) = &mut self.completion {
            let count = completion.labels.len();

            completion.current = if forward {
                (completion.current + 1) % count
            } else {
                (completion.current + count - 1) % count
            };

            return;
        }

        let Some(partial) = self.completion_partial() else {
            return;
        };

        let partial = partial.to_lowercase();
        let all = self.section_labels();

        let labels: Vec<String> = all
            .iter()
            .filter(|label| label.to_lowercase().starts_with(&partial))
            .cloned()
            .collect();

        if labels.is_empty() {
            if all.is_empty() {
                self.notify("No sections yet (start a task with \"Name: \")");
            } else {
                self.notify("No matching sections");
            }

            return;
        }

        let current = if forward { 0 } else { labels.len() - 1 };
        self.completion = Some(Completion { labels, current });
    }

    // Replace the partial word with the chosen section, ready for the task text.
    fn accept_completion(&mut self) {
        let Some(completion) = self.completion.take() else {
            return;
        };

        if let Mode::Adding(input, _) | Mode::Editing { input, .. } = &mut self.mode {
            let label = &completion.labels[completion.current];
            *input = format!("{label}: ").into();
        }
    }

    // The dim text drawn after the cursor: the rest of the suggested "Section: ".
    pub fn completion_ghost(&self) -> Option<String> {
        let completion = self.completion.as_ref()?;
        let label = &completion.labels[completion.current];
        let typed = self.completion_partial().unwrap_or("").to_lowercase();

        let start = label
            .char_indices()
            .map(|(index, _)| index)
            .chain([label.len()])
            .find(|&index| label[..index].to_lowercase() == typed)
            .unwrap_or(0);

        Some(format!("{}: ", &label[start..]))
    }

    pub fn navigate(&mut self, forward: bool, count: Option<usize>) {
        let visible = self.visible_indices();

        if visible.is_empty() {
            return;
        }

        // Positions within the visible tasks; in [*] they equal task indices.
        let last = visible.len() - 1;
        let current = self
            .list
            .selected()
            .and_then(|selected| visible.iter().position(|&index| index == selected))
            .unwrap_or(0);

        // Like Vim, a counted motion stops at the ends; a plain motion wraps around.
        let next = match (count, forward) {
            (Some(count), true) => current.saturating_add(count).min(last),
            (Some(count), false) => current.saturating_sub(count),
            (None, true) if current == last => 0,
            (None, true) => current + 1,
            (None, false) if current == 0 => last,
            (None, false) => current - 1,
        };

        self.list.select(Some(visible[next]));
        self.reveal_selection = true;
    }

    pub fn insertion_index(&self, placement: AddPlacement) -> usize {
        match placement {
            AddPlacement::End => self.tasks.len(),
            AddPlacement::Below => self
                .list
                .selected()
                .map_or(self.tasks.len(), |index| index + 1),
            AddPlacement::Above => self.list.selected().unwrap_or(self.tasks.len()),
        }
    }

    fn insert_task(
        &mut self,
        text: String,
        placement: AddPlacement,
        label: &str,
        storage: &Storage,
    ) -> Result<()> {
        let index = self.insertion_index(placement);
        let mut tasks = self.tasks.clone();
        tasks.insert(index, Task::new(text));

        self.replace(tasks, Some(index), label, storage)
    }

    fn page(&mut self, forward: bool) {
        let visible = self.visible_indices();

        if visible.is_empty() {
            return;
        }

        let position = |task: usize| visible.iter().position(|&index| index == task);
        let last = visible.len() - 1;
        let current = self.list.selected().unwrap_or(visible[0]);
        let current_position = position(current).unwrap_or(0);

        let start = self
            .row_tasks
            .iter()
            .position(|&task| task == Some(current))
            .unwrap_or(0);

        // Anchor within a tall task when its beginning has scrolled offscreen.
        let anchor = if self.row_tasks.get(self.scroll) == Some(&Some(current)) {
            self.scroll
        } else {
            start
        };

        let height = usize::from(self.viewport_height.max(1));

        let target = if forward {
            anchor.saturating_add(height)
        } else {
            anchor.saturating_sub(height)
        };

        let candidate = self
            .row_tasks
            .get(target.min(self.row_tasks.len().saturating_sub(1)))
            .copied()
            .flatten()
            .and_then(position)
            .unwrap_or(current_position)
            .min(last);

        // A page landing in the same wrapped task must still make task-level progress.
        let next = if forward {
            candidate.max((current_position + 1).min(last))
        } else {
            candidate.min(current_position.saturating_sub(1))
        };

        if next != current_position {
            self.list.select(Some(visible[next]));
            self.scroll = if forward {
                self.scroll.saturating_add(height)
            } else {
                self.scroll.saturating_sub(height)
            };
            self.reveal_selection = true;
        }
    }

    fn replace(
        &mut self,
        tasks: Vec<Task>,
        selected: Option<usize>,
        label: impl Into<String>,
        storage: &Storage,
    ) -> Result<()> {
        let section_order = self.section_order.clone();

        self.replace_with_section_order(tasks, section_order, selected, label, storage)
    }

    // Update memory only after a successful save. Any save error exits the event loop;
    // after a post-replacement error, the next run reloads the file already on disk.
    // `section_order` is settled against `tasks`, so new sections join at the end.
    fn replace_with_section_order(
        &mut self,
        tasks: Vec<Task>,
        section_order: Vec<String>,
        selected: Option<usize>,
        label: impl Into<String>,
        storage: &Storage,
    ) -> Result<()> {
        let section_order = settled_section_order(&section_order, &tasks);

        storage.save_view(&tasks, &section_order, selected, self.view.saved_key())?;
        self.persisted_selection = crate::storage::valid_selection(&tasks, selected);
        self.persisted_view = self.view.clone();
        self.needs_migration = false;

        if self.history.len() == HISTORY_LIMIT {
            self.history.pop_front();
        }

        self.history.push_back(UndoState {
            tasks: std::mem::replace(&mut self.tasks, tasks),
            section_order: std::mem::replace(&mut self.section_order, section_order),
            selected: self.list.selected(),
            label: label.into(),
        });
        self.redo_history.clear();
        self.list.select(selected);
        self.reveal_selection = true;
        self.settle_view();

        Ok(())
    }

    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_history.is_empty()
    }

    pub fn search_query(&self) -> Option<&str> {
        match &self.mode {
            Mode::Searching { input, .. } | Mode::SearchResults { input, .. } => Some(input.text()),
            _ => None,
        }
    }

    fn start_search(&mut self) {
        self.mode = Mode::Searching {
            input: TextInput::default(),
            original: self.list.selected(),
        };
        self.list.select(None);
        self.reveal_selection = true;
    }

    fn refresh_search(&mut self) {
        let Mode::Searching { input, original } = &self.mode else {
            return;
        };

        if !input.text().is_empty() && self.last_search_query.as_deref() != Some(input.text()) {
            self.last_search_query = Some(input.text().to_owned());
        }

        let anchor = original.unwrap_or(0).min(self.tasks.len());
        let selected = (anchor..self.tasks.len()).chain(0..anchor).find(|&index| {
            self.is_visible(index) && crate::search::matches(&self.search_text(index), input.text())
        });
        self.list.select(selected);
        self.reveal_selection = true;
    }

    fn find_search_result(&self, query: &str, forward: bool) -> Option<usize> {
        let len = self.tasks.len();
        let matches = |&index: &usize| {
            self.is_visible(index) && crate::search::matches(&self.search_text(index), query)
        };

        if forward {
            let start = self.list.selected().map_or(0, |index| index + 1);
            (start..len).chain(0..start).find(matches)
        } else {
            let end = self.list.selected().unwrap_or(len);
            (0..end).rev().chain((end..len).rev()).find(matches)
        }
    }

    fn search_next(&mut self, forward: bool) {
        if self.list.selected().is_none() {
            return;
        }

        let Some(query) = self.search_query() else {
            return;
        };

        let selected = self.find_search_result(query, forward);
        self.list.select(selected);
        self.reveal_selection = true;
    }

    fn resume_search(&mut self, forward: bool) {
        let Some(query) = self.last_search_query.as_deref() else {
            self.notify("No previous search");

            return;
        };

        let original = self.list.selected();
        let selected = original
            .filter(|&index| {
                self.is_visible(index) && crate::search::matches(&self.search_text(index), query)
            })
            .or_else(|| self.find_search_result(query, forward));

        let Some(index) = selected else {
            self.notify(format!("No matches for \"{query}\""));

            return;
        };

        self.mode = Mode::SearchResults {
            input: query.into(),
            original,
        };
        self.list.select(Some(index));
        self.reveal_selection = true;
    }

    fn finish_search(&mut self, accept: bool) {
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);

        if let Mode::Searching { input, original } | Mode::SearchResults { input, original } = mode
        {
            if !input.text().is_empty() {
                self.last_search_query = Some(input.text().to_owned());
            }

            if !accept {
                self.list.select(original);
            }

            self.reveal_selection = true;
        }
    }

    fn undo(&mut self, storage: &Storage) -> Result<()> {
        self.restore_snapshot(storage, false)
    }

    fn redo(&mut self, storage: &Storage) -> Result<()> {
        self.restore_snapshot(storage, true)
    }

    fn restore_snapshot(&mut self, storage: &Storage, redo: bool) -> Result<()> {
        let (source, destination) = if redo {
            (&mut self.redo_history, &mut self.history)
        } else {
            (&mut self.history, &mut self.redo_history)
        };

        let Some(previous) = source.back() else {
            self.notify(if redo {
                "Nothing to redo"
            } else {
                "Nothing to undo"
            });

            return Ok(());
        };

        storage.save_view(
            &previous.tasks,
            &previous.section_order,
            previous.selected,
            self.view.saved_key(),
        )?;
        self.persisted_selection =
            crate::storage::valid_selection(&previous.tasks, previous.selected);
        self.persisted_view = self.view.clone();
        self.needs_migration = false;
        let previous = source.pop_back().unwrap();

        if destination.len() == HISTORY_LIMIT {
            destination.pop_front();
        }

        let verb = if redo { "Redid" } else { "Undid" };
        let message = format!("{verb}: {}", previous.label);

        // The label describes the same change in either direction, so it moves with it.
        destination.push_back(UndoState {
            tasks: std::mem::replace(&mut self.tasks, previous.tasks),
            section_order: std::mem::replace(&mut self.section_order, previous.section_order),
            selected: self.list.selected(),
            label: previous.label,
        });
        self.list
            .select(previous.selected.filter(|&index| index < self.tasks.len()));
        self.reveal_selection = true;
        self.settle_view();
        self.notify(message);

        Ok(())
    }

    fn toggle(&mut self, index: usize, storage: &Storage) -> Result<()> {
        let mut tasks = self.tasks.clone();
        tasks[index].completed = !tasks[index].completed;
        tasks[index].carried = false;

        let label = if tasks[index].completed {
            "complete task"
        } else {
            "uncomplete task"
        };

        // The task stays in place, so do not force the viewport to scroll to it.
        self.replace(tasks, Some(index), label, storage)?;
        self.reveal_selection = false;

        Ok(())
    }

    fn toggle_carryover(&mut self, storage: &Storage) -> Result<()> {
        let Some(index) = self.list.selected() else {
            return Ok(());
        };

        if self.tasks[index].completed {
            self.notify("Completed tasks can't be carried");

            return Ok(());
        }

        let mut tasks = self.tasks.clone();
        tasks[index].carried = !tasks[index].carried;

        let label = if tasks[index].carried {
            "mark carryover"
        } else {
            "unmark carryover"
        };

        self.replace(tasks, Some(index), label, storage)
    }

    fn clear_unmarked(&mut self, storage: &Storage) -> Result<()> {
        // Only the current tab's unmarked tasks are cleared; other tabs are untouched.
        let survivors: Vec<usize> = (0..self.tasks.len())
            .filter(|&index| !self.is_visible(index) || self.tasks[index].carried)
            .collect();
        let removed = self.tasks.len() - survivors.len();

        if removed == 0 {
            return Ok(());
        }

        // Keep the closest surviving task in this tab, otherwise the closest anywhere.
        let anchor = self.list.selected().unwrap_or(0);
        let closest = |pool: &[usize]| {
            pool.iter()
                .copied()
                .min_by_key(|index| index.abs_diff(anchor))
        };

        let visible_survivors: Vec<usize> = survivors
            .iter()
            .copied()
            .filter(|&index| self.is_visible(index))
            .collect();
        let kept = visible_survivors.len();
        let chosen = closest(&visible_survivors).or_else(|| closest(&survivors));
        let selected = chosen.and_then(|old| survivors.iter().position(|&index| index == old));

        let tasks = survivors
            .iter()
            .map(|&index| self.tasks[index].clone())
            .collect();
        let cleared = self.tasks_phrase(removed, "");

        // Emptying the current tab returns to [*].
        if visible_survivors.is_empty() {
            self.view = SectionView::All;
        }

        self.replace(tasks, selected, format!("clear {cleared}"), storage)?;

        if kept == 0 {
            self.notify(format!("Cleared {cleared}"));
        } else {
            self.notify(format!("Cleared {cleared}, kept {kept} carried"));
        }

        Ok(())
    }

    fn move_selected(&mut self, down: bool, count: usize, storage: &Storage) -> Result<()> {
        let Some(index) = self.list.selected() else {
            return Ok(());
        };

        // Count positions among the visible tasks, then move to the passed task's slot.
        let visible = self.visible_indices();

        let Some(position) = visible.iter().position(|&visible| visible == index) else {
            return Ok(());
        };

        let target_position = if down {
            position.saturating_add(count).min(visible.len() - 1)
        } else {
            position.saturating_sub(count)
        };

        if target_position == position {
            return Ok(());
        }

        let target = visible[target_position];

        // One remove/insert keeps a counted move to a single save and undo step.
        let mut tasks = self.tasks.clone();
        let task = tasks.remove(index);
        tasks.insert(target, task);

        self.replace(tasks, Some(target), "move task", storage)
    }

    fn clear_completed(&mut self, storage: &Storage) -> Result<()> {
        // Only the current tab's completed tasks are cleared; other tabs are untouched.
        let survivors: Vec<usize> = (0..self.tasks.len())
            .filter(|&index| !self.is_visible(index) || !self.tasks[index].completed)
            .collect();
        let removed = self.tasks.len() - survivors.len();

        if removed == 0 {
            let section = self.scope_label().map(|label| format!("{label} "));
            self.notify(format!("No completed {}tasks", section.unwrap_or_default()));

            return Ok(());
        }

        // Keep the selected task if it survives, otherwise the next survivor or the previous
        // one, preferring survivors in this tab.
        let anchor = self.list.selected().unwrap_or(0);
        let pick = |pool: &[usize]| {
            pool.iter()
                .copied()
                .find(|&index| index >= anchor)
                .or_else(|| pool.last().copied())
        };

        let visible_survivors: Vec<usize> = survivors
            .iter()
            .copied()
            .filter(|&index| self.is_visible(index))
            .collect();
        let chosen = pick(&visible_survivors).or_else(|| pick(&survivors));
        let selected = chosen.and_then(|old| survivors.iter().position(|&index| index == old));

        let tasks = survivors
            .iter()
            .map(|&index| self.tasks[index].clone())
            .collect();
        let cleared = self.tasks_phrase(removed, "completed ");

        // Emptying the current tab returns to [*].
        if visible_survivors.is_empty() {
            self.view = SectionView::All;
        }

        self.replace(tasks, selected, format!("clear {cleared}"), storage)?;
        self.notify(format!("Cleared {cleared}"));

        Ok(())
    }

    // How the current tab is named in messages: its section label, "unsectioned", or
    // nothing in [*].
    pub fn scope_label(&self) -> Option<String> {
        match &self.view {
            SectionView::All => None,
            SectionView::Unsectioned => Some("unsectioned".to_owned()),
            SectionView::Named(_) => self
                .section_tabs()
                .into_iter()
                .find(|(view, _)| *view == self.view)
                .map(|(_, label)| label),
        }
    }

    // "2 completed Work tasks", "1 task", ...
    fn tasks_phrase(&self, count: usize, kind: &str) -> String {
        let section = self
            .scope_label()
            .map(|label| format!("{label} "))
            .unwrap_or_default();

        format!("{count} {kind}{section}{}", task_noun(count))
    }

    fn start_rename_section(&mut self) {
        if self.section_tabs().is_empty() {
            self.notify("No sections yet (start a task with \"Name: \")");

            return;
        }

        let (SectionView::Named(_), Some(label)) = (&self.view, self.scope_label()) else {
            self.notify("Switch to a section tab to rename it");

            return;
        };

        self.mode = Mode::RenamingSection {
            input: label.into(),
        };
    }

    // Why the typed name can't be used, if it can't. The tab's own name in another case is
    // fine: that only changes the spelling.
    pub fn rename_problem(&self) -> Option<&'static str> {
        let (Mode::RenamingSection { input }, SectionView::Named(current)) =
            (&self.mode, &self.view)
        else {
            return None;
        };

        let name = input.text().trim();

        if name.is_empty() {
            return Some("Type a section name");
        }

        if invalid_section_name(name) {
            return Some("Section names are one word without colons");
        }

        let key = name.to_lowercase();
        let taken = key != *current
            && self
                .tasks
                .iter()
                .any(|task| task.section_key().as_ref() == Some(&key));

        taken.then_some("Section name already taken")
    }

    // Shown on every key while renaming, even with notifications off, because it explains
    // why Enter does nothing.
    fn explain_rename_problem(&mut self) {
        if let Some(problem) = self.rename_problem() {
            self.notification = Some(Notification::new(problem));
        }
    }

    // Give every task in the current section tab the new name, then follow it to its tab.
    fn rename_section(&mut self, name: &str, storage: &Storage) -> Result<()> {
        let SectionView::Named(old_key) = self.view.clone() else {
            return Ok(());
        };

        let old_label = self.scope_label().unwrap_or_default();
        let mut tasks = self.tasks.clone();
        let mut renamed = 0;

        for task in &mut tasks {
            if task.section_key().as_ref() == Some(&old_key) {
                task.section = Some(name.to_owned());
                renamed += 1;
            }
        }

        // The same spelling on every task is not a change.
        if tasks == self.tasks {
            return Ok(());
        }

        // The renamed section keeps its place among the tabs.
        let new_key = name.to_lowercase();

        let mut section_order = self.section_order.clone();

        if let Some(key) = section_order.iter_mut().find(|key| **key == old_key) {
            *key = new_key.clone();
        }

        // The new tab is saved with the renamed tasks; a failed save keeps the old one.
        let old_view = std::mem::replace(&mut self.view, SectionView::Named(new_key));

        let renamed_tasks = self.replace_with_section_order(
            tasks,
            section_order,
            self.list.selected(),
            "rename section",
            storage,
        );

        if let Err(error) = renamed_tasks {
            self.view = old_view;

            return Err(error);
        }

        self.notify(format!(
            "Renamed {old_label} to {name} ({renamed} {})",
            task_noun(renamed)
        ));

        Ok(())
    }

    fn cycle_theme(&mut self, forward: bool) {
        self.theme = self.theme.cycle(forward);
        self.styles =
            crate::styles::Styles::resolve(self.theme, &self.style_overrides, self.cli_accent);
        self.notify(format!(
            "Theme: {} ({})",
            self.theme.display_name(),
            self.theme.id()
        ));
    }

    fn yank_task(&mut self) {
        if let Some(task) = self.list.selected().and_then(|index| self.tasks.get(index)) {
            self.task_register = Some(task.display_text());
            self.notify("Copied task");
        }
    }

    fn paste_task(&mut self, placement: AddPlacement, storage: &Storage) -> Result<()> {
        let Some(text) = self.task_register.clone() else {
            self.notify("Nothing to paste");

            return Ok(());
        };

        self.insert_task(text, placement, "paste task", storage)
    }

    pub fn copy_markdown(&mut self, output: &mut impl std::io::Write) {
        // Inside a tab, copy only what that tab shows.
        let visible: Vec<Task> = self
            .visible_indices()
            .into_iter()
            .map(|index| self.tasks[index].clone())
            .collect();

        match crate::export::copy(&visible, output) {
            Ok(()) => self.notify("Copy sent to terminal"),
            // Failures are always reported, even when confirmations are disabled.
            Err(error) => {
                self.notification = Some(Notification::new(format!("Copy failed: {error}")));
            }
        }
    }

    fn notify(&mut self, message: impl Into<String>) {
        if self.notifications {
            self.notification = Some(Notification::new(message));
        }
    }

    pub fn key(&mut self, key: KeyEvent, storage: &Storage) -> Result<bool> {
        let key = self.normalize_key(key);

        if key.kind == KeyEventKind::Release
            || (key.kind == KeyEventKind::Repeat && !self.repeat_allowed(key))
        {
            return Ok(false);
        }

        self.notification = None;

        // Every accepted key consumes a pending count, even shortcuts that ignore it.
        let count = self.pending_count.take();

        // After `s` / Alt+S, a digit picks a tab. Any other key only cancels the jump, so a
        // mistyped follow-up never runs its own, possibly destructive, shortcut.
        if std::mem::take(&mut self.pending_section_jump) {
            if let KeyCode::Char(digit @ '0'..='9') = key.code
                && key.modifiers.is_empty()
            {
                self.jump_to_section(digit);
            }

            return Ok(false);
        }

        // Modifier shortcuts edit the input directly, so a pending suggestion is dropped.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            self.completion = None;
        }

        if self.consume_modified_key(key, storage)? {
            self.refresh_section_picker();
            self.explain_rename_problem();
            return Ok(false);
        }

        let quit = match self.mode {
            Mode::Normal => self.handle_normal_key(key, count, storage)?,
            Mode::Adding(_, _)
            | Mode::Editing { .. }
            | Mode::Searching { .. }
            | Mode::RenamingSection { .. }
            | Mode::FuzzySection { .. } => {
                self.handle_text_input_key(key, storage)?;
                false
            }
            Mode::SearchResults { .. } => {
                self.handle_search_results_key(key);
                false
            }
            Mode::ConfirmClear => {
                self.handle_confirm_clear_key(key, storage)?;
                false
            }
        };

        self.refresh_section_picker();
        self.explain_rename_problem();

        Ok(quit)
    }

    fn normalize_key(&self, key: KeyEvent) -> KeyEvent {
        // Route cancellation through Esc, including its press/repeat policy.
        if self.mode != Mode::Normal
            && key.modifiers == KeyModifiers::CONTROL
            && matches!(key.code, KeyCode::Char('g' | 'G'))
        {
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
                ..key
            }
        } else if key.modifiers == KeyModifiers::CONTROL
            && matches!(self.mode, Mode::Normal | Mode::SearchResults { .. })
            && matches!(key.code, KeyCode::Char('p' | 'P' | 'n' | 'N'))
        {
            KeyEvent {
                code: if matches!(key.code, KeyCode::Char('p' | 'P')) {
                    KeyCode::Up
                } else {
                    KeyCode::Down
                },
                modifiers: KeyModifiers::NONE,
                ..key
            }
        } else {
            key
        }
    }

    fn repeat_allowed(&self, key: KeyEvent) -> bool {
        match &self.mode {
            Mode::Normal
                if matches!(
                    key.code,
                    KeyCode::Tab | KeyCode::BackTab | KeyCode::Char('n' | 'N')
                ) =>
            {
                key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
            }
            Mode::Normal => {
                key.modifiers.is_empty()
                    && matches!(
                        key.code,
                        KeyCode::Char('j' | 'k' | 'h' | 'l')
                            | KeyCode::Up
                            | KeyCode::Down
                            | KeyCode::PageUp
                            | KeyCode::PageDown
                    )
            }
            // Stepping through picker matches repeats like switching tabs in Normal mode.
            Mode::FuzzySection { .. } if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) => {
                true
            }
            Mode::Adding(_, _)
            | Mode::Editing { .. }
            | Mode::Searching { .. }
            | Mode::RenamingSection { .. }
            | Mode::FuzzySection { .. } => {
                matches!(
                    key.code,
                    KeyCode::Char(_)
                        | KeyCode::Backspace
                        | KeyCode::Delete
                        | KeyCode::Left
                        | KeyCode::Right
                        | KeyCode::Home
                        | KeyCode::End
                )
            }
            Mode::SearchResults { .. } => matches!(
                key.code,
                KeyCode::Char('n' | 'N') | KeyCode::Up | KeyCode::Down
            ),
            Mode::ConfirmClear => false,
        }
    }

    // Returns true when the event was consumed, including ignored modifier combinations.
    fn consume_modified_key(&mut self, key: KeyEvent, storage: &Storage) -> Result<bool> {
        // Windows reports AltGr text as Ctrl+Alt; the backend cannot distinguish
        // it from a literal Ctrl+Alt character shortcut. Accept it only in input modes.
        if cfg!(windows)
            && key.modifiers.difference(KeyModifiers::SHIFT)
                == (KeyModifiers::CONTROL | KeyModifiers::ALT)
            && let (
                Mode::Adding(input, _)
                | Mode::Editing { input, .. }
                | Mode::Searching { input, .. }
                | Mode::RenamingSection { input }
                | Mode::FuzzySection { input, .. },
                KeyCode::Char(character),
            ) = (&mut self.mode, key.code)
            && !character.is_control()
        {
            input.insert(single_line_character(character).encode_utf8(&mut [0; 4]));
            self.refresh_search();
            return Ok(true);
        }

        if key.modifiers == KeyModifiers::ALT
            && matches!(key.code, KeyCode::Char('b' | 'B' | 'f' | 'F' | 'd' | 'D'))
            && let Mode::Adding(input, _)
            | Mode::Editing { input, .. }
            | Mode::Searching { input, .. }
            | Mode::RenamingSection { input }
            | Mode::FuzzySection { input, .. } = &mut self.mode
        {
            match key.code {
                KeyCode::Char('b' | 'B') => input.move_word_left(),
                KeyCode::Char('f' | 'F') => input.move_word_right(),
                KeyCode::Char('d' | 'D') => input.delete_word_after(),
                _ => unreachable!(),
            }

            self.refresh_search();
            return Ok(true);
        }

        // Outside text input, Alt+B/F switch sections the way they move between words.
        if self.mode == Mode::Normal && key.modifiers == KeyModifiers::ALT {
            match key.code {
                KeyCode::Char('w' | 'W') => self.yank_task(),
                KeyCode::Char('b' | 'B') => self.cycle_section(false),
                KeyCode::Char('f' | 'F') => self.cycle_section(true),
                KeyCode::Char('p' | 'P') => self.move_section(false, storage)?,
                KeyCode::Char('n' | 'N') => self.move_section(true, storage)?,
                KeyCode::Char('s' | 'S') => self.pending_section_jump = true,
                _ => {}
            }

            return Ok(true);
        }

        // Only the explicitly supported input shortcuts may consume other Control events.
        if key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return Ok(true);
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if key.modifiers == KeyModifiers::CONTROL {
                if self.mode == Mode::Normal {
                    match key.code {
                        KeyCode::Char('z' | 'Z') => self.undo(storage)?,
                        KeyCode::Char('r' | 'R') => self.redo(storage)?,
                        KeyCode::Char('y' | 'Y') => {
                            self.paste_task(AddPlacement::Below, storage)?
                        }
                        KeyCode::Char('f' | 'F') => self.start_search(),
                        _ => {}
                    }

                    return Ok(true);
                }

                // In the section picker, Ctrl+Left/Right step through matches like
                // Shift+Tab/Tab; every other input keeps them for moving by word.
                if matches!(self.mode, Mode::FuzzySection { .. })
                    && matches!(key.code, KeyCode::Left | KeyCode::Right)
                {
                    self.cycle_section_match(key.code == KeyCode::Right);

                    return Ok(true);
                }

                if let Mode::Adding(input, _)
                | Mode::Editing { input, .. }
                | Mode::Searching { input, .. }
                | Mode::RenamingSection { input }
                | Mode::FuzzySection { input, .. } = &mut self.mode
                {
                    match key.code {
                        KeyCode::Char('a' | 'A') => input.set_cursor(0),
                        KeyCode::Char('e' | 'E') => input.set_cursor(input.text().len()),
                        KeyCode::Left => input.move_word_left(),
                        KeyCode::Right => input.move_word_right(),
                        KeyCode::Backspace | KeyCode::Char('w' | 'W') => input.delete_word_before(),
                        KeyCode::Char('b' | 'B') => input.move_left(),
                        KeyCode::Char('f' | 'F') => input.move_right(),
                        KeyCode::Char('d' | 'D') => input.delete_after(),
                        KeyCode::Char('k' | 'K') => input.delete_to_end(),
                        KeyCode::Char('u' | 'U') => input.delete_to_start(),
                        _ => {}
                    }
                }
            }

            self.refresh_search();
            return Ok(true);
        }

        Ok(false)
    }

    fn handle_text_input_key(&mut self, key: KeyEvent, storage: &Storage) -> Result<()> {
        if matches!(self.mode, Mode::Searching { .. }) {
            if key.code == KeyCode::Esc {
                self.finish_search(false);
                return Ok(());
            }

            if key.code == KeyCode::Enter {
                if self.search_query() == Some("") {
                    self.finish_search(false);
                } else if self.list.selected().is_some()
                    && let Mode::Searching { input, original } =
                        std::mem::replace(&mut self.mode, Mode::Normal)
                {
                    let single_match = (0..self.tasks.len())
                        .filter(|&index| {
                            self.is_visible(index)
                                && crate::search::matches(&self.search_text(index), input.text())
                        })
                        .take(2)
                        .count()
                        == 1;

                    self.mode = Mode::SearchResults { input, original };

                    if single_match {
                        self.finish_search(true);
                    }
                }

                return Ok(());
            }
        }

        if let Mode::FuzzySection { matches, .. } = &self.mode {
            let forward = key.code == KeyCode::Tab && !key.modifiers.contains(KeyModifiers::SHIFT);

            // Other keys edit the query below, like any other input.
            match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    self.cycle_section_match(forward);
                    return Ok(());
                }
                // With no matches there is nothing to pick; Esc still cancels.
                KeyCode::Enter => {
                    if !matches.is_empty() {
                        self.finish_section_picker(true);
                    }

                    return Ok(());
                }
                KeyCode::Esc => {
                    self.finish_section_picker(false);
                    return Ok(());
                }
                _ => {}
            }
        }

        if matches!(self.mode, Mode::Adding(..) | Mode::Editing { .. }) {
            let forward = key.code == KeyCode::Tab && !key.modifiers.contains(KeyModifiers::SHIFT);

            match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    self.cycle_completion(forward);
                    return Ok(());
                }
                _ if self.completion.is_none() => {}
                KeyCode::Char(' ') | KeyCode::Enter => {
                    self.accept_completion();
                    return Ok(());
                }
                // Typing continues after the suggested section.
                KeyCode::Char(character) if !character.is_control() => self.accept_completion(),
                KeyCode::Backspace | KeyCode::Esc => {
                    self.completion = None;
                    return Ok(());
                }
                _ => self.completion = None,
            }
        }

        let placement = match self.mode {
            Mode::Adding(_, placement) => placement,
            _ => AddPlacement::End,
        };

        let edited_task = match &self.mode {
            Mode::Editing { index, .. } => Some(*index),
            _ => None,
        };

        let renaming = matches!(self.mode, Mode::RenamingSection { .. });

        let (Mode::Adding(input, _)
        | Mode::Editing { input, .. }
        | Mode::Searching { input, .. }
        | Mode::RenamingSection { input }
        | Mode::FuzzySection { input, .. }) = &mut self.mode
        else {
            unreachable!("text input mode");
        };

        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.reveal_selection = true;
            }
            KeyCode::Backspace => input.delete_before(),
            KeyCode::Delete => input.delete_after(),
            KeyCode::Left => input.move_left(),
            KeyCode::Right => input.move_right(),
            KeyCode::Home => input.set_cursor(0),
            KeyCode::End => input.set_cursor(input.text().len()),
            KeyCode::Char(character) if !character.is_control() => {
                input.insert(single_line_character(character).encode_utf8(&mut [0; 4]))
            }
            // A taken or invalid name keeps the input open; the notice says why.
            KeyCode::Enter if renaming => {
                let name = input.text().trim().to_owned();

                if self.rename_problem().is_none() {
                    self.rename_section(&name, storage)?;
                    self.mode = Mode::Normal;
                    self.reveal_selection = true;
                }
            }
            KeyCode::Enter => {
                let text = input.text().trim().to_owned();
                // A chosen section with nothing after it is not a task.
                let bare_section =
                    task_prefix(input.text()).is_some() && section_body(input.text()).is_empty();

                if !text.is_empty() && !bare_section {
                    if let Some(index) = edited_task {
                        // Editing re-reads the "Name: " syntax, so it can change the section.
                        let (section, body) = split_section(&text);

                        if self.tasks[index].section != section || self.tasks[index].text != body {
                            let mut tasks = self.tasks.clone();
                            tasks[index].section = section;
                            tasks[index].text = body;
                            self.replace(tasks, Some(index), "edit task", storage)?;
                        }
                    } else {
                        self.insert_task(text, placement, "add task", storage)?;
                    }
                }

                self.mode = Mode::Normal;
                self.reveal_selection = true;
            }
            _ => {}
        }

        self.refresh_search();

        Ok(())
    }

    fn handle_search_results_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.finish_search(false),
            KeyCode::Enter => self.finish_search(true),
            KeyCode::Char('N') | KeyCode::Up => self.search_next(false),
            KeyCode::Char('n') => self.search_next(!key.modifiers.contains(KeyModifiers::SHIFT)),
            KeyCode::Down => self.search_next(true),
            _ => {}
        }
    }

    fn handle_confirm_clear_key(&mut self, key: KeyEvent, storage: &Storage) -> Result<()> {
        match key.code {
            KeyCode::Enter => {
                self.clear_unmarked(storage)?;
                self.mode = Mode::Normal;
            }
            KeyCode::Esc => self.mode = Mode::Normal,
            _ => {}
        }

        Ok(())
    }

    fn handle_normal_key(
        &mut self,
        key: KeyEvent,
        count: Option<usize>,
        storage: &Storage,
    ) -> Result<bool> {
        match key.code {
            // Like Vim, 1-9 start a count and 0 only continues one.
            KeyCode::Char(digit @ '0'..='9') if digit != '0' || count.is_some() => {
                let value = digit.to_digit(10).unwrap() as usize;
                let count = count.unwrap_or(0).saturating_mul(10).saturating_add(value);
                self.pending_count = Some(count);
            }
            KeyCode::Char('/') => self.start_search(),
            KeyCode::Char('`') => self.start_section_picker(),
            KeyCode::Char('n' | 'N')
                if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() =>
            {
                self.resume_search(
                    key.code == KeyCode::Char('n') && !key.modifiers.contains(KeyModifiers::SHIFT),
                );
            }
            KeyCode::Char('t' | 'T')
                if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() =>
            {
                self.cycle_theme(
                    key.code == KeyCode::Char('t') && !key.modifiers.contains(KeyModifiers::SHIFT),
                );
            }
            KeyCode::Char('y') => self.yank_task(),
            KeyCode::Char('p' | 'P') => {
                let above =
                    key.code == KeyCode::Char('P') || key.modifiers.contains(KeyModifiers::SHIFT);
                self.paste_task(
                    if above {
                        AddPlacement::Above
                    } else {
                        AddPlacement::Below
                    },
                    storage,
                )?;
            }
            KeyCode::Char('c') => self.copy_markdown(&mut std::io::stdout().lock()),
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('u') => self.undo(storage)?,
            KeyCode::Char('w') => {
                self.wrap = !self.wrap;
                self.reveal_selection = true;
                self.notify(if self.wrap { "Wrap on" } else { "Wrap off" });
            }
            KeyCode::Char('m') => self.toggle_carryover(storage)?,
            KeyCode::Char('x') => self.clear_completed(storage)?,
            KeyCode::Char('J') => self.move_selected(true, count.unwrap_or(1), storage)?,
            KeyCode::Char('K') => self.move_selected(false, count.unwrap_or(1), storage)?,
            KeyCode::Char('j') | KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.move_selected(true, count.unwrap_or(1), storage)?;
            }
            KeyCode::Char('k') | KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.move_selected(false, count.unwrap_or(1), storage)?;
            }
            KeyCode::Char('j') | KeyCode::Down => self.navigate(true, count),
            KeyCode::Char('k') | KeyCode::Up => self.navigate(false, count),
            KeyCode::Char('H') => self.move_section(false, storage)?,
            KeyCode::Char('L') => self.move_section(true, storage)?,
            KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.move_section(false, storage)?;
            }
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.move_section(true, storage)?;
            }
            KeyCode::Char('h') => self.cycle_section(false),
            KeyCode::Char('l') => self.cycle_section(true),
            KeyCode::Char('s') => self.pending_section_jump = true,
            KeyCode::Tab | KeyCode::BackTab
                if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() =>
            {
                self.cycle_section(
                    key.code == KeyCode::Tab && !key.modifiers.contains(KeyModifiers::SHIFT),
                );
            }
            KeyCode::PageUp => self.page(false),
            KeyCode::PageDown => self.page(true),
            KeyCode::Char('g' | 'G') | KeyCode::Home | KeyCode::End if !self.tasks.is_empty() => {
                let first = key.code == KeyCode::Home
                    || (key.code == KeyCode::Char('g')
                        && !key.modifiers.contains(KeyModifiers::SHIFT));
                let visible = self.visible_indices();
                let index = if first {
                    visible.first()
                } else {
                    visible.last()
                };
                self.list.select(index.copied());
                self.reveal_selection = true;
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                if let Some(index) = self.list.selected() {
                    self.toggle(index, storage)?;
                }
            }
            KeyCode::Char('a' | 'o' | 'O') => {
                let placement = match key.code {
                    KeyCode::Char('a') => AddPlacement::End,
                    KeyCode::Char('O') => AddPlacement::Above,
                    _ if key.modifiers.contains(KeyModifiers::SHIFT) => AddPlacement::Above,
                    _ => AddPlacement::Below,
                };

                self.mode = Mode::Adding(TextInput::default(), placement);

                // A section tab starts new tasks with its own section suggested.
                if let Some(current) = self.scope_label()
                    && matches!(self.view, SectionView::Named(_))
                {
                    let labels = self.section_labels();
                    let current = labels.iter().position(|label| *label == current);

                    self.completion = current.map(|current| Completion { labels, current });
                }
            }
            KeyCode::Char('e') => {
                if let Some(index) = self.list.selected() {
                    self.mode = Mode::Editing {
                        index,
                        input: self.tasks[index].display_text().into(),
                    };
                }
            }
            KeyCode::Char('E') => self.start_rename_section(),
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(index) = self.list.selected() {
                    let visible = self.visible_indices();
                    let position = visible.iter().position(|&visible| visible == index);
                    let mut tasks = self.tasks.clone();
                    tasks.remove(index);

                    // Prefer the next visible task, then the previous one. Later indices
                    // shift down by one after the removal.
                    let next = position.and_then(|position| visible.get(position + 1));
                    let previous = position
                        .and_then(|position| position.checked_sub(1))
                        .map(|position| visible[position]);

                    let visible_neighbor = next.map(|&next| next - 1).or(previous);

                    // Deleting a tab's last task returns to [*].
                    if visible_neighbor.is_none() {
                        self.view = SectionView::All;
                    }

                    let selected = visible_neighbor
                        .or_else(|| (!tasks.is_empty()).then(|| index.min(tasks.len() - 1)));

                    self.replace(tasks, selected, "delete task", storage)?;
                    self.notify("Deleted task");
                }
            }
            KeyCode::Char('D')
                if (0..self.tasks.len())
                    .any(|index| self.is_visible(index) && !self.tasks[index].carried) =>
            {
                self.mode = Mode::ConfirmClear
            }
            _ => {}
        }

        self.refresh_search();

        Ok(false)
    }

    pub fn paste(&mut self, text: &str) {
        self.notification = None;
        self.pending_count = None;
        self.pending_section_jump = false;
        self.accept_completion();

        if let Mode::Adding(input, _)
        | Mode::Editing { input, .. }
        | Mode::Searching { input, .. }
        | Mode::RenamingSection { input }
        | Mode::FuzzySection { input, .. } = &mut self.mode
        {
            let normalized: String = text
                .replace("\r\n", "\n")
                .chars()
                .map(single_line_character)
                .collect();

            input.insert(&normalized);
        }

        self.refresh_search();
        self.refresh_section_picker();
        self.explain_rename_problem();
    }

    pub fn mouse(&mut self, event: MouseEvent, rows: Rect, storage: &Storage) -> Result<()> {
        self.notification = None;
        self.pending_count = None;
        self.pending_section_jump = false;

        if self.mode == Mode::Normal
            && event.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some((_, view)) = self
                .tab_targets
                .iter()
                .find(|(area, _)| area.contains((event.column, event.row).into()))
        {
            self.show_section(view.clone());

            return Ok(());
        }

        if self.mode != Mode::Normal || !rows.contains((event.column, event.row).into()) {
            return Ok(());
        }

        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(index) = self
                    .row_tasks
                    .get(self.scroll + usize::from(event.row - rows.y))
                    .copied()
                    .flatten()
                {
                    self.toggle(index, storage)?;
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let height = usize::from(rows.height);
                self.scroll = if event.kind == MouseEventKind::ScrollDown {
                    self.scroll.saturating_add(1)
                } else {
                    self.scroll.saturating_sub(1)
                }
                .min(self.row_tasks.len().saturating_sub(height));
                let end = (self.scroll + height).min(self.row_tasks.len());
                let visible = &self.row_tasks[self.scroll..end];

                if let Some(selected) = self.list.selected()
                    && !visible.contains(&Some(selected))
                {
                    self.list
                        .select(if event.kind == MouseEventKind::ScrollDown {
                            visible.iter().rev().find_map(|&task| task)
                        } else {
                            visible.iter().find_map(|&task| task)
                        });
                }

                self.reveal_selection = false;
            }
            _ => {}
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::Fixture;

    use unicode_segmentation::UnicodeSegmentation;

    fn app_with_tasks(count: usize) -> App {
        App::new(
            (0..count)
                .map(|index| Task::new(format!("task {index}")))
                .collect(),
        )
    }

    // KeyCode means an unmodified press; explicit events retain modifiers and kind.
    #[track_caller]
    fn app_with_texts(texts: &[&str]) -> App {
        App::new(texts.iter().map(|text| Task::new(*text)).collect())
    }

    // Tasks as typed, including any "Name: " section prefix.
    fn texts(app: &App) -> Vec<String> {
        app.tasks.iter().map(Task::display_text).collect()
    }

    fn press(app: &mut App, fixture: &Fixture, key: impl Into<KeyEvent>) -> bool {
        app.key(key.into(), &fixture.storage).unwrap()
    }

    fn repeat(app: &mut App, fixture: &Fixture, code: KeyCode, modifiers: KeyModifiers) {
        let key = KeyEvent::new_with_kind(code, modifiers, KeyEventKind::Repeat);
        assert!(!press(app, fixture, key));
    }

    #[derive(Debug, PartialEq)]
    struct TaskState {
        tasks: Vec<Task>,
        selected: Option<usize>,
        history: VecDeque<UndoState>,
        redo_history: VecDeque<UndoState>,
    }

    fn notice(app: &App) -> Option<&str> {
        app.notification.as_ref().map(Notification::message)
    }

    fn task_state(app: &App) -> TaskState {
        TaskState {
            tasks: app.tasks.clone(),
            selected: app.list.selected(),
            history: app.history.clone(),
            redo_history: app.redo_history.clone(),
        }
    }

    #[track_caller]
    fn block_saves(fixture: &Fixture) {
        let data = fixture.directory.join("data");
        std::fs::rename(&data, fixture.directory.join("backup")).unwrap();
        std::fs::write(data, "blocked").unwrap();
    }

    #[track_caller]
    fn restore_saves(fixture: &Fixture) {
        std::fs::remove_file(fixture.directory.join("data")).unwrap();
        std::fs::rename(
            fixture.directory.join("backup"),
            fixture.directory.join("data"),
        )
        .unwrap();
    }

    #[test]
    fn selection_migrates_on_clean_save_and_tracks_mutations_without_navigation_writes() {
        let fixture = Fixture::new();
        let mut empty = App::from_document(fixture.storage.load_document().unwrap());
        empty.save_selection(&fixture.storage).unwrap();

        assert!(!fixture.task_path().exists());

        fixture.storage.save(&[]).unwrap();
        std::fs::write(fixture.task_path(), "[]").unwrap();
        let mut empty = App::from_document(fixture.storage.load_document().unwrap());
        empty.save_selection(&fixture.storage).unwrap();

        assert!(!fixture.storage.load_document().unwrap().needs_migration);

        let tasks = app_with_tasks(3).tasks;
        fixture.storage.save(&tasks).unwrap();
        // A real legacy file: a bare array of tasks without ids or creation times.
        let legacy = serde_json::to_vec(
            &tasks
                .iter()
                .map(|task| serde_json::json!({"text": task.text, "completed": task.completed}))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        std::fs::write(fixture.task_path(), &legacy).unwrap();
        let mut app = App::from_document(fixture.storage.load_document().unwrap());

        assert_eq!(std::fs::read(fixture.task_path()).unwrap(), legacy);

        block_saves(&fixture);
        assert!(app.save_selection(&fixture.storage).is_err());
        assert!(app.needs_migration);

        restore_saves(&fixture);
        app.save_selection(&fixture.storage).unwrap();
        assert!(!app.needs_migration && !app.can_undo());
        assert!(!fixture.storage.load_document().unwrap().needs_migration);

        // A clean quit writes the migrated, versioned document.
        let saved = std::fs::read_to_string(fixture.task_path()).unwrap();
        assert!(saved.contains("\"schema_version\": 4"), "{saved}");

        app.list.select(Some(2));
        assert_eq!(fixture.storage.load_document().unwrap().selected, Some(0));

        block_saves(&fixture);
        assert!(app.save_selection(&fixture.storage).is_err());
        assert_eq!(app.persisted_selection, Some(0));
        assert_eq!(app.list.selected(), Some(2));

        restore_saves(&fixture);
        app.save_selection(&fixture.storage).unwrap();
        let restarted = App::from_document(fixture.storage.load_document().unwrap());

        assert_eq!(restarted.list.selected(), Some(2));
        assert!(!restarted.can_undo() && !restarted.can_redo());

        block_saves(&fixture);
        app.save_selection(&fixture.storage).unwrap(); // No redundant write.
        app.start_search();
        app.paste("task 0");
        app.save_selection(&fixture.storage).unwrap(); // Original settled focus is still 2.
        restore_saves(&fixture);
        app.finish_search(true);
        app.save_selection(&fixture.storage).unwrap();

        assert_eq!(fixture.storage.load_document().unwrap().selected, Some(0));

        app.list.select(Some(2));
        press(&mut app, &fixture, KeyCode::Char('d'));
        assert_eq!(fixture.storage.load_document().unwrap().selected, Some(1));

        app.undo(&fixture.storage).unwrap();
        assert_eq!(fixture.storage.load_document().unwrap().selected, Some(2));

        app.redo(&fixture.storage).unwrap();
        assert_eq!(fixture.storage.load_document().unwrap().selected, Some(1));

        app.toggle(0, &fixture.storage).unwrap();
        app.undo(&fixture.storage).unwrap();
        block_saves(&fixture);
        app.save_selection(&fixture.storage).unwrap();
        app.list.select(Some(0));
        let before = task_state(&app);

        assert!(app.toggle(0, &fixture.storage).is_err());
        assert!(app.undo(&fixture.storage).is_err());
        assert!(app.redo(&fixture.storage).is_err());
        assert_eq!(task_state(&app), before);
        assert_eq!(app.persisted_selection, Some(1));
    }

    #[test]
    fn modifier_early_returns_preserve_search_refresh_and_notification_dismissal() {
        let fixture = Fixture::new();

        for (modifiers, refresh) in [
            (KeyModifiers::CONTROL, true),
            (KeyModifiers::CONTROL | KeyModifiers::SHIFT, true),
            (KeyModifiers::ALT, false),
            (KeyModifiers::SUPER, false),
        ] {
            for kind in [
                KeyEventKind::Press,
                KeyEventKind::Repeat,
                KeyEventKind::Release,
            ] {
                let mut app = app_with_tasks(3);
                app.tasks[0].text = "hit first".into();
                app.tasks[2].text = "hit second".into();
                app.mode = Mode::Searching {
                    input: "hit".into(),
                    original: Some(0),
                };
                app.list.select(Some(2));
                app.notification = Some(Notification::new("Copied task"));
                press(
                    &mut app,
                    &fixture,
                    KeyEvent::new_with_kind(KeyCode::Char('z'), modifiers, kind),
                );
                let accepted = kind != KeyEventKind::Release;

                assert_eq!(app.notification.is_none(), accepted);
                assert_eq!(
                    app.list.selected(),
                    Some(if accepted && refresh { 0 } else { 2 })
                );
                assert_eq!(
                    app.last_search_query.as_deref(),
                    if accepted && refresh {
                        Some("hit")
                    } else {
                        None
                    }
                );
                assert!(
                    matches!(&app.mode, Mode::Searching { input, .. } if input.text() == "hit")
                );
                assert!(!app.can_undo() && !app.can_redo());
            }
        }

        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn restart_keeps_saved_tasks_but_never_restores_or_creates_history_files() {
        for legacy in [false, true] {
            let fixture = Fixture::new();
            let mut app = app_with_tasks(3);
            app.toggle(0, &fixture.storage).unwrap();
            app.toggle(1, &fixture.storage).unwrap();
            app.undo(&fixture.storage).unwrap();

            assert!(app.can_undo() && app.can_redo());

            let path = fixture.task_path().with_file_name("history.json");
            let bytes = b"legacy history deliberately left untouched";

            if legacy {
                std::fs::write(&path, bytes).unwrap();
            }

            app.list.select(Some(2));
            let tasks = app.tasks.clone();
            drop(app);
            let mut restarted = App::new(fixture.storage.load().unwrap());

            assert_eq!(restarted.tasks, tasks);
            assert_eq!(restarted.list.selected(), Some(0));
            assert!(!restarted.can_undo() && !restarted.can_redo());

            press(&mut restarted, &fixture, KeyCode::Char('u'));
            restarted.redo(&fixture.storage).unwrap();
            assert_eq!(restarted.tasks, tasks);
            assert_eq!(fixture.storage.load().unwrap(), tasks);

            restarted.toggle(2, &fixture.storage).unwrap();
            restarted.undo(&fixture.storage).unwrap();
            assert_eq!(fixture.storage.load().unwrap(), tasks);

            restarted.redo(&fixture.storage).unwrap();
            assert_eq!(fixture.storage.load().unwrap(), restarted.tasks);

            drop(restarted);

            if legacy {
                assert_eq!(std::fs::read(path).unwrap(), bytes);
            } else {
                assert!(!path.exists());
            }
        }
    }

    #[test]
    fn disabled_notifications_hide_confirmations_but_still_report_copy_failures() {
        struct BrokenOutput;
        impl std::io::Write for BrokenOutput {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("output unavailable"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let fixture = Fixture::new();
        let mut app = app_with_tasks(2);
        app.notifications = false;
        let theme = app.theme;

        press(&mut app, &fixture, KeyCode::Char('y'));
        assert_eq!(app.task_register.as_deref(), Some("task 0"));
        assert!(app.notification.is_none());

        press(&mut app, &fixture, KeyCode::Char('t'));
        assert_ne!(app.theme, theme);
        assert!(app.notification.is_none());

        let mut output = Vec::new();
        app.copy_markdown(&mut output);
        let mut expected = Vec::new();
        crate::export::copy(&app.tasks, &mut expected).unwrap();

        assert_eq!(output, expected);
        assert!(app.notification.is_none());

        for key in ['u', 'w', 'd', 'x', 'p'] {
            press(&mut app, &fixture, KeyCode::Char(key));
            assert!(app.notification.is_none(), "{key}");
        }

        app.copy_markdown(&mut BrokenOutput);

        assert!(
            app.notification
                .as_ref()
                .is_some_and(|notice| notice.message().starts_with("Copy failed:"))
        );
    }

    #[test]
    fn clipboard_output_and_errors_leave_tasks_selection_and_histories_unchanged() {
        struct BrokenOutput;
        impl std::io::Write for BrokenOutput {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("output unavailable"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        press(&mut app, &fixture, KeyCode::Char(' '));
        press(&mut app, &fixture, KeyCode::Char('j'));
        press(&mut app, &fixture, KeyCode::Char('m'));
        press(&mut app, &fixture, KeyCode::Char('u'));

        assert!(app.can_undo() && app.can_redo());

        let tasks = app.tasks.clone();
        let selected = app.list.selected();
        let undo = app.history.clone();
        let redo = app.redo_history.clone();
        let saved = fixture.storage.load().unwrap();
        app.scroll = 2;
        let mut output = Vec::new();
        app.copy_markdown(&mut output);
        let mut expected = Vec::new();
        crate::export::copy(&tasks, &mut expected).unwrap();

        assert_eq!(output, expected);
        assert_eq!(
            app.notification.as_ref().map(Notification::message),
            Some("Copy sent to terminal")
        );

        for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
            press(
                &mut app,
                &fixture,
                KeyEvent {
                    kind,
                    ..KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)
                },
            );

            assert_eq!(
                app.notification.as_ref().map(Notification::message),
                Some("Copy sent to terminal")
            );
        }

        app.copy_markdown(&mut BrokenOutput);
        assert_eq!(
            app.notification.as_ref().map(Notification::message),
            Some("Copy failed: output unavailable")
        );
        assert_eq!(app.tasks, tasks);
        assert_eq!(app.list.selected(), selected);
        assert_eq!(app.scroll, 2);
        assert_eq!(app.history.clone(), undo);
        assert_eq!(app.redo_history.clone(), redo);
        assert_eq!(fixture.storage.load().unwrap(), saved);

        press(&mut app, &fixture, KeyCode::Down);
        assert!(app.notification.is_none());
    }

    #[test]
    fn copy_key_remains_text_or_is_ignored_outside_normal_mode() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(1);
        let tasks = app.tasks.clone();

        for mode in [
            Mode::Adding("".into(), AddPlacement::End),
            Mode::Editing {
                index: 0,
                input: "".into(),
            },
            Mode::Searching {
                input: "".into(),
                original: Some(0),
            },
            Mode::SearchResults {
                input: "task".into(),
                original: Some(0),
            },
            Mode::RenamingSection { input: "".into() },
            Mode::ConfirmClear,
        ] {
            app.mode = mode;
            press(&mut app, &fixture, KeyCode::Char('c'));

            match &app.mode {
                Mode::Adding(input, _)
                | Mode::Editing { input, .. }
                | Mode::Searching { input, .. }
                | Mode::RenamingSection { input }
                | Mode::FuzzySection { input, .. } => assert_eq!(input.text(), "c"),
                Mode::SearchResults { input, .. } => assert_eq!(input.text(), "task"),
                Mode::ConfirmClear => {}
                Mode::Normal => panic!("copy changed mode"),
            }

            assert!(app.notification.is_none());
            assert_eq!(app.tasks, tasks);
            assert!(!app.can_undo() && !app.can_redo());
        }

        assert!(fixture.storage.load().unwrap().is_empty());
    }

    #[test]
    fn task_register_copies_literal_text_and_pastes_unmarked_with_shared_placement() {
        let fixture = Fixture::new();
        let text = "Work: *日本語* [literal]";

        for (completed, carried) in [(false, false), (true, false), (false, true)] {
            for copy in [
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT),
                KeyEvent::new(KeyCode::Char('W'), KeyModifiers::ALT),
            ] {
                for (paste, placement) in [
                    (
                        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
                        AddPlacement::Below,
                    ),
                    (
                        KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT),
                        AddPlacement::Above,
                    ),
                    (
                        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
                        AddPlacement::Below,
                    ),
                ] {
                    let mut app = app_with_tasks(3);
                    app.tasks[2] = Task {
                        completed,
                        carried,
                        ..Task::new(text)
                    };
                    app.list.select(Some(2));
                    fixture.storage.save(&app.tasks).unwrap();
                    let before = app.tasks.clone();
                    press(&mut app, &fixture, copy);

                    assert_eq!(
                        app.notification.as_ref().map(Notification::message),
                        Some("Copied task")
                    );
                    assert_eq!(app.tasks, before);
                    assert_eq!(fixture.storage.load().unwrap(), before);
                    assert!(!app.can_undo() && !app.can_redo());

                    let index = app.insertion_index(placement);
                    press(&mut app, &fixture, paste);
                    assert!(app.notification.is_none());

                    // A paste creates a new task with its own identity.
                    let pasted = &app.tasks[index];
                    assert!(before.iter().all(|task| task.id != pasted.id));
                    assert_eq!(
                        *pasted,
                        Task {
                            id: pasted.id,
                            created_at: pasted.created_at,
                            ..Task::new(text)
                        }
                    );
                    assert_eq!(app.list.selected(), Some(index));
                    assert!(app.reveal_selection);
                    assert_eq!(app.history.len(), 1);

                    let pasted = app.tasks.clone();
                    app.undo(&fixture.storage).unwrap();
                    assert_eq!(app.tasks, before);

                    press(
                        &mut app,
                        &fixture,
                        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
                    );

                    assert_eq!(app.tasks, pasted);
                    assert_eq!(app.task_register.as_deref(), Some(text));

                    app.undo(&fixture.storage).unwrap();
                    press(&mut app, &fixture, paste);
                    assert!(!app.can_redo());
                }
            }
        }
    }

    #[test]
    fn register_survives_deletion_and_handles_no_selection_repeats_and_failed_saves() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(1);
        press(&mut app, &fixture, KeyCode::Char('p'));

        assert!(!app.can_undo());

        press(&mut app, &fixture, KeyCode::Char('y'));
        press(&mut app, &fixture, KeyCode::Char('d'));
        press(&mut app, &fixture, KeyCode::Char('y'));

        assert!(app.notification.is_none());
        assert_eq!(app.task_register.as_deref(), Some("task 0"));

        press(&mut app, &fixture, KeyCode::Char('P'));
        assert_eq!(app.list.selected(), Some(0));

        app.list.select(None);
        press(&mut app, &fixture, KeyCode::Char('p'));
        assert_eq!(app.list.selected(), Some(1));

        press(&mut app, &fixture, KeyCode::Char('p'));
        assert_eq!(app.list.selected(), Some(2));

        for (code, modifiers) in [
            (KeyCode::Char('p'), KeyModifiers::NONE),
            (KeyCode::Char('P'), KeyModifiers::SHIFT),
            (KeyCode::Char('y'), KeyModifiers::CONTROL),
            (KeyCode::Char('w'), KeyModifiers::ALT),
        ] {
            repeat(&mut app, &fixture, code, modifiers);
        }

        assert_eq!(app.tasks.len(), 3);

        app.undo(&fixture.storage).unwrap();
        let before = task_state(&app);
        block_saves(&fixture);

        assert!(
            app.key(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
                &fixture.storage
            )
            .is_err()
        );
        assert_eq!(task_state(&app), before);
        assert_eq!(app.task_register.as_deref(), Some("task 0"));
    }

    #[test]
    fn theme_keys_cycle_from_active_theme_without_mutating_tasks_or_history() {
        use crate::themes::BuiltinTheme;
        let fixture = Fixture::new();
        let mut app = app_with_tasks(2);
        press(&mut app, &fixture, KeyCode::Char(' '));
        press(&mut app, &fixture, KeyCode::Char('m'));
        press(&mut app, &fixture, KeyCode::Char('u'));
        let before = task_state(&app);
        let saved = std::fs::read(fixture.task_path()).unwrap();
        app.theme = BuiltinTheme::TokyoNightStorm;
        app.style_overrides = toml::from_str::<crate::config::Config>(
            "[styles.normal]\nmodifiers=['bold']\n[styles.accent]\nmodifiers=['italic']",
        )
        .unwrap()
        .styles;
        app.cli_accent = Some(ratatui::style::Color::Blue);

        for (key, expected) in [
            (
                KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
                BuiltinTheme::KanagawaWave,
            ),
            (
                KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT),
                BuiltinTheme::TokyoNightStorm,
            ),
            (
                KeyEvent::new(KeyCode::Char('t'), KeyModifiers::SHIFT),
                BuiltinTheme::CatppuccinMocha,
            ),
        ] {
            press(&mut app, &fixture, key);
            assert_eq!(app.theme, expected);
            assert_eq!(
                app.styles,
                crate::styles::Styles::resolve(expected, &app.style_overrides, app.cli_accent)
            );
            assert_eq!(app.styles.accent.fg, Some(ratatui::style::Color::Blue));
            assert!(
                app.styles
                    .accent
                    .add_modifier
                    .contains(ratatui::style::Modifier::ITALIC)
            );
            assert_eq!(
                app.notification.as_ref().unwrap().message(),
                format!("Theme: {} ({})", expected.display_name(), expected.id())
            );
        }

        assert_eq!(task_state(&app), before);
        assert_eq!(std::fs::read(fixture.task_path()).unwrap(), saved);

        for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
            press(
                &mut app,
                &fixture,
                KeyEvent {
                    kind,
                    ..KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE)
                },
            );

            assert_eq!(app.theme, BuiltinTheme::CatppuccinMocha);
        }

        app.tasks.clear();
        app.list.select(None);
        app.theme = BuiltinTheme::Default;
        press(&mut app, &fixture, KeyCode::Char('T'));

        assert_eq!(app.theme, BuiltinTheme::SolarizedDark);

        press(&mut app, &fixture, KeyCode::Char('t'));
        assert_eq!(app.theme, BuiltinTheme::Default);

        for &(theme, id, display) in BuiltinTheme::ALL {
            for forward in [false, true] {
                app.theme = theme.cycle(!forward);
                press(
                    &mut app,
                    &fixture,
                    KeyCode::Char(if forward { 't' } else { 'T' }),
                );

                assert_eq!(app.theme, theme);
                assert_eq!(
                    app.notification.as_ref().unwrap().message(),
                    format!("Theme: {display} ({id})")
                );
            }
        }
    }

    #[test]
    fn theme_keys_remain_text_or_are_ignored_in_other_modes() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(1);

        for mode in [
            Mode::Adding("".into(), AddPlacement::End),
            Mode::Editing {
                index: 0,
                input: "".into(),
            },
            Mode::Searching {
                input: "".into(),
                original: Some(0),
            },
            Mode::SearchResults {
                input: "task".into(),
                original: Some(0),
            },
            Mode::ConfirmClear,
        ] {
            app.mode = mode;
            press(&mut app, &fixture, KeyCode::Char('t'));
            press(&mut app, &fixture, KeyCode::Char('T'));

            assert_eq!(app.theme, crate::themes::BuiltinTheme::Default);
            assert!(app.notification.is_none());

            match &app.mode {
                Mode::Adding(input, _)
                | Mode::Editing { input, .. }
                | Mode::Searching { input, .. } => assert_eq!(input.text(), "tT"),
                Mode::SearchResults { input, .. } => assert_eq!(input.text(), "task"),
                Mode::ConfirmClear => {}
                _ => panic!("unexpected mode change"),
            }
        }
    }

    #[test]
    fn section_prefix_is_a_leading_word_followed_by_a_colon_and_space() {
        for (text, expected, body) in [
            ("Work: Fix bug", Some("Work"), "Fix bug"),
            ("Work:   spaced", Some("Work"), "spaced"),
            ("foo: bar: baz", Some("foo"), "bar: baz"),
            ("家👩‍💻: task", Some("家👩‍💻"), "task"),
            ("Work:Fix bug", None, "Work:Fix bug"),
            ("https://example.com", None, "https://example.com"),
            ("10:30 call", None, "10:30 call"),
            ("Fix the bug: urgent", None, "Fix the bug: urgent"),
            (":empty", None, ":empty"),
            ("no colon", None, "no colon"),
            ("one\u{2003}two: task", None, "one\u{2003}two: task"),
            ("e\u{301}:\u{301} task", None, "e\u{301}:\u{301} task"),
            ("foo:bar:baz", None, "foo:bar:baz"),
        ] {
            assert_eq!(task_prefix(text), expected, "{text}");
            assert_eq!(section_body(text), body, "{text}");
        }
    }

    #[test]
    fn section_tabs_list_all_then_unsectioned_then_sections_alphabetically() {
        let mut app = app_with_texts(&["work: a", "Home: b", "plain", "Work: c", "Errands: d"]);

        let labels: Vec<_> = app
            .section_tabs()
            .into_iter()
            .map(|(_, label)| label)
            .collect();

        // Sections match case-insensitively and take the first spelling in the list.
        assert_eq!(labels, ["[*]", "[-]", "Errands", "Home", "work"]);

        app.tasks.remove(2);
        let labels: Vec<_> = app
            .section_tabs()
            .into_iter()
            .map(|(_, label)| label)
            .collect();
        assert_eq!(labels, ["[*]", "Errands", "Home", "work"]);

        let plain = app_with_texts(&["plain", "https://example.com", "10:30 call"]);
        assert!(plain.section_tabs().is_empty());
    }

    #[test]
    fn tab_keys_cycle_section_views_and_keep_a_visible_selection() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b", "Work: c", "plain"]);
        app.list.select(Some(2));

        for (key, view, selected) in [
            (KeyCode::Tab, SectionView::Unsectioned, 3),
            (KeyCode::Tab, SectionView::Named("home".into()), 1),
            (KeyCode::Tab, SectionView::Named("work".into()), 0),
            (KeyCode::Tab, SectionView::All, 0),
            (KeyCode::BackTab, SectionView::Named("work".into()), 0),
            (KeyCode::BackTab, SectionView::Named("home".into()), 1),
        ] {
            press(&mut app, &fixture, key);

            assert_eq!(app.view, view, "{key:?}");
            assert_eq!(app.list.selected(), Some(selected), "{key:?}");
        }

        // Shift+Tab and held keys also cycle; releases do nothing.
        let shift_tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT);
        press(&mut app, &fixture, shift_tab);
        assert_eq!(app.view, SectionView::Unsectioned);

        let held = KeyEvent::new_with_kind(KeyCode::Tab, KeyModifiers::NONE, KeyEventKind::Repeat);
        press(&mut app, &fixture, held);
        assert_eq!(app.view, SectionView::Named("home".into()));

        let released =
            KeyEvent::new_with_kind(KeyCode::Tab, KeyModifiers::NONE, KeyEventKind::Release);
        press(&mut app, &fixture, released);
        assert_eq!(app.view, SectionView::Named("home".into()));

        assert!(!app.can_undo());
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn h_l_and_alt_b_f_switch_sections_and_wrap_like_tab() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b", "plain"]);
        let alt = |character| KeyEvent::new(KeyCode::Char(character), KeyModifiers::ALT);

        for (key, view) in [
            (
                KeyEvent::from(KeyCode::Char('h')),
                SectionView::Named("work".into()),
            ),
            (KeyCode::Char('l').into(), SectionView::All),
            (KeyCode::Char('l').into(), SectionView::Unsectioned),
            (alt('f'), SectionView::Named("home".into())),
            (alt('b'), SectionView::Unsectioned),
            (alt('b'), SectionView::All),
            (alt('b'), SectionView::Named("work".into())),
        ] {
            press(&mut app, &fixture, key);

            assert_eq!(app.view, view, "{key:?}");
        }

        assert!(!app.can_undo());
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn alt_b_and_f_still_move_by_word_while_adding() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b"]);

        type_keys(&mut app, &fixture, "aone two");
        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT),
        );

        let Mode::Adding(input, _) = &app.mode else {
            panic!("expected to keep adding");
        };

        assert_eq!(input.cursor(), "one ".len());
        assert_eq!(app.view, SectionView::All);
    }

    #[test]
    fn s_or_alt_s_and_a_digit_jump_to_tabs_in_display_order() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b", "plain"]);

        type_keys(&mut app, &fixture, "s4");
        assert_eq!(app.view, SectionView::Named("work".into()));

        type_keys(&mut app, &fixture, "s1");
        assert_eq!(app.view, SectionView::All);

        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT),
        );
        press(&mut app, &fixture, KeyCode::Char('3'));
        assert_eq!(app.view, SectionView::Named("home".into()));

        // Numbers past the last tab, including 0 for the tenth, do nothing.
        type_keys(&mut app, &fixture, "s5");
        assert_eq!(app.view, SectionView::Named("home".into()));

        type_keys(&mut app, &fixture, "s0");
        assert_eq!(app.view, SectionView::Named("home".into()));
    }

    #[test]
    fn s0_jumps_to_the_tenth_tab() {
        let fixture = Fixture::new();
        let texts: Vec<String> = (1..=9).map(|number| format!("S{number}: task")).collect();
        let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
        let mut app = app_with_texts(&texts);

        type_keys(&mut app, &fixture, "s0");

        assert_eq!(app.view, SectionView::Named("s9".into()));
    }

    #[test]
    fn a_key_other_than_a_digit_after_s_only_cancels_the_jump() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b", "Work: c"]);

        type_keys(&mut app, &fixture, "sd");
        assert_eq!(app.tasks.len(), 3);
        assert!(!app.can_undo());

        press(&mut app, &fixture, KeyCode::Char('s'));
        press(&mut app, &fixture, KeyCode::Esc);
        type_keys(&mut app, &fixture, "2");
        assert_eq!(app.view, SectionView::All);

        // Counts work as before once the jump is cancelled.
        type_keys(&mut app, &fixture, "j");
        assert_eq!(app.list.selected(), Some(2));
    }

    // Tabs: [*], [-], Home, Wishlist, Work.
    fn picker_app() -> App {
        app_with_texts(&["Work: a", "Home: b", "Wishlist: c", "plain"])
    }

    fn picker_matches(app: &App) -> Vec<SectionView> {
        let Mode::FuzzySection { matches, .. } = &app.mode else {
            panic!("expected the section picker");
        };

        matches.clone()
    }

    fn named(key: &str) -> SectionView {
        SectionView::Named(key.into())
    }

    #[test]
    fn backtick_opens_a_section_picker_listing_tabs_in_order_without_moving() {
        let fixture = Fixture::new();
        let mut app = picker_app();
        app.list.select(Some(2));

        press(&mut app, &fixture, KeyCode::Char('`'));

        assert_eq!(
            picker_matches(&app),
            [
                SectionView::Unsectioned,
                named("home"),
                named("wishlist"),
                named("work")
            ]
        );
        assert_eq!(app.view, SectionView::All);
        assert_eq!(app.list.selected(), Some(2));
    }

    #[test]
    fn backtick_without_sections_stays_in_normal_mode() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["plain"]);

        press(&mut app, &fixture, KeyCode::Char('`'));

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.view, SectionView::All);
    }

    #[test]
    fn picker_queries_match_fuzzily_and_preview_the_best_match_even_when_deleting() {
        let fixture = Fixture::new();
        let mut app = picker_app();
        press(&mut app, &fixture, KeyCode::Char('`'));

        // "wl" is not a substring of "Wishlist", but its letters appear in order.
        type_keys(&mut app, &fixture, "wl");
        assert_eq!(picker_matches(&app), [named("wishlist")]);
        assert_eq!(app.view, named("wishlist"));
        assert_eq!(app.list.selected(), Some(2));

        press(&mut app, &fixture, KeyCode::Backspace);
        press(&mut app, &fixture, KeyCode::Char('o'));
        assert_eq!(picker_matches(&app), [named("work")]);
        assert_eq!(app.view, named("work"));
        assert_eq!(app.list.selected(), Some(0));

        press(&mut app, &fixture, KeyCode::Backspace);
        assert_eq!(app.view, named("wishlist"));

        // An emptied query lists every tab again and previews the first.
        press(&mut app, &fixture, KeyCode::Backspace);
        assert_eq!(picker_matches(&app).len(), 4);
        assert_eq!(app.view, SectionView::Unsectioned);
        assert_eq!(app.list.selected(), Some(3));

        // [-] is found by name.
        type_keys(&mut app, &fixture, "uns");
        assert_eq!(picker_matches(&app), [SectionView::Unsectioned]);
    }

    #[test]
    fn equally_good_picker_matches_keep_the_tab_order() {
        let fixture = Fixture::new();
        let mut app = picker_app();
        press(&mut app, &fixture, KeyCode::Char('`'));

        press(&mut app, &fixture, KeyCode::Char('w'));

        assert_eq!(picker_matches(&app), [named("wishlist"), named("work")]);
    }

    #[test]
    fn cursor_moves_in_the_picker_keep_the_focused_match() {
        let fixture = Fixture::new();
        let mut app = picker_app();
        press(&mut app, &fixture, KeyCode::Char('`'));
        press(&mut app, &fixture, KeyCode::Char('w'));
        press(&mut app, &fixture, KeyCode::Tab);

        press(&mut app, &fixture, KeyCode::Left);
        press(&mut app, &fixture, KeyCode::Home);
        press(&mut app, &fixture, KeyCode::End);

        assert_eq!(app.view, named("work"));
    }

    #[test]
    fn tab_and_ctrl_arrows_step_through_picker_matches_and_wrap() {
        let fixture = Fixture::new();
        let ctrl = |code| KeyEvent::new(code, KeyModifiers::CONTROL);

        // From [*], nothing is focused yet: Tab starts at the first match, Shift+Tab at the
        // last.
        let mut app = picker_app();
        press(&mut app, &fixture, KeyCode::Char('`'));

        for (key, view) in [
            (KeyEvent::from(KeyCode::Tab), SectionView::Unsectioned),
            (KeyEvent::from(KeyCode::BackTab), named("work")),
            (KeyEvent::from(KeyCode::Tab), SectionView::Unsectioned),
            (ctrl(KeyCode::Right), named("home")),
            (ctrl(KeyCode::Left), SectionView::Unsectioned),
            (ctrl(KeyCode::Left), named("work")),
        ] {
            press(&mut app, &fixture, key);

            assert_eq!(app.view, view, "{key:?}");
        }

        let mut app = picker_app();
        press(&mut app, &fixture, KeyCode::Char('`'));
        press(&mut app, &fixture, KeyCode::BackTab);
        assert_eq!(app.view, named("work"));

        // From a section tab, stepping starts at that tab, and a held Tab keeps stepping.
        let mut app = picker_app();
        app.show_section(named("home"));
        press(&mut app, &fixture, KeyCode::Char('`'));

        press(&mut app, &fixture, KeyCode::Tab);
        assert_eq!(app.view, named("wishlist"));

        repeat(&mut app, &fixture, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(app.view, named("work"));

        assert!(matches!(app.mode, Mode::FuzzySection { .. }));
        assert!(!app.can_undo());
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn enter_keeps_the_previewed_section_without_history_or_saving() {
        let fixture = Fixture::new();
        let mut app = picker_app();
        press(&mut app, &fixture, KeyCode::Char('`'));
        type_keys(&mut app, &fixture, "wl");

        // Holding Enter does nothing; only a press confirms.
        repeat(&mut app, &fixture, KeyCode::Enter, KeyModifiers::NONE);
        assert!(matches!(app.mode, Mode::FuzzySection { .. }));

        press(&mut app, &fixture, KeyCode::Enter);

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.view, named("wishlist"));
        assert_eq!(app.list.selected(), Some(2));
        assert!(!app.can_undo() && !app.can_redo());
        assert!(!fixture.task_path().exists());

        // Like any navigation, the chosen tab is saved on a clean exit.
        app.save_selection(&fixture.storage).unwrap();
        assert_eq!(
            App::from_document(fixture.storage.load_document().unwrap()).view,
            named("wishlist")
        );
    }

    #[test]
    fn esc_and_ctrl_g_restore_the_view_selection_and_scrolling() {
        let fixture = Fixture::new();

        for cancel in [
            KeyEvent::from(KeyCode::Esc),
            KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL),
        ] {
            let mut app = picker_app();
            app.list.select(Some(3));
            app.scroll = 2;
            app.tab_scroll = 1;
            app.reveal_selection = false;

            press(&mut app, &fixture, KeyCode::Char('`'));
            type_keys(&mut app, &fixture, "wo");
            press(&mut app, &fixture, KeyCode::Tab);
            app.scroll = 0;
            app.tab_scroll = 0;

            press(&mut app, &fixture, cancel);

            assert_eq!(app.mode, Mode::Normal);
            assert_eq!(app.view, SectionView::All);
            assert_eq!(app.list.selected(), Some(3));
            assert_eq!((app.scroll, app.tab_scroll), (2, 1));
            assert!(!app.reveal_selection);
            assert!(!app.can_undo() && !app.can_redo());
        }

        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn saving_while_picking_keeps_the_view_from_before_the_picker() {
        let fixture = Fixture::new();
        let mut app = picker_app();
        press(&mut app, &fixture, KeyCode::Char('`'));
        type_keys(&mut app, &fixture, "wl");

        app.save_selection(&fixture.storage).unwrap();

        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn without_picker_matches_tab_and_enter_wait_until_the_query_matches_again() {
        let fixture = Fixture::new();
        let mut app = picker_app();
        press(&mut app, &fixture, KeyCode::Char('`'));
        type_keys(&mut app, &fixture, "wlq");

        // The last preview stays on screen while nothing matches.
        assert!(picker_matches(&app).is_empty());
        assert_eq!(app.view, named("wishlist"));

        for key in [
            KeyEvent::from(KeyCode::Tab),
            KeyEvent::from(KeyCode::BackTab),
            KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL),
            KeyEvent::from(KeyCode::Enter),
        ] {
            press(&mut app, &fixture, key);

            assert!(matches!(app.mode, Mode::FuzzySection { .. }), "{key:?}");
            assert_eq!(app.view, named("wishlist"), "{key:?}");
        }

        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        type_keys(&mut app, &fixture, "hm");
        assert_eq!(app.view, named("home"));

        press(&mut app, &fixture, KeyCode::Esc);
        assert_eq!(app.view, SectionView::All);
    }

    #[test]
    fn task_shortcuts_and_clicks_only_type_into_the_section_picker() {
        let fixture = Fixture::new();
        let mut app = picker_app();
        let tasks = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char('`'));

        type_keys(&mut app, &fixture, "dx Jq");
        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
        );
        app.row_tasks = vec![Some(0), Some(1)];
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 1,
                row: 1,
                modifiers: KeyModifiers::NONE,
            },
            Rect::new(1, 1, 20, 2),
            &fixture.storage,
        )
        .unwrap();

        let Mode::FuzzySection { input, .. } = &app.mode else {
            panic!("expected the section picker");
        };

        // Typing previews matching tabs, so only the selection may change.
        assert_eq!(input.text(), "dx Jq");
        assert_eq!(app.tasks, tasks);
        assert!(!app.can_undo() && !app.can_redo());
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn the_picker_matches_unicode_names_and_pasted_text() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Café: latte", "日本語: 本", "Work: a"]);
        press(&mut app, &fixture, KeyCode::Char('`'));

        app.paste("cafe");
        assert_eq!(picker_matches(&app), [named("café")]);
        assert_eq!(app.view, named("café"));

        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        app.paste("日語");
        assert_eq!(picker_matches(&app), [named("日本語")]);
        assert_eq!(app.list.selected(), Some(1));
    }

    #[test]
    fn moving_a_section_swaps_named_tabs_as_one_change_and_keeps_tasks_and_view() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b", "Errands: c", "plain"]);
        let tasks = app.tasks.clone();
        app.show_section(SectionView::Named("errands".into()));

        type_keys(&mut app, &fixture, "L");
        assert_eq!(app.section_order, ["home", "errands", "work"]);

        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT),
        );
        assert_eq!(app.section_order, ["home", "work", "errands"]);

        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT),
        );
        type_keys(&mut app, &fixture, "H");
        assert_eq!(app.section_order, ["errands", "home", "work"]);

        type_keys(&mut app, &fixture, "LL");

        let labels: Vec<_> = app
            .section_tabs()
            .into_iter()
            .map(|(_, label)| label)
            .collect();
        assert_eq!(labels, ["[*]", "[-]", "Home", "Work", "Errands"]);
        assert_eq!(app.tasks, tasks);
        assert_eq!(app.view, SectionView::Named("errands".into()));

        let saved = fixture.storage.load_document().unwrap();
        assert_eq!(saved.section_order, ["home", "work", "errands"]);
        assert_eq!(saved.tasks, tasks);

        type_keys(&mut app, &fixture, "u");
        assert_eq!(app.section_order, ["home", "errands", "work"]);
        assert_eq!(notice(&app), Some("Undid: move section"));

        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.section_order, ["home", "work", "errands"]);
        assert_eq!(
            fixture.storage.load_document().unwrap().section_order,
            app.section_order
        );
    }

    #[test]
    fn sections_do_not_move_past_the_ends_or_from_the_all_and_unsectioned_tabs() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b", "plain"]);

        for view in [SectionView::All, SectionView::Unsectioned] {
            app.show_section(view);
            type_keys(&mut app, &fixture, "HL");
        }

        app.show_section(SectionView::Named("home".into()));
        type_keys(&mut app, &fixture, "H");

        app.show_section(SectionView::Named("work".into()));
        type_keys(&mut app, &fixture, "L");

        assert_eq!(app.section_order, ["home", "work"]);
        assert!(!app.can_undo());
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn new_sections_join_at_the_end_and_leave_with_their_last_task() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b"]);
        app.section_order.reverse();

        type_keys(&mut app, &fixture, "aAlpha: c");
        press(&mut app, &fixture, KeyCode::Enter);
        assert_eq!(app.section_order, ["work", "home", "alpha"]);

        // Deleting Work's only task drops it; adding it again puts it last.
        app.list.select(Some(0));
        type_keys(&mut app, &fixture, "d");
        assert_eq!(app.section_order, ["home", "alpha"]);

        type_keys(&mut app, &fixture, "aWork: again");
        press(&mut app, &fixture, KeyCode::Enter);
        assert_eq!(app.section_order, ["home", "alpha", "work"]);
        assert_eq!(
            fixture.storage.load_document().unwrap().section_order,
            app.section_order
        );
    }

    #[test]
    fn renaming_a_section_keeps_its_place_in_the_tab_order() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Alpha: a", "Beta: b", "Gamma: c"]);
        app.show_section(SectionView::Named("alpha".into()));

        press(&mut app, &fixture, KeyCode::Char('E'));
        type_rename(&mut app, &fixture, "Zulu");
        press(&mut app, &fixture, KeyCode::Enter);

        assert_eq!(app.section_order, ["zulu", "beta", "gamma"]);
        assert_eq!(app.view, SectionView::Named("zulu".into()));
    }

    #[test]
    fn reordering_tasks_never_changes_the_section_order() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b"]);
        app.list.select(Some(1));

        type_keys(&mut app, &fixture, "K");

        assert_eq!(texts(&app), ["Home: b", "Work: a"]);
        assert_eq!(app.section_order, ["home", "work"]);
    }

    #[test]
    fn navigation_and_jumps_stay_within_the_current_section() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: x", "Work: b", "plain", "Work: c"]);
        app.show_section(SectionView::Named("work".into()));

        for (keys, expected) in [
            ("j", 2),
            ("j", 4),
            ("j", 0),
            ("k", 4),
            ("5k", 0),
            ("9j", 4),
            ("g", 0),
            ("G", 4),
        ] {
            type_keys(&mut app, &fixture, keys);
            assert_eq!(app.list.selected(), Some(expected), "{keys}");
        }
    }

    #[test]
    fn reordering_in_a_section_moves_past_its_own_tasks_as_one_change() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: x", "Work: b", "Home: y", "Work: c"]);
        app.show_section(SectionView::Named("work".into()));

        press(&mut app, &fixture, KeyCode::Char('J'));

        assert_eq!(
            texts(&app),
            ["Home: x", "Work: b", "Work: a", "Home: y", "Work: c"]
        );
        assert_eq!(app.list.selected(), Some(2));

        type_keys(&mut app, &fixture, "2J");

        assert_eq!(
            texts(&app),
            ["Home: x", "Work: b", "Home: y", "Work: c", "Work: a"]
        );
        assert_eq!(app.list.selected(), Some(4));
        assert_eq!(app.history.len(), 2);

        type_keys(&mut app, &fixture, "9J");
        assert_eq!(app.history.len(), 2);

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(
            texts(&app),
            ["Home: x", "Work: b", "Work: a", "Home: y", "Work: c"]
        );
    }

    #[test]
    fn deleting_in_a_section_selects_a_neighbor_there_and_an_emptied_tab_shows_all() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: x", "Work: b"]);
        app.show_section(SectionView::Named("work".into()));

        press(&mut app, &fixture, KeyCode::Char('d'));

        assert_eq!(texts(&app), ["Home: x", "Work: b"]);
        assert_eq!(app.list.selected(), Some(1));
        assert_eq!(app.view, SectionView::Named("work".into()));

        press(&mut app, &fixture, KeyCode::Char('d'));

        assert_eq!(texts(&app), ["Home: x"]);
        assert_eq!(app.list.selected(), Some(0));
        assert_eq!(app.view, SectionView::All);
    }

    #[test]
    fn changes_that_move_the_selection_out_of_a_tab_follow_it_to_its_section() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: x", "Work: b"]);
        app.show_section(SectionView::Named("work".into()));

        press(&mut app, &fixture, KeyCode::Char('e'));
        press(&mut app, &fixture, KeyCode::Home);
        for _ in 0.."Work".len() {
            press(&mut app, &fixture, KeyCode::Delete);
        }
        type_keys(&mut app, &fixture, "Home");
        press(&mut app, &fixture, KeyCode::Enter);

        assert_eq!(texts(&app), ["Home: a", "Home: x", "Work: b"]);
        assert_eq!(app.view, SectionView::Named("home".into()));
        assert_eq!(app.list.selected(), Some(0));

        press(&mut app, &fixture, KeyCode::Char('u'));

        assert_eq!(texts(&app), ["Work: a", "Home: x", "Work: b"]);
        assert_eq!(app.view, SectionView::Named("work".into()));
        assert_eq!(app.list.selected(), Some(0));
    }

    #[test]
    fn search_in_a_section_matches_only_its_tasks_and_ignores_the_prefix() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: fix", "Home: fix", "Work: other"]);
        app.show_section(SectionView::Named("work".into()));
        app.list.select(Some(2));

        press(&mut app, &fixture, KeyCode::Char('/'));
        app.paste("fix");
        assert_eq!(app.list.selected(), Some(0));

        press(&mut app, &fixture, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.list.selected(), Some(0));

        press(&mut app, &fixture, KeyCode::Char('/'));
        app.paste("work");
        assert_eq!(app.list.selected(), None);

        press(&mut app, &fixture, KeyCode::Esc);
        app.show_section(SectionView::All);
        press(&mut app, &fixture, KeyCode::Char('/'));
        app.paste("work");
        assert_eq!(app.list.selected(), Some(0));
    }

    #[test]
    fn clearing_completed_in_a_section_affects_only_that_section() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b", "Work: c", "Work: d"]);
        for index in [0, 1, 3] {
            app.tasks[index].completed = true;
        }
        app.show_section(SectionView::Named("work".into()));

        press(&mut app, &fixture, KeyCode::Char('x'));

        assert_eq!(texts(&app), ["Home: b", "Work: c"]);
        assert_eq!(app.list.selected(), Some(1));
        assert_eq!(app.view, SectionView::Named("work".into()));
        assert_eq!(notice(&app), Some("Cleared 2 completed Work tasks"));

        press(&mut app, &fixture, KeyCode::Char('x'));
        assert_eq!(notice(&app), Some("No completed Work tasks"));

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(notice(&app), Some("Undid: clear 2 completed Work tasks"));
        assert_eq!(app.tasks.len(), 4);

        app.show_section(SectionView::All);
        press(&mut app, &fixture, KeyCode::Char('x'));
        assert_eq!(texts(&app), ["Work: c"]);
        assert_eq!(notice(&app), Some("Cleared 3 completed tasks"));
    }

    #[test]
    fn clearing_unmarked_in_a_section_keeps_other_sections_and_carried_tasks() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b", "Work: c", "Work: d"]);
        app.tasks[2].carried = true;
        app.show_section(SectionView::Named("work".into()));
        app.list.select(Some(3));

        press(&mut app, &fixture, KeyCode::Char('D'));
        press(&mut app, &fixture, KeyCode::Enter);

        assert_eq!(texts(&app), ["Home: b", "Work: c"]);
        assert_eq!(app.list.selected(), Some(1));
        assert_eq!(app.view, SectionView::Named("work".into()));
        assert_eq!(notice(&app), Some("Cleared 2 Work tasks, kept 1 carried"));

        // Nothing unmarked is left in this tab, so D does not ask.
        press(&mut app, &fixture, KeyCode::Char('D'));
        assert_eq!(app.mode, Mode::Normal);

        let mut app = app_with_texts(&["Work: a", "Home: b"]);
        app.show_section(SectionView::Named("work".into()));

        press(&mut app, &fixture, KeyCode::Char('D'));
        press(&mut app, &fixture, KeyCode::Enter);

        assert_eq!(texts(&app), ["Home: b"]);
        assert_eq!(app.list.selected(), Some(0));
        assert_eq!(app.view, SectionView::All);
        assert_eq!(notice(&app), Some("Cleared 1 Work task"));
    }

    #[test]
    fn copying_in_a_tab_copies_only_its_tasks_with_full_text() {
        let mut app = app_with_texts(&["Work: a", "plain", "Work: b"]);

        for (view, shown) in [
            (SectionView::Named("work".into()), vec![0, 2]),
            (SectionView::Unsectioned, vec![1]),
            (SectionView::All, vec![0, 1, 2]),
        ] {
            app.show_section(view);
            let mut output = Vec::new();
            app.copy_markdown(&mut output);

            let tasks: Vec<_> = shown
                .iter()
                .map(|&index| app.tasks[index].clone())
                .collect();
            let mut expected = Vec::new();
            crate::export::copy(&tasks, &mut expected).unwrap();

            assert_eq!(output, expected);
        }
    }

    fn input_text(app: &App) -> &str {
        match &app.mode {
            Mode::Adding(input, _)
            | Mode::Editing { input, .. }
            | Mode::RenamingSection { input } => input.text(),
            mode => panic!("expected an input mode, got {mode:?}"),
        }
    }

    #[test]
    fn adding_on_a_section_tab_suggests_that_section_and_typing_accepts_it() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["work: a", "Home: b"]);
        app.show_section(SectionView::Named("work".into()));

        press(&mut app, &fixture, KeyCode::Char('a'));
        assert_eq!(input_text(&app), "");
        assert_eq!(app.completion_ghost().as_deref(), Some("work: "));

        type_keys(&mut app, &fixture, "x");
        assert_eq!(input_text(&app), "work: x");
        assert_eq!(app.completion_ghost(), None);

        press(&mut app, &fixture, KeyCode::Enter);

        assert_eq!(texts(&app), ["work: a", "Home: b", "work: x"]);
        assert_eq!(app.view, SectionView::Named("work".into()));
        assert_eq!(app.list.selected(), Some(2));

        app.show_section(SectionView::All);
        press(&mut app, &fixture, KeyCode::Char('a'));
        assert_eq!(app.completion_ghost(), None);
    }

    #[test]
    fn space_or_enter_accept_a_suggestion_and_backspace_or_esc_dismiss_it() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b"]);
        app.show_section(SectionView::Named("work".into()));

        for accept in [KeyCode::Char(' '), KeyCode::Enter] {
            press(&mut app, &fixture, KeyCode::Char('o'));
            press(&mut app, &fixture, accept);

            assert_eq!(input_text(&app), "Work: ");
            assert_eq!(app.completion_ghost(), None);

            // A section with no task text adds nothing.
            press(&mut app, &fixture, KeyCode::Enter);
            assert_eq!(app.mode, Mode::Normal);
            assert_eq!(app.tasks.len(), 2);
        }

        for dismiss in [KeyCode::Backspace, KeyCode::Esc] {
            press(&mut app, &fixture, KeyCode::Char('a'));
            press(&mut app, &fixture, dismiss);

            assert_eq!(input_text(&app), "");
            assert_eq!(app.completion_ghost(), None);

            press(&mut app, &fixture, KeyCode::Esc);
            assert_eq!(app.mode, Mode::Normal);
        }

        press(&mut app, &fixture, KeyCode::Char('a'));
        press(&mut app, &fixture, KeyCode::Backspace);
        type_keys(&mut app, &fixture, "plain");
        press(&mut app, &fixture, KeyCode::Enter);

        assert_eq!(texts(&app), ["Work: a", "Home: b", "plain"]);
        assert_eq!(app.view, SectionView::Unsectioned);
        assert!(!app.can_redo());
        assert_eq!(app.history.len(), 1);
    }

    #[test]
    fn tab_cycles_matching_sections_on_empty_or_partial_input() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b", "Errands: c", "Workout: d"]);
        press(&mut app, &fixture, KeyCode::Char('a'));

        for (key, ghost) in [
            (KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), "Errands: "),
            (KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), "Home: "),
            (
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
                "Errands: ",
            ),
            (
                KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT),
                "Workout: ",
            ),
        ] {
            press(&mut app, &fixture, key);
            assert_eq!(app.completion_ghost().as_deref(), Some(ghost), "{key:?}");
        }

        press(&mut app, &fixture, KeyCode::Esc);
        type_keys(&mut app, &fixture, "wO");
        press(&mut app, &fixture, KeyCode::Tab);
        assert_eq!(app.completion_ghost().as_deref(), Some("rk: "));

        press(&mut app, &fixture, KeyCode::Tab);
        assert_eq!(app.completion_ghost().as_deref(), Some("rkout: "));

        press(&mut app, &fixture, KeyCode::Enter);
        assert_eq!(input_text(&app), "Workout: ");

        press(&mut app, &fixture, KeyCode::Esc);
        press(&mut app, &fixture, KeyCode::Char('a'));
        type_keys(&mut app, &fixture, "zz");
        press(&mut app, &fixture, KeyCode::Tab);

        assert_eq!(app.completion_ghost(), None);
        assert_eq!(notice(&app), Some("No matching sections"));

        // Text with a space can no longer become a section.
        type_keys(&mut app, &fixture, " x");
        press(&mut app, &fixture, KeyCode::Tab);
        assert_eq!(app.completion_ghost(), None);
        assert_eq!(input_text(&app), "zz x");
    }

    #[test]
    fn editing_an_erased_task_can_move_it_to_another_section() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b"]);
        app.show_section(SectionView::Named("work".into()));

        press(&mut app, &fixture, KeyCode::Char('e'));
        assert_eq!(input_text(&app), "Work: a");
        assert_eq!(app.completion_ghost(), None);

        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        press(&mut app, &fixture, KeyCode::Tab);
        assert_eq!(app.completion_ghost().as_deref(), Some("Home: "));

        // Modifier shortcuts edit the input directly and drop the suggestion.
        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.completion_ghost(), None);

        press(&mut app, &fixture, KeyCode::Tab);
        app.paste("moved task");
        assert_eq!(input_text(&app), "Home: moved task");

        press(&mut app, &fixture, KeyCode::Enter);

        assert_eq!(texts(&app), ["Home: moved task", "Home: b"]);
        assert_eq!(app.view, SectionView::Named("home".into()));
        assert_eq!(app.list.selected(), Some(0));
    }

    // Replace the whole rename input with `name`.
    fn type_rename(app: &mut App, fixture: &Fixture, name: &str) {
        press(
            app,
            fixture,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        app.paste(name);
    }

    #[test]
    fn renaming_a_section_updates_every_task_in_it_and_follows_the_new_tab() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b", "work: c"]);
        app.show_section(SectionView::Named("work".into()));

        press(&mut app, &fixture, KeyCode::Char('E'));
        assert_eq!(input_text(&app), "Work");

        type_rename(&mut app, &fixture, "Job");
        press(&mut app, &fixture, KeyCode::Enter);

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(texts(&app), ["Job: a", "Home: b", "Job: c"]);
        assert_eq!(app.view, SectionView::Named("job".into()));
        assert_eq!(notice(&app), Some("Renamed Work to Job (2 tasks)"));

        let saved = fixture.storage.load_document().unwrap();
        assert_eq!(saved.tasks, app.tasks);
        assert_eq!(saved.section.as_deref(), Some("job"));
    }

    #[test]
    fn renaming_to_a_taken_or_invalid_name_is_blocked_and_explained() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b"]);
        app.show_section(SectionView::Named("work".into()));
        let original = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char('E'));

        for (name, problem) in [
            ("home", "Section name already taken"),
            ("two words", "Section names are one word without colons"),
            ("a:b", "Section names are one word without colons"),
            ("", "Type a section name"),
        ] {
            type_rename(&mut app, &fixture, name);
            assert_eq!(app.rename_problem(), Some(problem));
            assert_eq!(notice(&app), Some(problem));

            press(&mut app, &fixture, KeyCode::Enter);

            assert_eq!(input_text(&app), name);
            assert_eq!(notice(&app), Some(problem));
            assert_eq!(app.tasks, original);
            assert!(!app.can_undo());
        }
    }

    #[test]
    fn the_rename_problem_is_shown_even_with_notifications_off() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b"]);
        app.notifications = false;
        app.show_section(SectionView::Named("work".into()));
        press(&mut app, &fixture, KeyCode::Char('E'));

        type_rename(&mut app, &fixture, "Home");

        assert_eq!(notice(&app), Some("Section name already taken"));
    }

    #[test]
    fn renaming_only_the_case_is_allowed_and_unifies_spellings() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["work: a", "Work: b"]);
        app.show_section(SectionView::Named("work".into()));
        press(&mut app, &fixture, KeyCode::Char('E'));

        type_rename(&mut app, &fixture, "WORK");
        assert_eq!(app.rename_problem(), None);
        press(&mut app, &fixture, KeyCode::Enter);

        assert_eq!(texts(&app), ["WORK: a", "WORK: b"]);
        assert_eq!(app.view, SectionView::Named("work".into()));
    }

    #[test]
    fn confirming_an_unchanged_name_closes_the_input_without_a_change() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a"]);
        app.show_section(SectionView::Named("work".into()));

        press(&mut app, &fixture, KeyCode::Char('E'));
        press(&mut app, &fixture, KeyCode::Enter);

        assert_eq!(app.mode, Mode::Normal);
        assert!(!app.can_undo());
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn escape_cancels_a_rename_without_changes() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a"]);
        app.show_section(SectionView::Named("work".into()));
        press(&mut app, &fixture, KeyCode::Char('E'));
        type_rename(&mut app, &fixture, "Job");

        press(&mut app, &fixture, KeyCode::Esc);

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(texts(&app), ["Work: a"]);
        assert!(!app.can_undo());
    }

    #[test]
    fn a_rename_is_one_undo_step_and_undo_returns_to_the_old_tab() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b", "Work: c"]);
        app.show_section(SectionView::Named("work".into()));
        press(&mut app, &fixture, KeyCode::Char('E'));
        type_rename(&mut app, &fixture, "Job");
        press(&mut app, &fixture, KeyCode::Enter);

        press(&mut app, &fixture, KeyCode::Char('u'));

        assert_eq!(texts(&app), ["Work: a", "Home: b", "Work: c"]);
        assert_eq!(app.view, SectionView::Named("work".into()));
        assert_eq!(notice(&app), Some("Undid: rename section"));
    }

    #[test]
    fn the_rename_key_outside_a_named_tab_only_explains() {
        let fixture = Fixture::new();

        for (texts, view, message) in [
            (
                vec!["a"],
                SectionView::All,
                "No sections yet (start a task with \"Name: \")",
            ),
            (
                vec!["Work: a", "b"],
                SectionView::All,
                "Switch to a section tab to rename it",
            ),
            (
                vec!["Work: a", "b"],
                SectionView::Unsectioned,
                "Switch to a section tab to rename it",
            ),
        ] {
            let mut app = app_with_texts(&texts);
            app.show_section(view);

            press(&mut app, &fixture, KeyCode::Char('E'));

            assert_eq!(app.mode, Mode::Normal);
            assert_eq!(notice(&app), Some(message));
        }
    }

    #[test]
    fn a_failed_rename_save_keeps_the_tasks_tab_history_and_input() {
        let fixture = Fixture::new();
        std::fs::write(fixture.directory.join("data"), "blocking file").unwrap();
        let mut app = app_with_texts(&["Work: a"]);
        app.show_section(SectionView::Named("work".into()));
        let original = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char('E'));
        type_rename(&mut app, &fixture, "Job");

        let result = app.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &fixture.storage,
        );

        assert!(result.is_err());
        assert_eq!(app.tasks, original);
        assert_eq!(app.view, SectionView::Named("work".into()));
        assert!(!app.can_undo());
        assert_eq!(input_text(&app), "Job");
    }

    #[test]
    fn the_section_tab_is_saved_and_restored_and_unknown_sections_fall_back_to_all() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "plain"]);
        fixture.storage.save(&app.tasks).unwrap();
        // In [*] the document has no active-tab `section` key (tasks still store theirs).
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture.task_path()).unwrap()).unwrap();
        assert!(saved.get("section").is_none());
        assert_eq!(saved["tasks"][0]["section"], "Work");

        for (view, saved) in [
            (SectionView::Named("work".into()), "\"section\": \"work\""),
            (SectionView::Unsectioned, "\"section\": \"\""),
        ] {
            app.show_section(view.clone());
            app.save_selection(&fixture.storage).unwrap();

            let file = std::fs::read_to_string(fixture.task_path()).unwrap();
            assert!(file.contains(saved), "{file}");

            let restored = App::from_document(fixture.storage.load_document().unwrap());
            assert_eq!(restored.view, view);
        }

        let file = std::fs::read_to_string(fixture.task_path()).unwrap();
        std::fs::write(fixture.task_path(), file.replace("\"\"", "\"missing\"")).unwrap();
        let restored = App::from_document(fixture.storage.load_document().unwrap());

        assert_eq!(restored.view, SectionView::All);
    }

    #[test]
    fn control_p_n_match_arrow_navigation_and_preserve_task_state() {
        let fixture = Fixture::new();

        for results in [false, true] {
            for selected in [None, Some(0), Some(2), Some(4)] {
                for (alias, arrow) in [
                    ('p', KeyCode::Up),
                    ('P', KeyCode::Up),
                    ('n', KeyCode::Down),
                    ('N', KeyCode::Down),
                ] {
                    for kind in [
                        KeyEventKind::Press,
                        KeyEventKind::Repeat,
                        KeyEventKind::Release,
                    ] {
                        let setup = || {
                            let mut app = app_with_tasks(5);

                            for (i, task) in app.tasks.iter_mut().enumerate() {
                                task.text = if i == 2 {
                                    "unrelated".into()
                                } else {
                                    "hit words ".repeat(8).trim().into()
                                };
                                task.completed = i == 1 || i == 3;
                                task.carried = i == 2;
                            }

                            app.list.select(selected);
                            app.keybinds = false;

                            if results {
                                app.mode = Mode::SearchResults {
                                    input: "hit".into(),
                                    original: selected,
                                };
                            }

                            app
                        };

                        let mut actual = setup();
                        let mut expected = setup();
                        let original = actual.tasks.clone();
                        press(
                            &mut actual,
                            &fixture,
                            KeyEvent::new_with_kind(
                                KeyCode::Char(alias),
                                KeyModifiers::CONTROL,
                                kind,
                            ),
                        );
                        press(
                            &mut expected,
                            &fixture,
                            KeyEvent::new_with_kind(arrow, KeyModifiers::NONE, kind),
                        );

                        assert_eq!(actual.list.selected(), expected.list.selected());
                        assert_eq!(actual.mode, expected.mode);

                        let mut terminal =
                            ratatui::Terminal::new(ratatui::backend::TestBackend::new(24, 6))
                                .unwrap();
                        let mut rows = Rect::default();
                        terminal
                            .draw(|frame| rows = crate::ui::draw(frame, &mut actual))
                            .unwrap();

                        if let Some(index) = actual.list.selected() {
                            assert!(
                                actual.row_tasks[actual.scroll
                                    ..(actual.scroll + usize::from(rows.height))
                                        .min(actual.row_tasks.len())]
                                    .contains(&Some(index))
                            );
                        }

                        assert_eq!(actual.tasks, original);
                        assert!(!actual.can_undo() && !actual.can_redo());
                    }
                }
            }
        }

        for mode in [
            Mode::Adding("text".into(), AddPlacement::End),
            Mode::Editing {
                index: 0,
                input: "text".into(),
            },
            Mode::Searching {
                input: "text".into(),
                original: Some(0),
            },
            Mode::ConfirmClear,
        ] {
            let mut app = app_with_tasks(1);
            app.mode = mode;
            let before = format!("{:?}", app.mode);

            for code in ['p', 'n'] {
                press(
                    &mut app,
                    &fixture,
                    KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL),
                );

                assert_eq!(format!("{:?}", app.mode), before);
            }
        }

        let mut empty = App::new(Vec::new());

        for code in ['p', 'n'] {
            press(
                &mut empty,
                &fixture,
                KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL),
            );

            assert_eq!(empty.list.selected(), None);
        }

        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn readline_shortcuts_edit_only_the_active_input() {
        let fixture = Fixture::new();

        for text in [
            "",
            "abc",
            "e\u{301}界 👩‍💻 👍🏽 last",
            "long text ".repeat(30).as_str(),
        ] {
            for cursor in [0, text.len() / 2, text.len()] {
                for (command, modifiers) in [
                    ('k', KeyModifiers::CONTROL),
                    ('u', KeyModifiers::CONTROL),
                    ('w', KeyModifiers::CONTROL),
                    ('b', KeyModifiers::CONTROL),
                    ('f', KeyModifiers::CONTROL),
                    ('d', KeyModifiers::CONTROL),
                    ('b', KeyModifiers::ALT),
                    ('f', KeyModifiers::ALT),
                    ('d', KeyModifiers::ALT),
                ] {
                    for mode in 0..3 {
                        for kind in [
                            KeyEventKind::Press,
                            KeyEventKind::Repeat,
                            KeyEventKind::Release,
                        ] {
                            let mut app = app_with_tasks(1);
                            app.keybinds = false;
                            let mut input = TextInput::from(text);
                            input.set_cursor(cursor);
                            let mut expected = input.clone();

                            if kind != KeyEventKind::Release {
                                match command {
                                    'k' => expected.delete_to_end(),
                                    'u' => expected.delete_to_start(),
                                    'w' => expected.delete_word_before(),
                                    'b' if modifiers == KeyModifiers::ALT => {
                                        expected.move_word_left()
                                    }
                                    'b' => expected.move_left(),
                                    'f' if modifiers == KeyModifiers::ALT => {
                                        expected.move_word_right()
                                    }
                                    'f' => expected.move_right(),
                                    'd' if modifiers == KeyModifiers::ALT => {
                                        expected.delete_word_after()
                                    }
                                    'd' => expected.delete_after(),
                                    _ => unreachable!(),
                                }
                            }

                            app.mode = match mode {
                                0 => Mode::Adding(input, AddPlacement::End),
                                1 => Mode::Editing { index: 0, input },
                                _ => Mode::Searching {
                                    input,
                                    original: Some(0),
                                },
                            };
                            press(
                                &mut app,
                                &fixture,
                                KeyEvent::new_with_kind(KeyCode::Char(command), modifiers, kind),
                            );

                            let actual = match &app.mode {
                                Mode::Adding(input, _)
                                | Mode::Editing { input, .. }
                                | Mode::Searching { input, .. } => input,
                                _ => panic!("input shortcut changed mode"),
                            };

                            assert_eq!(actual, &expected);

                            let mut terminal =
                                ratatui::Terminal::new(ratatui::backend::TestBackend::new(18, 6))
                                    .unwrap();
                            let mut rows = Rect::default();
                            terminal
                                .draw(|frame| rows = crate::ui::draw(frame, &mut app))
                                .unwrap();
                            let cursor = terminal.backend().cursor_position();

                            assert!(terminal.backend().cursor_visible());
                            assert!(if mode == 2 {
                                cursor.y == 5 && cursor.x < 18
                            } else {
                                rows.contains(cursor)
                            });
                            assert_eq!(app.tasks[0].text, "task 0");
                            assert!(!app.can_undo() && !app.can_redo());
                        }
                    }
                }
            }
        }

        let mut app = app_with_tasks(1);
        press(&mut app, &fixture, KeyCode::Char('d'));
        let tasks = app.tasks.clone();
        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        );

        assert_eq!(app.tasks, tasks);

        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        );

        assert!(matches!(app.mode, Mode::Searching { .. }));
    }

    #[test]
    fn control_g_cancels_each_mode_through_esc_without_persisting() {
        let fixture = Fixture::new();

        for mode in [
            Mode::Normal,
            Mode::Adding("new".into(), AddPlacement::End),
            Mode::Editing {
                index: 0,
                input: "edited".into(),
            },
            Mode::Searching {
                input: "task".into(),
                original: Some(1),
            },
            Mode::SearchResults {
                input: "task".into(),
                original: Some(1),
            },
            Mode::ConfirmClear,
        ] {
            let search = matches!(mode, Mode::Searching { .. } | Mode::SearchResults { .. });
            let mut app = app_with_tasks(2);
            app.toggle(0, &fixture.storage).unwrap();
            app.toggle(1, &fixture.storage).unwrap();
            app.undo(&fixture.storage).unwrap();
            app.mode = mode;
            app.list.select(Some(0));

            let snapshot = (
                app.tasks.clone(),
                app.history.clone(),
                app.redo_history.clone(),
            );

            let bytes = std::fs::read(fixture.task_path()).unwrap();
            let mode_before = format!("{:?}", app.mode);

            for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
                press(
                    &mut app,
                    &fixture,
                    KeyEvent::new_with_kind(KeyCode::Char('g'), KeyModifiers::CONTROL, kind),
                );

                assert_eq!(format!("{:?}", app.mode), mode_before);
            }

            press(
                &mut app,
                &fixture,
                KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL),
            );

            assert_eq!(app.mode, Mode::Normal);
            assert_eq!(app.list.selected(), Some(if search { 1 } else { 0 }));
            assert_eq!(
                (
                    app.tasks.clone(),
                    app.history.clone(),
                    app.redo_history.clone()
                ),
                snapshot
            );
            assert_eq!(std::fs::read(fixture.task_path()).unwrap(), bytes);
        }
    }

    #[test]
    fn remembered_query_survives_accept_cancel_and_empty_input_but_not_restart() {
        for (query, matches) in [("task 0", 1), ("task", 2), ("missing", 0)] {
            for accept in [false, true] {
                let fixture = Fixture::new();
                let mut app = app_with_tasks(2);
                fixture.storage.save(&app.tasks).unwrap();

                for key in ['n', 'N'] {
                    press(&mut app, &fixture, KeyCode::Char(key));
                    assert_eq!(app.mode, Mode::Normal);
                    assert_eq!(app.list.selected(), Some(0));
                }

                press(&mut app, &fixture, KeyCode::Char('/'));
                app.paste(query);
                assert_eq!(app.last_search_query.as_deref(), Some(query));

                if accept {
                    press(&mut app, &fixture, KeyCode::Enter);

                    if matches == 2 {
                        press(&mut app, &fixture, KeyCode::Enter);
                    }

                    if matches == 0 {
                        assert!(matches!(app.mode, Mode::Searching { .. }));
                    }
                }

                press(&mut app, &fixture, KeyCode::Esc);
                assert_eq!(app.mode, Mode::Normal);
                assert_eq!(app.search_query(), None);
                assert_eq!(app.last_search_query.as_deref(), Some(query));

                press(&mut app, &fixture, KeyCode::Char('/'));
                assert_eq!(app.search_query(), Some(""));

                press(&mut app, &fixture, KeyCode::Enter);
                assert_eq!(app.last_search_query.as_deref(), Some(query));

                press(&mut app, &fixture, KeyCode::Char('/'));
                app.paste("x");
                press(&mut app, &fixture, KeyCode::Backspace);
                press(&mut app, &fixture, KeyCode::Esc);

                assert_eq!(app.last_search_query.as_deref(), Some("x"));

                let reopened = App::new(fixture.storage.load().unwrap());
                assert_eq!(reopened.last_search_query, None);
                assert_eq!(reopened.search_query(), None);
            }
        }
    }

    #[test]
    fn resumed_search_keeps_matching_selection_and_cycles_from_a_new_checkpoint() {
        let fixture = Fixture::new();

        let mut app = app_with_tasks(5);

        for index in [0, 2, 4] {
            app.tasks[index].text = format!("HiT {index}");
        }

        app.tasks[2].carried = true;
        app.tasks[4].completed = true;
        let original = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char('/'));
        app.paste("hit");
        press(&mut app, &fixture, KeyCode::Esc);

        for (selected, forward, backward) in [
            (None, 0, 4),
            (Some(0), 0, 0),
            (Some(1), 2, 0),
            (Some(2), 2, 2),
            (Some(3), 4, 2),
            (Some(4), 4, 4),
        ] {
            for (code, modifiers, first, next) in [
                (
                    KeyCode::Char('n'),
                    KeyModifiers::NONE,
                    forward,
                    if forward == 4 { 0 } else { forward + 2 },
                ),
                (
                    KeyCode::Char('N'),
                    KeyModifiers::SHIFT,
                    backward,
                    if backward == 0 { 4 } else { backward - 2 },
                ),
                (
                    KeyCode::Char('n'),
                    KeyModifiers::SHIFT,
                    backward,
                    if backward == 0 { 4 } else { backward - 2 },
                ),
            ] {
                app.list.select(selected);
                press(
                    &mut app,
                    &fixture,
                    KeyEvent::new_with_kind(code, modifiers, KeyEventKind::Release),
                );

                assert_eq!(app.mode, Mode::Normal);

                press(
                    &mut app,
                    &fixture,
                    KeyEvent::new_with_kind(code, modifiers, KeyEventKind::Repeat),
                );

                assert!(matches!(app.mode, Mode::SearchResults { .. }));
                assert_eq!(app.list.selected(), Some(first));

                press(
                    &mut app,
                    &fixture,
                    KeyEvent::new_with_kind(code, modifiers, KeyEventKind::Repeat),
                );

                assert_eq!(app.list.selected(), Some(next));

                press(&mut app, &fixture, KeyCode::Esc);
                assert_eq!(app.list.selected(), selected);
                assert_eq!(app.mode, Mode::Normal);

                press(&mut app, &fixture, KeyEvent::new(code, modifiers));
                press(&mut app, &fixture, KeyCode::Enter);
                assert_eq!(app.list.selected(), Some(first));
                assert_eq!(app.search_query(), None);
            }
        }

        assert_eq!(app.tasks, original);
        assert!(!app.can_undo() && !app.can_redo());

        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn resumed_search_uses_live_tasks_and_leaves_history_and_failed_matches_alone() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        app.tasks[0].text = "hit zero".into();
        app.tasks[2].text = "hit two".into();
        press(&mut app, &fixture, KeyCode::Char('/'));
        app.paste("hit");
        press(&mut app, &fixture, KeyCode::Esc);
        press(&mut app, &fixture, KeyCode::Char('d'));
        press(&mut app, &fixture, KeyCode::Char('N'));

        assert_eq!(app.list.selected(), Some(1));

        press(&mut app, &fixture, KeyCode::Esc);
        assert_eq!(app.list.selected(), Some(0));

        app.list.select(Some(1));
        app.mode = Mode::Editing {
            index: 1,
            input: "changed".into(),
        };
        press(&mut app, &fixture, KeyCode::Enter);
        let changed = task_state(&app);
        app.reveal_selection = false;

        for key in ['n', 'N'] {
            press(&mut app, &fixture, KeyCode::Char(key));
            assert_eq!(task_state(&app), changed);
            assert_eq!(app.mode, Mode::Normal);
            assert!(!app.reveal_selection);
        }

        press(&mut app, &fixture, KeyCode::Char('u'));
        press(&mut app, &fixture, KeyCode::Char('K'));
        let reordered = task_state(&app);
        press(&mut app, &fixture, KeyCode::Char('n'));

        assert_eq!(app.list.selected(), Some(0));

        press(&mut app, &fixture, KeyCode::Enter);
        assert_eq!(task_state(&app), reordered);
        assert_eq!(fixture.storage.load().unwrap(), app.tasks);
        assert_eq!(app.last_search_query.as_deref(), Some("hit"));

        app.tasks.clear();
        app.list.select(None);
        press(&mut app, &fixture, KeyCode::Char('n'));

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.list.selected(), None);
    }

    #[test]
    fn section_tab_keys_do_nothing_outside_normal_mode() {
        let fixture = Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b"]);
        let original = app.tasks.clone();

        for mode in [
            Mode::Adding("text".into(), AddPlacement::End),
            Mode::Editing {
                index: 0,
                input: "text".into(),
            },
            Mode::Searching {
                input: "text".into(),
                original: Some(0),
            },
            Mode::SearchResults {
                input: "text".into(),
                original: Some(0),
            },
            Mode::ConfirmClear,
        ] {
            app.mode = mode;
            let before = format!("{:?}", app.mode);
            let selected = app.list.selected();

            for code in [KeyCode::Tab, KeyCode::BackTab] {
                press(&mut app, &fixture, code);

                assert_eq!(format!("{:?}", app.mode), before);
                assert_eq!(app.view, SectionView::All);

                if !matches!(app.mode, Mode::Searching { .. }) {
                    assert_eq!(app.list.selected(), selected);
                }
            }

            assert_eq!(app.tasks, original);
        }

        assert!(!app.can_undo() && !app.can_redo());
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn carryover_no_ops_preserve_redo_and_completed_clearing_keeps_carried_tasks() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(1);

        for _ in 0..2 {
            press(&mut app, &fixture, KeyCode::Char('m'));
        }

        app.undo(&fixture.storage).unwrap();
        let carried = task_state(&app);
        let bytes = std::fs::read(fixture.task_path()).unwrap();

        for code in [KeyCode::Char('D'), KeyCode::Char('x')] {
            press(&mut app, &fixture, code);
            assert_eq!(app.mode, Mode::Normal);
            assert_eq!(task_state(&app), carried);
            assert_eq!(std::fs::read(fixture.task_path()).unwrap(), bytes);
        }

        press(&mut app, &fixture, KeyCode::Enter);
        press(&mut app, &fixture, KeyCode::Char('a'));
        app.paste("temporary");
        press(&mut app, &fixture, KeyCode::Enter);
        app.undo(&fixture.storage).unwrap();
        let completed = task_state(&app);

        assert!(app.can_redo());

        press(&mut app, &fixture, KeyCode::Char('m'));
        assert_eq!(task_state(&app), completed);

        let mut mixed = app_with_tasks(2);
        mixed.tasks[0].carried = true;
        mixed.tasks[1].completed = true;
        press(&mut mixed, &fixture, KeyCode::Char('x'));

        assert_eq!(mixed.tasks.len(), 1);
        assert!(mixed.tasks[0].carried);

        mixed.undo(&fixture.storage).unwrap();
        assert!(mixed.tasks[0].carried && mixed.tasks[1].completed);

        let mut empty = App::new(Vec::new());
        press(&mut empty, &fixture, KeyCode::Char('m'));
        assert!(empty.tasks.is_empty() && !empty.can_undo());
    }

    #[test]
    fn carryover_completion_and_runtime_history_are_coherent() {
        for action in [KeyCode::Enter, KeyCode::Char(' '), KeyCode::Null] {
            let fixture = Fixture::new();
            let mut app = app_with_tasks(2);
            press(&mut app, &fixture, KeyCode::Char('m'));

            assert!(app.tasks[0].carried);
            assert_eq!(app.history.len(), 1);

            press(&mut app, &fixture, KeyCode::Char('m'));
            assert!(!app.tasks[0].carried);

            app.undo(&fixture.storage).unwrap();
            assert!(app.tasks[0].carried);
            assert!(app.tasks[0].carried && app.can_redo());

            if action == KeyCode::Null {
                app.row_tasks = vec![Some(0), Some(1)];
                app.mouse(
                    MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column: 1,
                        row: 1,
                        modifiers: KeyModifiers::NONE,
                    },
                    Rect::new(1, 1, 20, 2),
                    &fixture.storage,
                )
                .unwrap();
            } else {
                press(&mut app, &fixture, action);
            }

            let index = app.list.selected().unwrap();
            assert!(app.tasks[index].completed && !app.tasks[index].carried);

            let history = app.history.len();
            press(&mut app, &fixture, KeyCode::Char('m'));
            assert_eq!(app.history.len(), history);

            app.undo(&fixture.storage).unwrap();
            assert!(app.tasks[0].carried && !app.tasks[0].completed);

            app.redo(&fixture.storage).unwrap();
            let index = app.list.selected().unwrap();
            assert!(app.tasks[index].completed && !app.tasks[index].carried);

            press(&mut app, &fixture, KeyCode::Enter);
            assert!(
                app.tasks
                    .iter()
                    .all(|task| !task.completed && !task.carried)
            );
            assert_eq!(fixture.storage.load().unwrap(), app.tasks);
        }
    }

    #[test]
    fn clearing_preserves_carried_order_selection_and_repeated_resets() {
        for selection in [None, Some(0), Some(1), Some(2), Some(3), Some(4)] {
            let fixture = Fixture::new();
            let mut app = app_with_tasks(5);
            app.tasks[1].carried = true;
            app.tasks[2].completed = true;
            app.tasks[3].carried = true;
            app.list.select(selection);
            let original = app.tasks.clone();
            let survivors = vec![original[1].clone(), original[3].clone()];
            press(&mut app, &fixture, KeyCode::Char('D'));

            assert_eq!(app.mode, Mode::ConfirmClear);
            assert_eq!(app.tasks, original);

            press(&mut app, &fixture, KeyCode::Enter);
            assert_eq!(app.tasks, survivors);
            assert_eq!(app.history.len(), 1);

            let expected = if selection.is_some_and(|index| index >= 3) {
                1
            } else {
                0
            };

            assert_eq!(app.list.selected(), Some(expected));

            for _ in 0..3 {
                press(&mut app, &fixture, KeyCode::Char('D'));
            }

            assert_eq!(app.mode, Mode::Normal);
            assert_eq!(app.history.len(), 1);

            app.undo(&fixture.storage).unwrap();
            assert_eq!(app.tasks, original);
            assert_eq!(app.list.selected(), selection);

            app.redo(&fixture.storage).unwrap();
            assert_eq!(app.tasks, survivors);
            assert_eq!(app.tasks, survivors);

            press(&mut app, &fixture, KeyCode::Char('x'));
            assert_eq!(app.tasks, survivors);

            for key in [KeyCode::Char('d'), KeyCode::Delete] {
                press(&mut app, &fixture, key);
            }

            assert!(app.tasks.is_empty());
        }
    }

    #[test]
    fn carryover_survives_other_operations_and_failed_saves_preserve_history() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(2);
        press(&mut app, &fixture, KeyCode::Char('m'));
        press(&mut app, &fixture, KeyCode::Char('J'));

        assert!(app.tasks[1].carried);

        press(&mut app, &fixture, KeyCode::Char('e'));
        app.paste(" edited");
        press(&mut app, &fixture, KeyCode::Enter);

        assert!(app.tasks[1].carried);

        press(&mut app, &fixture, KeyCode::Char('a'));
        app.paste("new");
        press(&mut app, &fixture, KeyCode::Enter);

        assert!(!app.tasks[2].carried);

        app.undo(&fixture.storage).unwrap();
        let original = task_state(&app);
        block_saves(&fixture);

        assert!(app.toggle_carryover(&fixture.storage).is_err());
        assert_eq!(task_state(&app), original);

        press(&mut app, &fixture, KeyCode::Char('D'));
        assert!(
            app.key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &fixture.storage
            )
            .is_err()
        );
        assert_eq!(task_state(&app), original);

        restore_saves(&fixture);
        assert_eq!(fixture.storage.load().unwrap(), app.tasks);

        app.mode = Mode::Normal;
        app.list.select(None);
        let before = task_state(&app);
        press(&mut app, &fixture, KeyCode::Char('m'));

        assert_eq!(task_state(&app), before);

        for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
            press(
                &mut app,
                &fixture,
                KeyEvent::new_with_kind(KeyCode::Char('m'), KeyModifiers::NONE, kind),
            );

            assert_eq!(task_state(&app), before);
        }

        for mode in [
            Mode::Adding("".into(), AddPlacement::End),
            Mode::Editing {
                index: 0,
                input: "".into(),
            },
            Mode::Searching {
                input: "".into(),
                original: None,
            },
            Mode::SearchResults {
                input: "".into(),
                original: None,
            },
            Mode::ConfirmClear,
        ] {
            app.mode = mode;
            press(&mut app, &fixture, KeyCode::Char('m'));
            assert_eq!(app.tasks, fixture.storage.load().unwrap());
            assert_eq!(app.history.len(), original.history.len());
            assert_eq!(app.redo_history.len(), original.redo_history.len());
        }
    }

    #[test]
    fn conventional_aliases_match_primary_actions_and_ignore_repeats() {
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;

        for (primary, alias) in [
            (
                KeyEvent::new(KeyCode::Char('u'), none),
                KeyEvent::new(KeyCode::Char('z'), ctrl),
            ),
            (
                KeyEvent::new(KeyCode::Char('/'), none),
                KeyEvent::new(KeyCode::Char('f'), ctrl),
            ),
            (
                KeyEvent::new(KeyCode::Char('d'), none),
                KeyEvent::new(KeyCode::Delete, none),
            ),
        ] {
            for count in [0, 3] {
                let fixture = Fixture::new();
                let mut outcomes = Vec::new();
                let tasks = app_with_tasks(count).tasks;

                for event in [primary, alias] {
                    let mut app = App::new(tasks.clone());

                    if count > 0 {
                        app.toggle(0, &fixture.storage).unwrap();
                        app.toggle(1, &fixture.storage).unwrap();
                        app.undo(&fixture.storage).unwrap();
                        app.list.select(Some(2));
                    }

                    let before = (
                        app.tasks.clone(),
                        app.history.clone(),
                        app.redo_history.clone(),
                    );

                    for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
                        press(
                            &mut app,
                            &fixture,
                            KeyEvent::new_with_kind(event.code, event.modifiers, kind),
                        );

                        assert_eq!(app.mode, Mode::Normal);
                        assert_eq!(
                            (
                                app.tasks.clone(),
                                app.history.clone(),
                                app.redo_history.clone()
                            ),
                            before
                        );
                    }

                    assert!(!press(&mut app, &fixture, event));

                    let mode = format!("{:?}", app.mode);
                    outcomes.push((task_state(&app), mode));
                }

                assert_eq!(outcomes[0], outcomes[1]);

                if count == 0 {
                    assert!(!fixture.task_path().exists());
                }
            }
        }
    }

    #[test]
    fn conventional_aliases_do_not_leak_into_other_modes_or_replace_text_delete() {
        let fixture = Fixture::new();

        for mode in [
            Mode::Adding("abc".into(), AddPlacement::End),
            Mode::Editing {
                index: 0,
                input: "abc".into(),
            },
            Mode::Searching {
                input: "abc".into(),
                original: Some(0),
            },
            Mode::SearchResults {
                input: "task".into(),
                original: Some(0),
            },
            Mode::ConfirmClear,
        ] {
            let mut app = app_with_tasks(2);
            app.toggle(0, &fixture.storage).unwrap();
            app.toggle(1, &fixture.storage).unwrap();
            app.undo(&fixture.storage).unwrap();
            app.mode = mode;

            if let Mode::Adding(input, _)
            | Mode::Editing { input, .. }
            | Mode::Searching { input, .. } = &mut app.mode
            {
                input.set_cursor(0);
            }

            let before = (
                app.tasks.clone(),
                app.history.clone(),
                app.redo_history.clone(),
            );

            let original_mode = format!("{:?}", app.mode);

            for character in ['z', 'y'] {
                press(
                    &mut app,
                    &fixture,
                    KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL),
                );

                assert_eq!(format!("{:?}", app.mode), original_mode);
            }

            press(&mut app, &fixture, KeyCode::Delete);

            if let Mode::Adding(input, _)
            | Mode::Editing { input, .. }
            | Mode::Searching { input, .. } = &app.mode
            {
                assert_eq!(input.text(), "bc");
            } else {
                assert_eq!(format!("{:?}", app.mode), original_mode);
            }

            assert_eq!(
                (
                    app.tasks.clone(),
                    app.history.clone(),
                    app.redo_history.clone()
                ),
                before
            );
        }
    }

    #[test]
    fn alias_save_failures_preserve_current_state_and_both_histories() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(2);
        app.toggle(0, &fixture.storage).unwrap();
        app.toggle(1, &fixture.storage).unwrap();
        app.undo(&fixture.storage).unwrap();
        let before = task_state(&app);
        block_saves(&fixture);

        for event in [
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
        ] {
            assert!(app.key(event, &fixture.storage).is_err());
            assert_eq!(task_state(&app), before);
        }
    }

    #[test]
    fn input_shortcuts_dispatch_without_leaking_modified_characters() {
        let fixture = Fixture::new();
        let text = "one  e\u{301}界  👩‍💻 end";
        let end_word = "one  e\u{301}界  👩‍💻 ".len();

        for edit in [false, true] {
            for kind in [KeyEventKind::Press, KeyEventKind::Repeat] {
                for (code, modifiers, start, expected_cursor, expected_text) in [
                    (
                        KeyCode::Left,
                        KeyModifiers::NONE,
                        text.len(),
                        text.len() - 1,
                        text,
                    ),
                    (KeyCode::Right, KeyModifiers::NONE, 0, 1, text),
                    (KeyCode::Home, KeyModifiers::NONE, text.len(), 0, text),
                    (KeyCode::End, KeyModifiers::NONE, 0, text.len(), text),
                    (
                        KeyCode::Char('a'),
                        KeyModifiers::CONTROL,
                        text.len(),
                        0,
                        text,
                    ),
                    (
                        KeyCode::Char('A'),
                        KeyModifiers::CONTROL,
                        text.len(),
                        0,
                        text,
                    ),
                    (
                        KeyCode::Char('e'),
                        KeyModifiers::CONTROL,
                        0,
                        text.len(),
                        text,
                    ),
                    (
                        KeyCode::Left,
                        KeyModifiers::CONTROL,
                        text.len(),
                        end_word,
                        text,
                    ),
                    (KeyCode::Right, KeyModifiers::CONTROL, 0, 5, text),
                    (
                        KeyCode::Backspace,
                        KeyModifiers::CONTROL,
                        text.len(),
                        end_word,
                        &text[..end_word],
                    ),
                    (
                        KeyCode::Backspace,
                        KeyModifiers::NONE,
                        3,
                        2,
                        "on  e\u{301}界  👩‍💻 end",
                    ),
                    (KeyCode::Delete, KeyModifiers::NONE, 5, 5, "one  界  👩‍💻 end"),
                    (
                        KeyCode::Char('Z'),
                        KeyModifiers::SHIFT,
                        0,
                        1,
                        "Zone  e\u{301}界  👩‍💻 end",
                    ),
                ] {
                    let mut app = app_with_tasks(1);
                    let original = app.tasks.clone();
                    let mut input = TextInput::from(text);
                    input.set_cursor(start);
                    app.mode = if edit {
                        Mode::Editing { index: 0, input }
                    } else {
                        Mode::Adding(input, AddPlacement::End)
                    };
                    press(
                        &mut app,
                        &fixture,
                        KeyEvent::new_with_kind(code, modifiers, kind),
                    );

                    let input = match &app.mode {
                        Mode::Adding(input, _) | Mode::Editing { input, .. } => input,
                        _ => unreachable!(),
                    };

                    assert_eq!(input.cursor(), expected_cursor, "{code:?} {modifiers:?}");
                    assert_eq!(input.text(), expected_text);
                    assert_eq!(app.tasks, original);
                    assert!(!app.can_undo());
                }
            }

            let mut app = app_with_tasks(1);
            app.mode = if edit {
                Mode::Editing {
                    index: 0,
                    input: text.into(),
                }
            } else {
                Mode::Adding(text.into(), AddPlacement::End)
            };
            let original_mode = format!("{:?}", app.mode);

            for (code, modifiers) in [
                (KeyCode::Delete, KeyModifiers::CONTROL),
                (KeyCode::Char('q'), KeyModifiers::CONTROL),
                (KeyCode::Left, KeyModifiers::CONTROL | KeyModifiers::ALT),
                (
                    KeyCode::Char('e'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                ),
                (KeyCode::Left, KeyModifiers::ALT),
                (KeyCode::Char('Q'), KeyModifiers::SUPER),
            ] {
                press(&mut app, &fixture, KeyEvent::new(code, modifiers));
                assert_eq!(format!("{:?}", app.mode), original_mode);
            }

            press(
                &mut app,
                &fixture,
                KeyEvent::new_with_kind(KeyCode::Home, KeyModifiers::NONE, KeyEventKind::Release),
            );

            assert_eq!(format!("{:?}", app.mode), original_mode);
        }

        for mode in [Mode::Normal, Mode::ConfirmClear] {
            let mut app = app_with_tasks(1);
            app.mode = mode;
            let original_mode = format!("{:?}", app.mode);

            for code in [
                KeyCode::Char('a'),
                KeyCode::Char('e'),
                KeyCode::Left,
                KeyCode::Right,
                KeyCode::Backspace,
            ] {
                press(
                    &mut app,
                    &fixture,
                    KeyEvent::new(code, KeyModifiers::CONTROL),
                );

                assert_eq!(format!("{:?}", app.mode), original_mode);
                assert_eq!(app.tasks.len(), 1);
                assert!(!app.can_undo());
            }
        }

        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn search_uses_original_anchor_and_accepts_or_cancels_without_mutating() {
        let fixture = Fixture::new();

        let mut app = App::new(
            [
                ("alpha zero", false),
                ("alpine", false),
                ("beta", false),
                ("ALPHA done", true),
                ("alpha last", true),
            ]
            .map(|(text, completed)| Task {
                completed,
                ..Task::new(text)
            })
            .into(),
        );
        app.list.select(Some(2));
        let original = app.tasks.clone();
        fixture.storage.save(&original).unwrap();
        let bytes = std::fs::read(fixture.task_path()).unwrap();
        press(&mut app, &fixture, KeyCode::Char('/'));

        assert_eq!(app.list.selected(), None);

        app.paste("a");
        assert_eq!(app.list.selected(), Some(2));

        press(&mut app, &fixture, KeyCode::Char('l'));
        assert_eq!(app.list.selected(), Some(3));

        press(&mut app, &fixture, KeyCode::Backspace);
        assert_eq!(app.list.selected(), Some(2));

        app.paste("lpha");
        assert_eq!(app.list.selected(), Some(3));

        press(&mut app, &fixture, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::SearchResults { .. }));

        repeat(&mut app, &fixture, KeyCode::Enter, KeyModifiers::NONE);
        assert!(matches!(app.mode, Mode::SearchResults { .. }));

        for (code, expected) in [
            (KeyCode::Char('n'), 4),
            (KeyCode::Down, 0),
            (KeyCode::Char('N'), 4),
            (KeyCode::Up, 3),
        ] {
            repeat(&mut app, &fixture, code, KeyModifiers::NONE);
            assert_eq!(app.list.selected(), Some(expected));
        }

        repeat(&mut app, &fixture, KeyCode::Char('n'), KeyModifiers::SHIFT);
        assert_eq!(app.list.selected(), Some(0));

        for code in [
            KeyCode::Char('a'),
            KeyCode::Char('e'),
            KeyCode::Char('d'),
            KeyCode::Char('D'),
            KeyCode::Char('x'),
            KeyCode::Char('u'),
            KeyCode::Char('J'),
            KeyCode::Char('K'),
            KeyCode::Char('w'),
            KeyCode::Char('q'),
            KeyCode::Char(' '),
            KeyCode::PageDown,
            KeyCode::Home,
        ] {
            assert!(!press(&mut app, &fixture, code));
            assert_eq!(app.list.selected(), Some(0));
            assert!(matches!(app.mode, Mode::SearchResults { .. }));
        }

        app.paste("ignored");
        assert_eq!(app.search_query(), Some("alpha"));

        press(&mut app, &fixture, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.list.selected(), Some(2));
        assert!(app.reveal_selection);

        press(&mut app, &fixture, KeyCode::Char('/'));
        app.paste("alpha");
        press(&mut app, &fixture, KeyCode::Enter);
        press(&mut app, &fixture, KeyCode::Enter);
        repeat(&mut app, &fixture, KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.list.selected(), Some(3));
        assert_eq!(app.tasks, original);
        assert!(!app.can_undo());
        assert_eq!(std::fs::read(fixture.task_path()).unwrap(), bytes);
    }

    #[test]
    fn search_input_reuses_cursor_editing_and_paste_normalization() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(1);
        press(&mut app, &fixture, KeyCode::Char('/'));
        app.paste("α beta");

        for (code, modifiers, text, cursor) in [
            (KeyCode::Left, KeyModifiers::CONTROL, "α beta", 3),
            (KeyCode::Backspace, KeyModifiers::CONTROL, "beta", 0),
            (KeyCode::Delete, KeyModifiers::NONE, "eta", 0),
            (KeyCode::Right, KeyModifiers::NONE, "eta", 1),
            (KeyCode::Backspace, KeyModifiers::NONE, "ta", 0),
            (KeyCode::Char('e'), KeyModifiers::NONE, "eta", 1),
            (
                KeyCode::Char('\u{301}'),
                KeyModifiers::NONE,
                "e\u{301}ta",
                3,
            ),
            (KeyCode::End, KeyModifiers::NONE, "e\u{301}ta", 5),
            (KeyCode::Home, KeyModifiers::NONE, "e\u{301}ta", 0),
            (KeyCode::Right, KeyModifiers::CONTROL, "e\u{301}ta", 5),
            (KeyCode::Char('a'), KeyModifiers::CONTROL, "e\u{301}ta", 0),
            (KeyCode::Char('e'), KeyModifiers::CONTROL, "e\u{301}ta", 5),
        ] {
            repeat(&mut app, &fixture, code, modifiers);

            let Mode::Searching { input, .. } = &app.mode else {
                panic!()
            };

            assert_eq!(input.text(), text);
            assert_eq!(input.cursor(), cursor);
        }

        repeat(&mut app, &fixture, KeyCode::Left, KeyModifiers::CONTROL);
        repeat(&mut app, &fixture, KeyCode::Right, KeyModifiers::NONE);
        app.paste("X\r\n\tY");

        assert_eq!(app.search_query(), Some("e\u{301}X  Yta"));

        press(
            &mut app,
            &fixture,
            KeyEvent::new(
                KeyCode::Char('€'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ),
        );

        assert_eq!(
            app.search_query(),
            Some(if cfg!(windows) {
                "e\u{301}X  Y€ta"
            } else {
                "e\u{301}X  Yta"
            })
        );

        press(&mut app, &fixture, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Searching { .. }));
        assert_eq!(app.list.selected(), None);

        press(&mut app, &fixture, KeyCode::Esc);
        assert_eq!(app.list.selected(), Some(0));
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn empty_and_single_result_search_preserve_existing_undo_and_ignore_mouse() {
        let fixture = Fixture::new();

        for count in [0, 1] {
            let mut app = app_with_tasks(count);

            if count == 1 {
                press(&mut app, &fixture, KeyCode::Char(' '));
            }

            let original = app.tasks.clone();
            let selection = app.list.selected();
            let history = app.history.len();
            press(&mut app, &fixture, KeyCode::Char('/'));
            press(&mut app, &fixture, KeyCode::Enter);

            assert_eq!(app.mode, Mode::Normal);
            assert_eq!(app.list.selected(), selection);

            press(&mut app, &fixture, KeyCode::Char('/'));
            app.paste("task");
            let rows = Rect::new(0, 0, 20, 3);

            for kind in [
                MouseEventKind::Down(MouseButton::Left),
                MouseEventKind::ScrollDown,
            ] {
                app.mouse(
                    MouseEvent {
                        kind,
                        column: 1,
                        row: 1,
                        modifiers: KeyModifiers::NONE,
                    },
                    rows,
                    &fixture.storage,
                )
                .unwrap();
            }

            press(&mut app, &fixture, KeyCode::Enter);
            assert_eq!(app.mode == Mode::Normal, count == 1);

            for code in [
                KeyCode::Char('n'),
                KeyCode::Char('N'),
                KeyCode::Down,
                KeyCode::Up,
            ] {
                press(&mut app, &fixture, code);
            }

            assert_eq!(app.list.selected(), selection);

            press(
                &mut app,
                &fixture,
                KeyEvent::new_with_kind(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Release),
            );

            assert!(app.search_query().is_some());

            press(&mut app, &fixture, KeyCode::Esc);
            assert_eq!(app.tasks, original);
            assert_eq!(app.list.selected(), selection);
            assert_eq!(app.history.len(), history);

            if count == 1 {
                press(&mut app, &fixture, KeyCode::Char('u'));
                assert!(!app.tasks[0].completed);
            }
        }
    }

    #[test]
    fn separators_normalize_in_all_input_modes_without_submitting() {
        let fixture = Fixture::new();
        let text = "\u{2028}e\u{301}界👩‍💻\u{2029}\u{a0}\u{2003}\u{202f}\r\n\t\0";

        for mode in 0..3 {
            for pasted in [false, true] {
                let mut app = app_with_tasks(1);
                let original = task_state(&app);
                let input = TextInput::from("start");
                app.mode = match mode {
                    0 => Mode::Adding(input, AddPlacement::End),
                    1 => Mode::Editing { index: 0, input },
                    _ => Mode::Searching {
                        input,
                        original: Some(0),
                    },
                };

                if pasted {
                    app.paste(text);
                } else {
                    for character in text.chars() {
                        press(&mut app, &fixture, KeyCode::Char(character));
                    }
                }

                let expected = format!(
                    "start e\u{301}界👩‍💻 \u{a0}\u{2003}\u{202f}{}",
                    if pasted { "   " } else { "" }
                );

                let input = match &app.mode {
                    Mode::Adding(input, _)
                    | Mode::Editing { input, .. }
                    | Mode::Searching { input, .. } => input,
                    _ => panic!("input submitted or cancelled unexpectedly"),
                };

                assert_eq!(input.text(), expected);
                assert_eq!(input.cursor(), expected.len());
                assert_eq!(app.tasks, original.tasks);
                assert_eq!(app.history, original.history);
                assert_eq!(app.redo_history, original.redo_history);

                if mode == 2 {
                    assert_eq!(app.last_search_query.as_deref(), Some(expected.as_str()));
                }
            }
        }

        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn altgr_characters_are_windows_input_only_and_respect_event_kinds() {
        let fixture = Fixture::new();
        let ctrl_alt = KeyModifiers::CONTROL | KeyModifiers::ALT;

        for mode in 0..3 {
            for kind in [
                KeyEventKind::Press,
                KeyEventKind::Repeat,
                KeyEventKind::Release,
            ] {
                for modifiers in [
                    ctrl_alt,
                    ctrl_alt | KeyModifiers::SHIFT,
                    ctrl_alt | KeyModifiers::SUPER,
                ] {
                    for character in [
                        '@', '{', '€', 'ł', 'A', 'a', 'e', '\u{301}', '\n', '\0', '\u{2028}',
                        '\u{2029}',
                    ] {
                        let mut app = app_with_tasks(1);
                        let original = app.tasks.clone();
                        let mut input = TextInput::from("e\u{301}👩‍💻suffix");
                        input.set_cursor("e\u{301}".len());
                        let mut expected = input.clone();

                        if cfg!(windows)
                            && kind != KeyEventKind::Release
                            && !modifiers.contains(KeyModifiers::SUPER)
                            && !character.is_control()
                        {
                            let character = match character {
                                '\u{2028}' | '\u{2029}' => ' ',
                                other => other,
                            };
                            expected.insert(character.encode_utf8(&mut [0; 4]));
                        }

                        app.mode = match mode {
                            0 => Mode::Adding(input, AddPlacement::End),
                            1 => Mode::Editing { index: 0, input },
                            _ => Mode::Searching {
                                input,
                                original: Some(0),
                            },
                        };

                        assert!(!press(
                            &mut app,
                            &fixture,
                            KeyEvent::new_with_kind(KeyCode::Char(character), modifiers, kind)
                        ));

                        let actual = match &app.mode {
                            Mode::Adding(input, _)
                            | Mode::Editing { input, .. }
                            | Mode::Searching { input, .. } => input,
                            _ => panic!("AltGr must not change mode"),
                        };

                        assert_eq!(actual, &expected, "{character:?} {modifiers:?} {kind:?}");
                        assert_eq!(app.tasks, original);
                        assert!(!app.can_undo());
                    }
                }
            }
        }

        for mode in [Mode::Normal, Mode::ConfirmClear] {
            let mut app = app_with_tasks(1);
            app.mode = mode;
            let original = format!("{:?}", app.mode);

            for character in ['a', 'e', 'd', 'D', 'u', 'q', ' '] {
                assert!(!press(
                    &mut app,
                    &fixture,
                    KeyEvent::new(KeyCode::Char(character), ctrl_alt)
                ));
                assert_eq!(format!("{:?}", app.mode), original);
                assert_eq!(app.tasks.len(), 1);
                assert!(!app.can_undo());
            }
        }

        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn typing_and_paste_insert_at_the_cursor_in_both_input_modes() {
        let fixture = Fixture::new();

        for edit in [false, true] {
            let mut app = app_with_tasks(1);
            let original = app.tasks.clone();
            let mut input = TextInput::from("ab");
            input.set_cursor(1);
            app.mode = if edit {
                Mode::Editing { index: 0, input }
            } else {
                Mode::Adding(input, AddPlacement::End)
            };
            press(&mut app, &fixture, KeyCode::Char('界'));
            app.paste("e\u{301}\r\nq\tD\u{2028}");

            match &app.mode {
                Mode::Adding(input, _) | Mode::Editing { input, .. } => {
                    assert_eq!(input.text(), "a界e\u{301} q D b");
                    assert_eq!(&input.text()[input.cursor()..], "b");
                }
                _ => unreachable!(),
            }

            press(&mut app, &fixture, KeyCode::Backspace);
            press(&mut app, &fixture, KeyCode::Enter);
            let index = if edit { 0 } else { 1 };

            assert_eq!(app.tasks[index].text, "a界e\u{301} q Db");
            assert_eq!(fixture.storage.load().unwrap(), app.tasks);
            assert_eq!(app.history.len(), 1);

            press(&mut app, &fixture, KeyCode::Char('u'));
            assert_eq!(app.tasks, original);
        }
    }

    #[test]
    fn backspace_removes_whole_graphemes_in_add_and_edit_modes() {
        for start in [KeyCode::Char('a'), KeyCode::Char('e')] {
            let fixture = Fixture::new();
            let mut app = app_with_tasks(1);
            let original = app.tasks.clone();
            press(&mut app, &fixture, start);

            let prefix = match &app.mode {
                Mode::Adding(input, _) | Mode::Editing { input, .. } => input.text().to_owned(),
                _ => unreachable!(),
            };

            for grapheme in ["e\u{301}", "👩‍💻", "👨‍👩‍👧‍👦", "👍🏽", "🇺🇳"]
            {
                for character in grapheme.chars() {
                    press(&mut app, &fixture, KeyCode::Char(character));
                }

                press(&mut app, &fixture, KeyCode::Backspace);

                match &app.mode {
                    Mode::Adding(input, _) | Mode::Editing { input, .. } => {
                        assert_eq!(input.text(), prefix)
                    }
                    _ => unreachable!(),
                }
            }

            app.paste("e\u{301}👩‍💻");
            repeat(&mut app, &fixture, KeyCode::Backspace, KeyModifiers::NONE);

            match &app.mode {
                Mode::Adding(input, _) | Mode::Editing { input, .. } => {
                    assert_eq!(input.text(), &format!("{prefix}e\u{301}"))
                }
                _ => unreachable!(),
            }

            for _ in 0..prefix.graphemes(true).count() + 2 {
                repeat(&mut app, &fixture, KeyCode::Backspace, KeyModifiers::NONE);
            }

            match &app.mode {
                Mode::Adding(input, _) | Mode::Editing { input, .. } => {
                    assert!(input.text().is_empty())
                }
                _ => unreachable!(),
            }

            assert_eq!(app.tasks, original);
            assert!(!fixture.task_path().exists());
        }
    }

    #[test]
    fn pasted_multiline_text_is_literal_and_requires_explicit_confirmation() {
        for ending in [KeyCode::Enter, KeyCode::Esc] {
            let fixture = Fixture::new();
            let mut app = app_with_tasks(1);
            let original = app.tasks.clone();
            press(&mut app, &fixture, KeyCode::Char('a'));
            app.paste("Start ");
            app.paste("日本語\r\nq\nD\rd\tx\0u\u{0008}\u{001b}[13u\u{0085}👩‍💻\u{2028}e\u{2029}end");
            let expected = "Start 日本語 q D d x u  [13u 👩‍💻 e end";

            assert_eq!(app.mode, Mode::Adding(expected.into(), AddPlacement::End));
            assert_eq!(app.tasks, original);
            assert_eq!(app.list.selected(), Some(0));
            assert!(!fixture.task_path().exists());

            press(&mut app, &fixture, ending);
            assert_eq!(app.mode, Mode::Normal);

            if ending == KeyCode::Enter {
                assert_eq!(app.tasks.len(), 2);
                assert_eq!(app.tasks[1].text, expected);
                assert!(!app.tasks[1].completed);
                assert_eq!(fixture.storage.load().unwrap(), app.tasks);
            } else {
                assert_eq!(app.tasks, original);
                assert!(!fixture.task_path().exists());
            }
        }
    }

    #[test]
    fn paste_appends_to_edit_input_and_preserves_completion_order_and_undo() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        app.list.select(Some(1));
        app.tasks[1].completed = true;
        let original = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char('e'));
        app.paste(" \r\nD\nd q日本語");

        assert_eq!(
            app.mode,
            Mode::Editing {
                index: 1,
                input: "task 1  D d q日本語".into()
            }
        );
        assert_eq!(app.tasks, original);
        assert!(!fixture.task_path().exists());

        press(&mut app, &fixture, KeyCode::Enter);
        let mut expected = original.clone();
        expected[1].text = "task 1  D d q日本語".into();

        assert_eq!(app.tasks, expected);
        assert_eq!(app.list.selected(), Some(1));
        assert_eq!(fixture.storage.load().unwrap(), expected);

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, original);
    }

    #[test]
    fn paste_in_normal_or_confirmation_mode_cannot_act_or_consume_undo() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        app.list.select(Some(1));
        let original = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char(' '));
        let modified = app.tasks.clone();

        for confirm in [false, true] {
            if confirm {
                press(&mut app, &fixture, KeyCode::Char('D'));
            }

            app.paste("q\r\nD\r\nd\ra\nx u\u{001b}日本語");
            app.paste("");
            assert_eq!(app.tasks, modified);
            assert_eq!(app.list.selected(), Some(1));
            assert_eq!(
                app.mode,
                if confirm {
                    Mode::ConfirmClear
                } else {
                    Mode::Normal
                }
            );
            assert_eq!(fixture.storage.load().unwrap(), modified);
        }

        press(&mut app, &fixture, KeyCode::Esc);
        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, original);
    }

    #[test]
    fn page_keys_are_safe_at_boundaries_and_in_input_modes() {
        let fixture = Fixture::new();

        for count in [0, 1, 3] {
            let mut app = app_with_tasks(count);
            app.row_tasks = (0..count).map(Some).collect();
            app.viewport_height = 2;
            let original = app.tasks.clone();

            for code in [
                KeyCode::PageUp,
                KeyCode::PageDown,
                KeyCode::PageDown,
                KeyCode::PageUp,
            ] {
                press(&mut app, &fixture, code);
                assert_eq!(app.tasks, original);
                assert!(!app.can_undo());
                assert!(app.list.selected().is_none_or(|index| index < count));
            }

            for mode in [
                Mode::Adding("input".into(), AddPlacement::End),
                Mode::Editing {
                    index: 0,
                    input: "input".into(),
                },
                Mode::ConfirmClear,
            ] {
                app.mode = mode;
                let selected = app.list.selected();
                press(&mut app, &fixture, KeyCode::PageDown);
                press(&mut app, &fixture, KeyCode::PageUp);

                assert_eq!(app.list.selected(), selected);

                match &app.mode {
                    Mode::Adding(input, _) | Mode::Editing { input, .. } => {
                        assert_eq!(input.text(), "input")
                    }
                    mode => assert_eq!(*mode, Mode::ConfirmClear),
                }
            }
        }

        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn first_and_last_navigation_preserves_tasks_and_undo_and_is_modal() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        app.list.select(Some(1));
        let original = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char(' '));
        let changed = app.tasks.clone();

        for (code, selected) in [
            (KeyCode::Char('G'), 2),
            (KeyCode::Char('g'), 0),
            (KeyCode::End, 2),
            (KeyCode::Home, 0),
        ] {
            app.reveal_selection = false;
            press(&mut app, &fixture, code);
            assert_eq!(app.list.selected(), Some(selected));
            assert!(app.reveal_selection);
            assert_eq!(app.tasks, changed);
            assert_eq!(fixture.storage.load().unwrap(), changed);

            let mut empty = app_with_tasks(0);
            press(&mut empty, &fixture, code);
            assert_eq!(empty.list.selected(), None);
        }

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, original);
        assert_eq!(app.list.selected(), Some(1));

        for code in [KeyCode::Char('G'), KeyCode::Char('g')] {
            press(&mut app, &fixture, KeyEvent::new(code, KeyModifiers::SHIFT));
            assert_eq!(app.list.selected(), Some(2));
        }

        app.list.select(Some(1));

        for mode in [
            Mode::Adding(TextInput::default(), AddPlacement::End),
            Mode::Editing {
                index: 1,
                input: TextInput::default(),
            },
        ] {
            app.mode = mode;

            for code in [
                KeyCode::Char('g'),
                KeyCode::Char('G'),
                KeyCode::Home,
                KeyCode::End,
            ] {
                press(&mut app, &fixture, code);
            }

            match &app.mode {
                Mode::Adding(input, _) | Mode::Editing { input, .. } => {
                    assert_eq!(input.text(), "gG")
                }
                _ => unreachable!(),
            }

            assert_eq!(app.list.selected(), Some(1));
        }

        app.mode = Mode::ConfirmClear;

        for code in [
            KeyCode::Char('g'),
            KeyCode::Char('G'),
            KeyCode::Home,
            KeyCode::End,
        ] {
            press(&mut app, &fixture, code);
        }

        assert_eq!(app.mode, Mode::ConfirmClear);
        assert_eq!(app.tasks, original);
        assert_eq!(app.list.selected(), Some(1));
    }

    fn type_keys(app: &mut App, fixture: &Fixture, keys: &str) {
        for character in keys.chars() {
            press(app, fixture, KeyCode::Char(character));
        }
    }

    fn task_json(created_at: &str) -> String {
        format!(
            r#"{{"id":"00000000-0000-4000-8000-000000000001","created_at":"{created_at}","text":"t","completed":false}}"#
        )
    }

    #[test]
    fn creation_times_round_trip_as_whole_second_utc() {
        for (text, unix_seconds) in [
            ("1970-01-01T00:00:00Z", 0),
            ("2000-02-29T00:00:00Z", 951_782_400),
            ("1999-12-31T23:59:59Z", 946_684_799),
            ("2100-03-01T00:00:00Z", 4_107_542_400),
            ("2026-11-07T19:24:05Z", 1_794_079_445),
        ] {
            let task: Task = serde_json::from_str(&task_json(text)).unwrap();

            assert_eq!(task.created_at.unix_timestamp(), unix_seconds);
            assert_eq!(serde_json::to_string(&task).unwrap(), task_json(text));
        }

        let now = current_time();
        assert_eq!(now.offset(), time::UtcOffset::UTC);
        assert_eq!(now.nanosecond(), 0);

        let written = serde_json::to_value(Task::new("new")).unwrap();
        let written = written["created_at"].as_str().unwrap();
        assert!(written.len() == 20 && written.ends_with('Z'), "{written}");
    }

    #[test]
    fn non_canonical_or_impossible_creation_times_are_rejected() {
        for text in [
            "",
            "2026-11-07",
            "2026-11-07T19:24:05",
            "2026-11-07 19:24:05Z",
            "2026-11-07t19:24:05z",
            "2026-11-07T19:24:05.5Z",
            "2026-11-07T19:24:05+00:00",
            "2026-11-07T21:24:05+02:00",
            "2026-02-29T00:00:00Z",
            "2100-02-29T00:00:00Z",
            "2026-11-31T00:00:00Z",
            "2026-11-07T24:00:00Z",
            "2026-11-07T23:60:00Z",
            "2026-11-07T23:59:60Z",
            "２026-11-07T19:24:05Z",
        ] {
            assert!(
                serde_json::from_str::<Task>(&task_json(text)).is_err(),
                "{text:?}"
            );
        }

        assert!(serde_json::from_str::<Task>(&task_json("2024-02-29T00:00:00Z")).is_ok());
    }

    #[test]
    fn a_current_file_is_not_rewritten_without_changes() {
        let fixture = Fixture::new();
        let app = app_with_tasks(3);
        fixture.storage.save(&app.tasks).unwrap();
        let saved = std::fs::read(fixture.task_path()).unwrap();

        let mut loaded = App::from_document(fixture.storage.load_document().unwrap());
        loaded.save_selection(&fixture.storage).unwrap();

        assert_eq!(loaded.tasks, app.tasks);
        assert_eq!(std::fs::read(fixture.task_path()).unwrap(), saved);
    }

    #[test]
    fn typed_prefixes_set_the_section_field_and_edits_or_pastes_keep_it_consistent() {
        let fixture = Fixture::new();
        let mut app = App::new(Vec::new());

        type_keys(&mut app, &fixture, "a");
        app.paste("Work: fix bug");
        press(&mut app, &fixture, KeyCode::Enter);

        let task = &app.tasks[0];
        assert_eq!(
            (task.section.as_deref(), task.text.as_str()),
            (Some("Work"), "fix bug")
        );
        let id = task.id;

        for (typed, section, body) in [
            ("Home: fix bug", Some("Home"), "fix bug"),
            ("fix bug", None, "fix bug"),
            ("Work: fix bug", Some("Work"), "fix bug"),
        ] {
            press(&mut app, &fixture, KeyCode::Char('e'));
            press(
                &mut app,
                &fixture,
                KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            );
            app.paste(typed);
            press(&mut app, &fixture, KeyCode::Enter);

            let task = &app.tasks[0];
            assert_eq!(
                (task.section.as_deref(), task.text.as_str()),
                (section, body)
            );
            assert_eq!(task.id, id);
        }

        // Yank and paste carry the section through the typed form.
        app.show_section(SectionView::All);
        type_keys(&mut app, &fixture, "yp");
        assert_eq!(app.tasks[1].section.as_deref(), Some("Work"));
        assert_ne!(app.tasks[1].id, id);

        assert_eq!(
            crate::export::markdown(&app.tasks[..1]),
            "- [ ] Work: fix bug\n"
        );
    }

    #[test]
    fn task_identity_survives_changes_and_new_tasks_get_their_own() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        let identity = |task: &Task| (task.id, task.created_at);
        let original: Vec<_> = app.tasks.iter().map(identity).collect();

        for keys in ["e", "x", " ", "m", "J", "K"] {
            type_keys(&mut app, &fixture, keys);

            if keys == "e" {
                type_keys(&mut app, &fixture, " edited");
                press(&mut app, &fixture, KeyCode::Enter);
            }
        }

        press(&mut app, &fixture, KeyCode::Char('u'));
        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
        );

        let mut kept: Vec<_> = app.tasks.iter().map(identity).collect();
        kept.sort();
        let mut expected = original.clone();
        expected.sort();
        assert_eq!(kept, expected);
        assert_eq!(
            saved_ids(&fixture),
            app.tasks.iter().map(|task| task.id).collect::<Vec<_>>()
        );

        let before = current_time();
        type_keys(&mut app, &fixture, "a");
        type_keys(&mut app, &fixture, "new");
        press(&mut app, &fixture, KeyCode::Enter);

        let added = app.tasks.last().unwrap();
        assert!(original.iter().all(|(id, _)| *id != added.id));
        assert!(added.created_at >= before && added.created_at <= current_time());
    }

    fn saved_ids(fixture: &Fixture) -> Vec<Uuid> {
        fixture
            .storage
            .load()
            .unwrap()
            .iter()
            .map(|task| task.id)
            .collect()
    }

    #[test]
    fn counts_move_the_selection_by_that_many_tasks_and_are_consumed() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(30);

        for (keys, expected) in [("5j", 5), ("j", 6), ("12j", 18), ("10k", 8), ("k", 7)] {
            type_keys(&mut app, &fixture, keys);
            assert_eq!(app.list.selected(), Some(expected), "{keys}");
        }

        assert!(!app.can_undo());
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn counted_navigation_stops_at_the_ends_while_plain_motions_wrap() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(10);
        app.list.select(Some(7));

        for (keys, expected) in [
            ("5j", 9),
            ("3j", 9),
            ("j", 0),
            ("4k", 0),
            ("k", 9),
            ("2k", 7),
            ("99999999999999999999999999j", 9),
            ("99999999999999999999999999k", 0),
        ] {
            type_keys(&mut app, &fixture, keys);
            assert_eq!(app.list.selected(), Some(expected), "{keys}");
        }
    }

    #[test]
    fn arrow_and_control_aliases_accept_counts() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(10);

        for (count, key, expected) in [
            ('3', KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 3),
            ('2', KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), 1),
            (
                '4',
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
                5,
            ),
            (
                '2',
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
                3,
            ),
        ] {
            press(&mut app, &fixture, KeyCode::Char(count));
            press(&mut app, &fixture, key);

            assert_eq!(app.list.selected(), Some(expected), "{count} {key:?}");
        }
    }

    #[test]
    fn zero_continues_a_count_but_does_not_start_one() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(20);

        type_keys(&mut app, &fixture, "0j");
        assert_eq!(app.list.selected(), Some(1));

        type_keys(&mut app, &fixture, "10j");
        assert_eq!(app.list.selected(), Some(11));
    }

    #[test]
    fn any_other_input_discards_a_pending_count() {
        let fixture = Fixture::new();

        for discard in 0..5 {
            let mut app = app_with_tasks(10);
            press(&mut app, &fixture, KeyCode::Char('5'));

            match discard {
                0 => {
                    press(&mut app, &fixture, KeyCode::Esc);
                }
                1 => {
                    press(&mut app, &fixture, KeyCode::Char('w'));
                }
                2 => {
                    press(
                        &mut app,
                        &fixture,
                        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
                    );
                }
                3 => {
                    let scroll = MouseEvent {
                        kind: MouseEventKind::ScrollDown,
                        column: 0,
                        row: 0,
                        modifiers: KeyModifiers::NONE,
                    };
                    app.mouse(scroll, Rect::default(), &fixture.storage)
                        .unwrap();
                }
                _ => app.paste("text"),
            }

            press(&mut app, &fixture, KeyCode::Char('j'));

            assert_eq!(app.list.selected(), Some(1), "discard {discard}");
        }
    }

    #[test]
    fn digit_repeats_and_releases_do_not_change_a_pending_count() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(60);
        press(&mut app, &fixture, KeyCode::Char('5'));

        for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
            let digit = KeyEvent::new_with_kind(KeyCode::Char('2'), KeyModifiers::NONE, kind);
            press(&mut app, &fixture, digit);
        }

        press(&mut app, &fixture, KeyCode::Char('j'));

        assert_eq!(app.list.selected(), Some(5));
    }

    #[test]
    fn counted_reordering_moves_a_task_as_one_undoable_change() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(6);
        app.list.select(Some(1));
        let original = app.tasks.clone();

        type_keys(&mut app, &fixture, "3J");

        let order: Vec<_> = [0, 2, 3, 4, 1, 5]
            .map(|index| original[index].clone())
            .into();
        assert_eq!(app.tasks, order);
        assert_eq!(app.list.selected(), Some(4));
        assert_eq!(app.history.len(), 1);
        assert_eq!(fixture.storage.load().unwrap(), order);

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, original);
        assert_eq!(app.list.selected(), Some(1));

        type_keys(&mut app, &fixture, "2K");
        assert_eq!(app.tasks[0], original[1]);
        assert_eq!(app.list.selected(), Some(0));

        type_keys(&mut app, &fixture, "9J");
        assert_eq!(app.tasks[5], original[1]);
        assert_eq!(app.list.selected(), Some(5));

        press(&mut app, &fixture, KeyCode::Char('2'));
        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT),
        );
        assert_eq!(app.tasks[3], original[1]);
        assert_eq!(app.list.selected(), Some(3));

        press(&mut app, &fixture, KeyCode::Char('2'));
        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::SHIFT),
        );
        assert_eq!(app.tasks[5], original[1]);
        assert_eq!(app.history.len(), 4);
    }

    #[test]
    fn counted_reordering_at_the_ends_is_a_no_op() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(4);
        let original = app.tasks.clone();

        for (selected, keys) in [(3, "3J"), (0, "2K")] {
            app.list.select(Some(selected));
            type_keys(&mut app, &fixture, keys);

            assert_eq!(app.tasks, original);
            assert_eq!(app.list.selected(), Some(selected));
        }

        assert!(!app.can_undo());
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn failed_counted_reordering_preserves_state_and_discards_the_count() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(6);
        type_keys(&mut app, &fixture, "j");
        fixture.storage.save(&app.tasks).unwrap();
        let before = task_state(&app);
        block_saves(&fixture);

        press(&mut app, &fixture, KeyCode::Char('3'));
        let result = app.key(
            KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE),
            &fixture.storage,
        );

        assert!(result.is_err());
        assert_eq!(task_state(&app), before);

        restore_saves(&fixture);
        press(&mut app, &fixture, KeyCode::Char('J'));

        assert_eq!(app.list.selected(), Some(2));
    }

    #[test]
    fn digits_remain_text_in_input_modes_and_empty_lists_ignore_counts() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);

        for start in ['a', 'e', '/'] {
            press(&mut app, &fixture, KeyCode::Char(start));
            type_keys(&mut app, &fixture, "5j");

            let text = match &app.mode {
                Mode::Adding(input, _)
                | Mode::Editing { input, .. }
                | Mode::Searching { input, .. } => input.text().to_owned(),
                _ => panic!("expected an input mode"),
            };
            assert!(text.ends_with("5j"), "{start}");

            press(&mut app, &fixture, KeyCode::Esc);
            assert!(app.pending_count.is_none());
        }

        let mut empty = app_with_tasks(0);
        type_keys(&mut empty, &fixture, "5j3J");

        assert!(empty.tasks.is_empty());
        assert_eq!(empty.list.selected(), None);
        assert!(empty.pending_count.is_none());
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn repeated_navigation_wraps_without_modifying_tasks() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        let original = app.tasks.clone();

        for (code, selected) in [
            (KeyCode::Char('j'), 1),
            (KeyCode::Down, 2),
            (KeyCode::Char('j'), 0),
            (KeyCode::Char('k'), 2),
            (KeyCode::Up, 1),
        ] {
            repeat(&mut app, &fixture, code, KeyModifiers::NONE);
            assert_eq!(app.list.selected(), Some(selected));
        }

        assert_eq!(app.tasks, original);
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn repeated_actions_and_releases_preserve_tasks_selection_mode_and_undo() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        app.list.select(Some(1));
        let original = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char(' '));
        let changed = app.tasks.clone();

        for code in [
            KeyCode::Char(' '),
            KeyCode::Enter,
            KeyCode::Char('d'),
            KeyCode::Char('D'),
            KeyCode::Char('x'),
            KeyCode::Char('u'),
            KeyCode::Char('J'),
            KeyCode::Char('K'),
            KeyCode::Char('a'),
            KeyCode::Char('e'),
            KeyCode::Char('q'),
        ] {
            repeat(&mut app, &fixture, code, KeyModifiers::NONE);
        }

        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Up,
            KeyCode::Down,
        ] {
            repeat(&mut app, &fixture, code, KeyModifiers::SHIFT);
        }

        let release = KeyEvent {
            kind: KeyEventKind::Release,
            ..KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)
        };

        assert!(!app.key(release, &fixture.storage).unwrap());
        assert_eq!(app.tasks, changed);
        assert_eq!(app.list.selected(), Some(1));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(fixture.storage.load().unwrap(), changed);

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, original);
    }

    #[test]
    fn text_repeats_work_but_repeated_enter_cannot_confirm_or_toggle() {
        let fixture = Fixture::new();

        for start in [KeyCode::Char('a'), KeyCode::Char('e')] {
            let mut app = app_with_tasks(1);
            let original = app.tasks.clone();
            press(&mut app, &fixture, start);

            for character in "uqd日本語".chars() {
                repeat(
                    &mut app,
                    &fixture,
                    KeyCode::Char(character),
                    KeyModifiers::NONE,
                );
            }

            repeat(&mut app, &fixture, KeyCode::Backspace, KeyModifiers::NONE);
            repeat(&mut app, &fixture, KeyCode::Enter, KeyModifiers::NONE);
            repeat(&mut app, &fixture, KeyCode::Esc, KeyModifiers::NONE);

            match &app.mode {
                Mode::Adding(input, _) | Mode::Editing { input, .. } => {
                    assert!(input.text().ends_with("uqd日本"))
                }
                _ => panic!("repeat must not leave text input"),
            }

            assert_eq!(app.tasks, original);

            press(&mut app, &fixture, KeyCode::Enter);
            let confirmed = app.tasks.clone();
            repeat(&mut app, &fixture, KeyCode::Enter, KeyModifiers::NONE);

            assert_eq!(app.tasks, confirmed);
            assert_eq!(fixture.storage.load().unwrap(), confirmed);
            assert!(app.tasks.iter().all(|task| !task.completed));

            press(&mut app, &fixture, KeyCode::Char('u'));
            assert_eq!(app.tasks, original);
        }
    }

    #[test]
    fn clear_dialog_requires_a_pressed_enter() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(1);
        let original = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char('D'));
        repeat(&mut app, &fixture, KeyCode::Enter, KeyModifiers::NONE);
        repeat(&mut app, &fixture, KeyCode::Esc, KeyModifiers::NONE);

        assert_eq!(app.mode, Mode::ConfirmClear);
        assert_eq!(app.tasks, original);
        assert!(!fixture.task_path().exists());

        press(&mut app, &fixture, KeyCode::Enter);
        assert!(app.tasks.is_empty());
    }

    #[test]
    fn clearing_completed_preserves_order_and_selects_the_nearest_survivor() {
        for (completed, selected, expected_selection) in [
            (vec![1, 3], 0, Some(0)),
            (vec![1, 3], 1, Some(1)),
            (vec![1, 3], 2, Some(1)),
            (vec![1, 3], 3, Some(2)),
            (vec![1, 3], 4, Some(2)),
            (vec![0, 2, 4], 4, Some(1)),
            (vec![0, 1, 2, 3, 4], 2, None),
        ] {
            let fixture = Fixture::new();
            let mut app = app_with_tasks(5);

            for index in completed {
                app.tasks[index].completed = true;
            }

            app.list.select(Some(selected));
            let original = app.tasks.clone();

            let expected: Vec<_> = original
                .iter()
                .filter(|task| !task.completed)
                .cloned()
                .collect();

            press(&mut app, &fixture, KeyCode::Char('x'));
            assert_eq!(app.mode, Mode::Normal);
            assert_eq!(app.tasks, expected);
            assert_eq!(app.list.selected(), expected_selection);
            assert_eq!(fixture.storage.load().unwrap(), expected);

            press(&mut app, &fixture, KeyCode::Char('u'));
            assert_eq!(app.tasks, original);
            assert_eq!(app.list.selected(), Some(selected));
            assert_eq!(fixture.storage.load().unwrap(), original);
        }
    }

    #[test]
    fn clearing_without_completed_tasks_is_a_no_op_and_preserves_undo() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(0);
        press(&mut app, &fixture, KeyCode::Char('x'));

        assert!(!fixture.task_path().exists());
        assert!(app.history.is_empty());

        press(&mut app, &fixture, KeyCode::Char('a'));
        press(&mut app, &fixture, KeyCode::Char('x'));
        press(&mut app, &fixture, KeyCode::Enter);
        press(&mut app, &fixture, KeyCode::Char('x'));

        assert_eq!(app.tasks.len(), 1);
        assert_eq!(app.tasks[0].text, "x");
        assert_eq!(app.list.selected(), Some(0));

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert!(app.tasks.is_empty());
        assert_eq!(app.list.selected(), None);
        assert!(fixture.storage.load().unwrap().is_empty());
    }

    #[test]
    fn clear_completed_does_not_activate_in_input_or_confirmation_modes() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(1);
        app.tasks[0].completed = true;
        let original = app.tasks.clone();

        for mode in [
            Mode::Adding(TextInput::default(), AddPlacement::End),
            Mode::Editing {
                index: 0,
                input: "task 0".into(),
            },
            Mode::ConfirmClear,
        ] {
            app.mode = mode;
            press(&mut app, &fixture, KeyCode::Char('x'));
            assert_eq!(app.tasks, original);

            match &app.mode {
                Mode::Adding(input, _) | Mode::Editing { input, .. } => {
                    assert!(input.text().ends_with('x'))
                }
                mode => assert_eq!(*mode, Mode::ConfirmClear),
            }

            assert!(!fixture.task_path().exists());
        }
    }

    #[test]
    fn undo_restores_list_and_selection_for_every_modification() {
        use KeyCode::{Char, Enter};

        for (keys, label) in [
            (vec![Char('d')], "delete task"),
            (vec![Char('x')], "clear 1 completed task"),
            (vec![Char(' ')], "uncomplete task"),
            (vec![Char('J')], "move task"),
            (vec![Char('K')], "move task"),
            (vec![Char('D'), Enter], "clear 3 tasks"),
            (vec![Char('a'), Char('x'), Enter], "add task"),
            (vec![Char('e'), Char('x'), Enter], "edit task"),
            (vec![Char('y'), Char('p')], "paste task"),
        ] {
            let fixture = Fixture::new();
            let mut app = app_with_tasks(3);
            app.list.select(Some(1));
            app.tasks[1].completed = true;
            let original = app.tasks.clone();

            for key in keys {
                press(&mut app, &fixture, key);
            }

            assert_ne!(app.tasks, original);

            let modified = app.tasks.clone();
            app.navigate(true, None);
            let selected_before_undo = app.list.selected();
            press(&mut app, &fixture, Char('u'));

            assert_eq!(app.tasks, original);
            assert_eq!(app.list.selected(), Some(1));
            assert_eq!(fixture.storage.load().unwrap(), original);
            assert!(app.reveal_selection);
            assert!(app.history.is_empty());
            assert_eq!(notice(&app), Some(format!("Undid: {label}").as_str()));

            press(&mut app, &fixture, Char('u'));
            assert_eq!(app.tasks, original);
            assert_eq!(notice(&app), Some("Nothing to undo"));

            press(
                &mut app,
                &fixture,
                KeyEvent::new(Char('r'), KeyModifiers::CONTROL),
            );

            assert_eq!(app.tasks, modified);
            assert_eq!(app.list.selected(), selected_before_undo);
            assert_eq!(fixture.storage.load().unwrap(), modified);
            assert!(!app.can_redo());
            assert_eq!(notice(&app), Some(format!("Redid: {label}").as_str()));

            press(
                &mut app,
                &fixture,
                KeyEvent::new(Char('r'), KeyModifiers::CONTROL),
            );
            assert_eq!(app.tasks, modified);
            assert_eq!(notice(&app), Some("Nothing to redo"));
        }
    }

    #[test]
    fn notifications_explain_no_ops_without_changing_tasks() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(2);
        let original = app.tasks.clone();

        press(&mut app, &fixture, KeyCode::Char('p'));
        assert_eq!(notice(&app), Some("Nothing to paste"));

        press(&mut app, &fixture, KeyCode::Char('n'));
        assert_eq!(notice(&app), Some("No previous search"));

        app.last_search_query = Some("missing".into());
        press(&mut app, &fixture, KeyCode::Char('n'));
        assert_eq!(notice(&app), Some("No matches for \"missing\""));
        assert_eq!(app.mode, Mode::Normal);

        press(&mut app, &fixture, KeyCode::Char('x'));
        assert_eq!(notice(&app), Some("No completed tasks"));

        assert_eq!(app.tasks, original);
        assert!(!app.can_undo());
        assert!(!fixture.task_path().exists());

        let mut done = app_with_tasks(2);
        done.tasks.iter_mut().for_each(|task| task.completed = true);
        let original = done.tasks.clone();

        press(&mut done, &fixture, KeyCode::Char('m'));
        assert_eq!(notice(&done), Some("Completed tasks can't be carried"));

        press(&mut done, &fixture, KeyCode::Tab);
        assert_eq!(
            notice(&done),
            Some("No sections yet (start a task with \"Name: \")")
        );
        assert_eq!(done.tasks, original);
        assert_eq!(done.view, SectionView::All);
        assert!(!done.can_undo());
    }

    #[test]
    fn notifications_confirm_easy_to_miss_changes() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(5);

        press(&mut app, &fixture, KeyCode::Char('w'));
        assert_eq!(notice(&app), Some("Wrap off"));

        press(&mut app, &fixture, KeyCode::Char('w'));
        assert_eq!(notice(&app), Some("Wrap on"));

        press(&mut app, &fixture, KeyCode::Char('m'));
        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(notice(&app), Some("Undid: mark carryover"));

        press(&mut app, &fixture, KeyCode::Char('d'));
        assert_eq!(notice(&app), Some("Deleted task"));

        for (completed, message) in [
            (1, "Cleared 1 completed task"),
            (2, "Cleared 2 completed tasks"),
        ] {
            let mut app = app_with_tasks(3);
            app.tasks[..completed]
                .iter_mut()
                .for_each(|task| task.completed = true);

            press(&mut app, &fixture, KeyCode::Char('x'));
            assert_eq!(notice(&app), Some(message));
        }

        for (carried, message) in [
            (false, "Cleared 3 tasks"),
            (true, "Cleared 2 tasks, kept 1 carried"),
        ] {
            let mut app = app_with_tasks(3);
            app.tasks[0].carried = carried;

            press(&mut app, &fixture, KeyCode::Char('D'));
            press(&mut app, &fixture, KeyCode::Enter);
            assert_eq!(notice(&app), Some(message));
        }
    }

    #[test]
    fn failed_saves_never_confirm_a_change() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        app.tasks[0].completed = true;
        fixture.storage.save(&app.tasks).unwrap();
        block_saves(&fixture);

        for key in [KeyCode::Char('d'), KeyCode::Char('x')] {
            let result = app.key(KeyEvent::new(key, KeyModifiers::NONE), &fixture.storage);

            assert!(result.is_err());
            assert_eq!(notice(&app), None);
        }

        press(&mut app, &fixture, KeyCode::Char('D'));
        let result = app.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &fixture.storage,
        );

        assert!(result.is_err());
        assert_eq!(notice(&app), None);
    }

    #[test]
    fn undo_and_redo_each_keep_exactly_256_states_and_discard_older_history() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(0);

        for index in 0..258 {
            press(&mut app, &fixture, KeyCode::Char('a'));
            app.paste(&format!("task {index}"));
            press(&mut app, &fixture, KeyCode::Enter);

            assert_eq!(app.history.len(), (index + 1).min(HISTORY_LIMIT));
        }

        assert_eq!(app.history.len(), HISTORY_LIMIT);

        for remaining in (2..258).rev() {
            press(&mut app, &fixture, KeyCode::Char('u'));
            assert_eq!(app.tasks.len(), remaining);
            assert_eq!(
                app.tasks.last().unwrap().text,
                format!("task {}", remaining - 1)
            );
            assert_eq!(app.list.selected(), Some(remaining - 1));
            assert_eq!(fixture.storage.load().unwrap(), app.tasks);
        }

        assert!(app.history.is_empty());

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks.len(), 2);
        assert_eq!(app.redo_history.len(), HISTORY_LIMIT);

        for remaining in 3..=258 {
            press(
                &mut app,
                &fixture,
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
            );

            assert_eq!(app.tasks.len(), remaining);
            assert_eq!(app.list.selected(), Some(remaining - 1));
            assert_eq!(fixture.storage.load().unwrap(), app.tasks);
        }

        assert!(app.redo_history.is_empty());

        app.redo(&fixture.storage).unwrap();
        assert_eq!(app.tasks.len(), 258);

        let restarted = App::new(fixture.storage.load().unwrap());
        assert!(!restarted.can_undo() && !restarted.can_redo());
    }

    #[test]
    fn redo_is_preserved_by_no_ops_and_failed_saves_but_cleared_by_a_new_branch() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(1);

        for _ in 0..3 {
            app.toggle(0, &fixture.storage).unwrap();
        }

        app.undo(&fixture.storage).unwrap();
        let original = app.tasks.clone();
        let undo = app.history.clone();
        let redo = app.redo_history.clone();

        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Char('J'),
            KeyCode::Char('K'),
            KeyCode::Char('x'),
            KeyCode::Char('a'),
            KeyCode::Enter,
            KeyCode::Char('e'),
            KeyCode::Enter,
            KeyCode::Char('D'),
            KeyCode::Esc,
            KeyCode::Char('/'),
        ] {
            press(&mut app, &fixture, code);
        }

        app.paste("task");
        press(&mut app, &fixture, KeyCode::Enter);
        press(&mut app, &fixture, KeyCode::Char('n'));
        press(&mut app, &fixture, KeyCode::Esc);

        assert_eq!(app.history, undo);
        assert_eq!(app.redo_history, redo);

        block_saves(&fixture);

        for operation in 0..3 {
            let result = match operation {
                0 => app.toggle(0, &fixture.storage),
                1 => app.undo(&fixture.storage),
                _ => app.redo(&fixture.storage),
            };

            assert!(result.is_err());
            assert_eq!(app.tasks, original);
            assert_eq!(app.list.selected(), Some(0));
            assert_eq!(app.history, undo);
            assert_eq!(app.redo_history, redo);
        }

        restore_saves(&fixture);
        app.redo(&fixture.storage).unwrap();
        assert!(app.tasks[0].completed);

        app.undo(&fixture.storage).unwrap();
        app.mode = Mode::Adding("new branch".into(), AddPlacement::End);
        press(&mut app, &fixture, KeyCode::Enter);
        let branched = app.tasks.clone();

        assert!(!app.can_redo());

        app.redo(&fixture.storage).unwrap();
        assert_eq!(app.tasks, branched);

        app.undo(&fixture.storage).unwrap();
        assert_eq!(app.tasks, original);
    }

    #[test]
    fn redo_shortcut_is_normal_mode_only_and_ignores_repeats_and_releases() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(1);
        app.toggle(0, &fixture.storage).unwrap();
        app.undo(&fixture.storage).unwrap();
        let original = app.tasks.clone();

        for mode in [
            Mode::Normal,
            Mode::Adding("task".into(), AddPlacement::End),
            Mode::Editing {
                index: 0,
                input: "task".into(),
            },
            Mode::Searching {
                input: "task".into(),
                original: Some(0),
            },
            Mode::SearchResults {
                input: "task".into(),
                original: Some(0),
            },
            Mode::ConfirmClear,
        ] {
            app.mode = mode;
            let before = format!("{:?}", app.mode);

            for kind in [
                KeyEventKind::Press,
                KeyEventKind::Repeat,
                KeyEventKind::Release,
            ] {
                if app.mode == Mode::Normal && kind == KeyEventKind::Press {
                    continue;
                }

                press(
                    &mut app,
                    &fixture,
                    KeyEvent::new_with_kind(KeyCode::Char('r'), KeyModifiers::CONTROL, kind),
                );

                assert_eq!(app.tasks, original);
                assert_eq!(format!("{:?}", app.mode), before);
                assert_eq!(app.redo_history.len(), 1);
            }
        }

        app.mode = Mode::Normal;
        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
        );

        assert!(app.tasks[0].completed);

        let restarted = App::new(fixture.storage.load().unwrap());
        assert!(!restarted.can_undo() && !restarted.can_redo());

        app.undo(&fixture.storage).unwrap();
        let restarted = App::new(fixture.storage.load().unwrap());
        assert!(!restarted.can_undo() && !restarted.can_redo());
    }

    #[test]
    fn new_mutations_after_undo_record_the_restored_state() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(0);

        for text in ["first", "second"] {
            press(&mut app, &fixture, KeyCode::Char('a'));
            app.paste(text);
            press(&mut app, &fixture, KeyCode::Enter);
        }

        press(&mut app, &fixture, KeyCode::Char('u'));
        press(&mut app, &fixture, KeyCode::Char('e'));
        app.paste(" edited");
        press(&mut app, &fixture, KeyCode::Enter);

        assert_eq!(app.history.len(), 2);

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks[0].text, "first");

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert!(app.tasks.is_empty());
        assert_eq!(app.list.selected(), None);
        assert!(app.reveal_selection);
        assert!(app.history.is_empty());
        assert!(fixture.storage.load().unwrap().is_empty());
    }

    #[test]
    fn mixed_mutations_undo_in_order_and_reveal_restored_wrapped_selection() {
        use KeyCode::{Char, Enter};
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        app.tasks[0].text = "wrapped task ".repeat(30).trim_end().to_owned();
        let mut snapshots = Vec::new();

        for keys in [
            vec![Char('a'), Char('n'), Enter],
            vec![Char('e'), Char('!'), Enter],
            vec![Char(' ')],
            vec![Char('K')],
            vec![Char('x')],
            vec![Char('d')],
            vec![Char('D'), Enter],
        ] {
            snapshots.push((app.tasks.clone(), app.list.selected()));

            for key in keys {
                press(&mut app, &fixture, key);
            }

            assert_eq!(app.history.len(), snapshots.len());
        }

        assert!(app.tasks.is_empty());

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(18, 6)).unwrap();

        for (tasks, selected) in snapshots.into_iter().rev() {
            app.scroll = usize::MAX;
            press(&mut app, &fixture, Char('u'));
            assert_eq!(app.tasks, tasks);
            assert_eq!(app.list.selected(), selected);
            assert!(app.reveal_selection);
            assert_eq!(fixture.storage.load().unwrap(), tasks);

            let mut rows = Rect::default();
            terminal
                .draw(|frame| rows = crate::ui::draw(frame, &mut app))
                .unwrap();

            if let Some(selected) = selected {
                let end = (app.scroll + usize::from(rows.height)).min(app.row_tasks.len());
                assert!(app.row_tasks[app.scroll..end].contains(&Some(selected)));
            }
        }

        assert!(app.history.is_empty());
    }

    #[test]
    fn undo_walks_back_through_history_and_a_new_app_starts_empty() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(2);
        let original = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char('d'));
        let preceding = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char('d'));

        assert!(app.tasks.is_empty());

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, preceding);
        assert_eq!(app.list.selected(), Some(0));

        let mut restarted = App::new(fixture.storage.load().unwrap());
        press(&mut restarted, &fixture, KeyCode::Char('u'));
        assert_eq!(restarted.tasks, preceding);
        assert!(restarted.history.is_empty());

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, original);
        assert_eq!(fixture.storage.load().unwrap(), original);
        assert!(app.history.is_empty());

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, original);
    }

    #[test]
    fn no_ops_and_cancelled_input_keep_undo_and_u_is_text_in_input_modes() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(1);
        let original = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char(' '));

        // Boundary movement, unchanged editing, blank adding, and cancelled clear.
        for code in [
            KeyCode::Char('J'),
            KeyCode::Char('e'),
            KeyCode::Enter,
            KeyCode::Char('a'),
            KeyCode::Enter,
            KeyCode::Char('D'),
            KeyCode::Char('u'),
            KeyCode::Esc,
        ] {
            press(&mut app, &fixture, code);
        }

        for code in [KeyCode::Char('a'), KeyCode::Char('e')] {
            press(&mut app, &fixture, code);
            press(&mut app, &fixture, KeyCode::Char('u'));

            match &app.mode {
                Mode::Adding(input, _) | Mode::Editing { input, .. } => {
                    assert!(input.text().ends_with('u'))
                }
                _ => panic!("expected text input"),
            }

            press(&mut app, &fixture, KeyCode::Esc);
        }

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, original);

        let mut empty = app_with_tasks(0);
        press(&mut empty, &fixture, KeyCode::Char('u'));
        assert!(empty.tasks.is_empty());
    }

    #[test]
    fn failed_modification_or_undo_preserves_the_undo_buffer() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(2);
        let original = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char(' '));
        let intermediate = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char('J'));
        let modified = app.tasks.clone();
        let history = app.history.clone();
        block_saves(&fixture);

        for code in [KeyCode::Char('d'), KeyCode::Char('x'), KeyCode::Char('u')] {
            assert!(
                app.key(KeyEvent::new(code, KeyModifiers::NONE), &fixture.storage)
                    .is_err()
            );
            assert_eq!(app.tasks, modified);
            assert_eq!(app.list.selected(), Some(1));
            assert_eq!(app.history, history);
        }

        restore_saves(&fixture);
        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, intermediate);
        assert_eq!(app.history.len(), 1);

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, original);
        assert_eq!(fixture.storage.load().unwrap(), original);
        assert!(app.history.is_empty());
    }

    #[test]
    fn editing_prefills_text_and_saves_without_changing_order_or_completion() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        app.list.select(Some(1));
        app.tasks[1].completed = true;
        press(&mut app, &fixture, KeyCode::Char('e'));

        assert_eq!(
            app.mode,
            Mode::Editing {
                index: 1,
                input: "task 1".into()
            }
        );

        press(&mut app, &fixture, KeyCode::Backspace);

        for character in "日本語🪷".chars() {
            press(&mut app, &fixture, KeyCode::Char(character));
        }

        press(&mut app, &fixture, KeyCode::Backspace);

        // List shortcuts remain text, and navigation keys cannot change the edit target.
        for character in " qjadDeK ".chars() {
            assert!(!press(&mut app, &fixture, KeyCode::Char(character)));
        }

        press(&mut app, &fixture, KeyCode::Down);
        press(&mut app, &fixture, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.list.selected(), Some(1));
        assert_eq!(app.tasks[0].text, "task 0");
        assert_eq!(app.tasks[1].text, "task 日本語 qjadDeK");
        assert!(app.tasks[1].completed);
        assert_eq!(app.tasks[2].text, "task 2");
        assert_eq!(fixture.storage.load().unwrap(), app.tasks);
    }

    #[test]
    fn cancelled_blank_and_unchanged_edits_do_not_write() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(1);
        let original = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char('e'));
        press(&mut app, &fixture, KeyCode::Char('x'));
        press(&mut app, &fixture, KeyCode::Esc);

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.tasks, original);

        press(&mut app, &fixture, KeyCode::Char('e'));
        press(&mut app, &fixture, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Normal);

        press(&mut app, &fixture, KeyCode::Char('e'));

        for _ in 0..original[0].text.chars().count() {
            press(&mut app, &fixture, KeyCode::Backspace);
        }

        press(&mut app, &fixture, KeyCode::Char(' '));
        press(&mut app, &fixture, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.tasks, original);
        assert_eq!(app.list.selected(), Some(0));
        assert!(!fixture.task_path().exists());

        let mut empty = app_with_tasks(0);
        press(&mut empty, &fixture, KeyCode::Char('e'));
        assert_eq!(empty.mode, Mode::Normal);
    }

    #[test]
    fn failed_edit_save_preserves_original_and_input() {
        let fixture = Fixture::new();
        std::fs::write(fixture.directory.join("data"), "blocking file").unwrap();
        let mut app = app_with_tasks(1);
        let original = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char('e'));
        press(&mut app, &fixture, KeyCode::Char('x'));

        assert!(
            app.key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &fixture.storage
            )
            .is_err()
        );
        assert_eq!(app.tasks, original);
        assert_eq!(app.list.selected(), Some(0));
        assert_eq!(
            app.mode,
            Mode::Editing {
                index: 0,
                input: "task 0x".into()
            }
        );
    }

    #[test]
    fn move_bindings_follow_the_task_and_save_order_and_completion() {
        for (down, up, modifiers) in [
            (KeyCode::Char('J'), KeyCode::Char('K'), KeyModifiers::NONE),
            (KeyCode::Char('J'), KeyCode::Char('K'), KeyModifiers::SHIFT),
            (KeyCode::Char('j'), KeyCode::Char('k'), KeyModifiers::SHIFT),
            (KeyCode::Down, KeyCode::Up, KeyModifiers::SHIFT),
        ] {
            let fixture = Fixture::new();
            let mut app = app_with_tasks(3);
            app.tasks[1].completed = true;
            app.list.select(Some(1));
            let original = app.tasks.clone();
            press(&mut app, &fixture, KeyEvent::new(down, modifiers));

            assert_eq!(app.list.selected(), Some(2));
            assert_eq!(
                app.tasks,
                vec![
                    original[0].clone(),
                    original[2].clone(),
                    original[1].clone()
                ]
            );
            assert_eq!(fixture.storage.load().unwrap(), app.tasks);

            press(&mut app, &fixture, KeyEvent::new(up, modifiers));
            assert_eq!(app.list.selected(), Some(1));
            assert_eq!(app.tasks, original);
            assert_eq!(fixture.storage.load().unwrap(), original);
        }
    }

    #[test]
    fn moves_at_boundaries_do_nothing_without_saving() {
        let fixture = Fixture::new();

        for count in [0, 1, 3] {
            let mut app = app_with_tasks(count);
            let original = app.tasks.clone();
            let first = app.list.selected();
            press(&mut app, &fixture, KeyCode::Char('K'));

            assert_eq!(app.list.selected(), first);

            app.list.select(count.checked_sub(1));
            let last = app.list.selected();
            press(&mut app, &fixture, KeyCode::Char('J'));

            assert_eq!(app.list.selected(), last);
            assert_eq!(app.tasks, original);
            assert!(!fixture.task_path().exists());
        }
    }

    #[test]
    fn move_shortcuts_are_isolated_from_input_and_confirmation() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        app.list.select(Some(1));
        let original = app.tasks.clone();

        for mode in [
            Mode::Adding(TextInput::default(), AddPlacement::End),
            Mode::ConfirmClear,
        ] {
            app.mode = mode;

            for code in [
                KeyCode::Char('J'),
                KeyCode::Char('K'),
                KeyCode::Down,
                KeyCode::Up,
            ] {
                press(&mut app, &fixture, KeyEvent::new(code, KeyModifiers::SHIFT));
            }

            assert_eq!(app.tasks, original);
            assert_eq!(app.list.selected(), Some(1));
            assert!(!fixture.task_path().exists());

            if let Mode::Adding(input, _) = &app.mode {
                assert_eq!(input.text(), "JK");
            }
        }
    }

    #[test]
    fn failed_move_save_preserves_order_and_selection() {
        let fixture = Fixture::new();
        std::fs::write(fixture.directory.join("data"), "blocking file").unwrap();
        let mut app = app_with_tasks(3);
        app.list.select(Some(1));
        let original = app.tasks.clone();

        for code in [KeyCode::Char('J'), KeyCode::Char('K')] {
            assert!(
                app.key(KeyEvent::new(code, KeyModifiers::NONE), &fixture.storage)
                    .is_err()
            );
            assert_eq!(app.tasks, original);
            assert_eq!(app.list.selected(), Some(1));
        }
    }

    #[test]
    fn navigation_wraps_and_empty_operations_are_safe() {
        let fixture = Fixture::new();
        let mut empty = app_with_tasks(0);

        for code in [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Enter,
            KeyCode::Char(' '),
            KeyCode::Char('d'),
            KeyCode::Char('D'),
            KeyCode::Esc,
        ] {
            assert!(!press(&mut empty, &fixture, code));
        }

        assert_eq!(empty.list.selected(), None);
        assert_eq!(empty.mode, Mode::Normal);
        assert!(!fixture.task_path().exists());

        let mut app = app_with_tasks(1000);
        press(&mut app, &fixture, KeyCode::Char('k'));
        assert_eq!(app.list.selected(), Some(999));

        press(&mut app, &fixture, KeyCode::Down);
        assert_eq!(app.list.selected(), Some(0));

        press(&mut app, &fixture, KeyCode::Char('j'));
        press(&mut app, &fixture, KeyCode::Up);
        assert_eq!(app.list.selected(), Some(0));
        assert!(!fixture.task_path().exists());
        assert!(press(&mut app, &fixture, KeyCode::Char('q')));
    }

    #[test]
    fn positional_insertions_preserve_order_selection_and_history() {
        for (count, selected, positions) in [
            (0, None, [0, 0, 0]),
            (1, Some(0), [1, 1, 0]),
            (3, Some(0), [3, 1, 0]),
            (3, Some(1), [3, 2, 1]),
            (3, Some(2), [3, 3, 2]),
            (3, None, [3, 3, 3]),
        ] {
            for (key, expected_index) in ['a', 'o', 'O'].into_iter().zip(positions) {
                let fixture = Fixture::new();
                let mut app = app_with_tasks(count);

                for (index, task) in app.tasks.iter_mut().enumerate() {
                    task.completed = index == 0;
                }

                app.list.select(selected);
                let original = app.tasks.clone();
                let mut expected = original.clone();
                expected.insert(expected_index, Task::new("New task"));
                press(&mut app, &fixture, KeyCode::Char(key));
                app.paste("New task");
                press(&mut app, &fixture, KeyCode::Enter);

                // The new task gets its own identity; everything else must match exactly.
                let created = &app.tasks[expected_index];
                assert!(original.iter().all(|task| task.id != created.id));
                expected[expected_index].id = created.id;
                expected[expected_index].created_at = created.created_at;

                assert_eq!(app.tasks, expected);
                assert_eq!(app.list.selected(), Some(expected_index));
                assert_eq!(fixture.storage.load().unwrap(), expected);
                assert_eq!(app.history.len(), 1);
                assert!(app.reveal_selection);

                app.keybinds = false;
                let mut terminal =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(18, 3)).unwrap();
                terminal
                    .draw(|frame| {
                        crate::ui::draw(frame, &mut app);
                    })
                    .unwrap();

                assert_eq!(app.row_tasks[app.scroll], Some(expected_index));

                app.undo(&fixture.storage).unwrap();
                assert_eq!(app.tasks, original);
                assert_eq!(app.list.selected(), selected);

                app.redo(&fixture.storage).unwrap();
                assert_eq!(app.tasks, expected);
                assert_eq!(app.list.selected(), Some(expected_index));

                app.undo(&fixture.storage).unwrap();
                press(&mut app, &fixture, KeyCode::Char(key));
                app.paste("New branch");
                press(&mut app, &fixture, KeyCode::Enter);

                assert!(!app.can_redo());
                assert_eq!(app.history.len(), 1);
            }
        }
    }

    #[test]
    fn positional_input_cancellation_and_save_failures_preserve_both_histories() {
        for key in ['a', 'o', 'O'] {
            let fixture = Fixture::new();
            let mut app = app_with_tasks(2);
            app.toggle(0, &fixture.storage).unwrap();
            app.toggle(1, &fixture.storage).unwrap();
            app.undo(&fixture.storage).unwrap();
            let original = task_state(&app);

            for ending in [KeyCode::Esc, KeyCode::Enter] {
                press(&mut app, &fixture, KeyCode::Char(key));
                app.paste(if ending == KeyCode::Esc {
                    "canceled task"
                } else {
                    " "
                });
                press(&mut app, &fixture, ending);

                assert_eq!(task_state(&app), original);
                assert_eq!(app.mode, Mode::Normal);
            }

            for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
                press(
                    &mut app,
                    &fixture,
                    KeyEvent::new_with_kind(KeyCode::Char(key), KeyModifiers::NONE, kind),
                );

                assert_eq!(app.mode, Mode::Normal);
            }

            press(&mut app, &fixture, KeyCode::Char(key));
            app.paste("new task");
            block_saves(&fixture);

            assert!(
                app.key(
                    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                    &fixture.storage
                )
                .is_err()
            );
            assert_eq!(task_state(&app), original);
            assert!(matches!(app.mode, Mode::Adding(_, _)));

            restore_saves(&fixture);
            assert_eq!(fixture.storage.load().unwrap(), app.tasks);
        }
    }

    #[test]
    fn input_is_unicode_safe_trimmed_and_isolates_shortcuts() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(1);
        press(&mut app, &fixture, KeyCode::Char('a'));

        for character in "  qjk dD 日本語🪷".chars() {
            assert!(!press(&mut app, &fixture, KeyCode::Char(character)));
        }

        press(&mut app, &fixture, KeyCode::Backspace);
        press(&mut app, &fixture, KeyCode::Char(' '));
        press(&mut app, &fixture, KeyCode::Enter);

        assert_eq!(app.tasks[1].text, "qjk dD 日本語");
        assert_eq!(app.list.selected(), Some(1));
        assert_eq!(fixture.storage.load().unwrap(), app.tasks);

        for ending in [KeyCode::Esc, KeyCode::Enter] {
            press(&mut app, &fixture, KeyCode::Char('a'));
            press(&mut app, &fixture, KeyCode::Char(' '));
            press(&mut app, &fixture, ending);

            assert_eq!(app.mode, Mode::Normal);
            assert_eq!(app.tasks.len(), 2);
        }
    }

    #[test]
    fn toggles_preserve_selection_and_order_and_persist_immediately() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        app.list.select(Some(1));

        for code in [KeyCode::Enter, KeyCode::Char(' ')] {
            press(&mut app, &fixture, code);
            assert_eq!(app.list.selected(), Some(1));
            assert_eq!(fixture.storage.load().unwrap(), app.tasks);
            assert_eq!(app.tasks[1].text, "task 1");
        }

        assert!(!app.tasks[1].completed);
    }

    #[test]
    fn delete_selects_next_then_previous_then_none() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(4);

        for (selected, expected, next) in [
            (0, vec!["task 1", "task 2", "task 3"], Some(0)),
            (1, vec!["task 1", "task 3"], Some(1)),
            (1, vec!["task 1"], Some(0)),
            (0, vec![], None),
        ] {
            app.list.select(Some(selected));
            press(&mut app, &fixture, KeyCode::Char('d'));
            assert_eq!(
                app.tasks
                    .iter()
                    .map(|task| task.text.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(app.list.selected(), next);
            assert_eq!(fixture.storage.load().unwrap(), app.tasks);
        }
    }

    #[test]
    fn only_enter_confirms_clear() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(2);
        app.tasks[1].completed = true;
        press(&mut app, &fixture, KeyCode::Char('D'));

        for code in [
            KeyCode::Char('y'),
            KeyCode::Char(' '),
            KeyCode::Char('d'),
            KeyCode::Char('D'),
            KeyCode::Char('q'),
            KeyCode::Down,
        ] {
            assert!(!press(&mut app, &fixture, code));
            assert_eq!(app.tasks.len(), 2);
            assert_eq!(app.mode, Mode::ConfirmClear);
        }

        press(&mut app, &fixture, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);
        assert!(!fixture.task_path().exists());

        press(&mut app, &fixture, KeyCode::Char('D'));
        press(&mut app, &fixture, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.list.selected(), None);
        assert!(fixture.storage.load().unwrap().is_empty());
    }

    #[test]
    fn mouse_toggle_undo_restores_the_selection_before_the_click() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        app.row_tasks = vec![Some(0), Some(1), Some(2)];
        let original = app.tasks.clone();

        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 3,
            modifiers: KeyModifiers::NONE,
        };

        app.mouse(click, Rect::new(1, 1, 30, 3), &fixture.storage)
            .unwrap();
        assert_eq!(app.list.selected(), Some(2));
        assert!(app.tasks[2].completed);
        assert_eq!(fixture.storage.load().unwrap(), app.tasks);

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, original);
        assert_eq!(app.list.selected(), Some(0));
        assert_eq!(fixture.storage.load().unwrap(), original);
        assert!(app.history.is_empty());
    }

    #[test]
    fn mouse_toggle_failure_preserves_selection_and_existing_undo() {
        for has_previous in [false, true] {
            let fixture = Fixture::new();
            let mut app = app_with_tasks(3);
            app.row_tasks = vec![Some(0), Some(1), Some(2)];
            let original = app.tasks.clone();

            if has_previous {
                press(&mut app, &fixture, KeyCode::Char(' '));
            }

            press(&mut app, &fixture, KeyCode::Char('j'));
            let before_click = app.tasks.clone();
            let data = fixture.directory.join("data");
            let backup = fixture.directory.join("backup");

            if has_previous {
                std::fs::rename(&data, &backup).unwrap();
            }

            std::fs::write(&data, "blocking file").unwrap();

            let click = MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 1,
                row: 3,
                modifiers: KeyModifiers::NONE,
            };

            assert!(
                app.mouse(click, Rect::new(1, 1, 30, 3), &fixture.storage)
                    .is_err()
            );
            assert_eq!(app.tasks, before_click);
            assert_eq!(app.list.selected(), Some(1));
            assert_eq!(!app.history.is_empty(), has_previous);

            if let Some(previous) = app.history.back() {
                assert_eq!(previous.tasks, original);
                assert_eq!(previous.selected, Some(0));
            }

            std::fs::remove_file(&data).unwrap();

            if has_previous {
                std::fs::rename(&backup, &data).unwrap();
            }

            press(&mut app, &fixture, KeyCode::Char('u'));
            assert_eq!(app.tasks, original);
            assert_eq!(app.list.selected(), Some(if has_previous { 0 } else { 1 }));
        }
    }

    #[test]
    fn mouse_uses_visible_rows_and_ignores_borders_and_modal_input() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(10);
        app.scroll = 5;
        app.row_tasks = (0..10).map(Some).collect();
        let rows = Rect::new(1, 1, 30, 3);

        let click = |column, row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };

        app.mouse(click(1, 2), rows, &fixture.storage).unwrap();
        assert_eq!(app.list.selected(), Some(6));
        assert!(app.tasks[6].completed);
        assert_eq!(fixture.storage.load().unwrap(), app.tasks);

        for (column, row) in [(0, 1), (31, 1), (1, 0), (1, 4)] {
            app.mouse(click(column, row), rows, &fixture.storage)
                .unwrap();
            assert_eq!(app.list.selected(), Some(6));
        }

        app.mode = Mode::Adding(TextInput::default(), AddPlacement::End);
        app.mouse(click(1, 2), rows, &fixture.storage).unwrap();
        assert!(app.tasks[6].completed);

        app.mode = Mode::Normal;
        app.list.select(Some(9));
        app.scroll = 7;
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                ..click(1, 1)
            },
            rows,
            &fixture.storage,
        )
        .unwrap();

        assert_eq!(app.list.selected(), Some(9));

        app.mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                ..click(1, 1)
            },
            rows,
            &fixture.storage,
        )
        .unwrap();

        assert_eq!(app.scroll, 6);
        assert_eq!(app.list.selected(), Some(6));
    }

    #[test]
    fn failed_save_does_not_accept_a_mutation() {
        let fixture = Fixture::new();
        std::fs::write(fixture.directory.join("data"), "blocking file").unwrap();
        let mut app = app_with_tasks(1);
        let original = app.tasks.clone();

        assert!(
            app.key(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                &fixture.storage
            )
            .is_err()
        );
        assert_eq!(app.tasks, original);
        assert_eq!(app.list.selected(), Some(0));
    }
}
