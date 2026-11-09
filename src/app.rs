use std::collections::VecDeque;
use std::time::{Duration, Instant};

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{layout::Rect, widgets::ListState};

use crate::config::{BorderStyle, MarkerStyle};
use crate::input::TextInput;
use crate::model::{
    SectionView, Task, TaskStatus, invalid_section_name, section_body, section_tabs_of,
    settled_section_order, settled_view, single_line_character, split_section, task_prefix,
    view_shows,
};
use crate::status_message::StatusMessage;
use crate::storage::{Result, Storage};

const HISTORY_LIMIT: usize = 256;

// The name the section picker matches [-] by.
const UNSECTIONED_NAME: &str = "unsectioned";

// A count or `s` prefix is forgotten this long after its last key, so a stray digit typed
// now cannot turn a much later `j` into a jump.
const PREFIX_TIMEOUT: Duration = Duration::from_secs(2);

fn task_noun(count: usize) -> &'static str {
    if count == 1 { "task" } else { "tasks" }
}

// The text an add or edit input submits as a task: trimmed, and None when it is blank or a
// chosen section with nothing after it.
fn submitted_task_text(input: &TextInput) -> Option<String> {
    let text = input.text().trim();
    let bare_section = task_prefix(input.text()).is_some() && section_body(input.text()).is_empty();

    (!text.is_empty() && !bare_section).then(|| text.to_owned())
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
    // The read-only keyboard reference opened with `?`. Drawing keeps `scroll` within
    // `max_scroll` (the offset that shows the last line) and records `page`, the content rows
    // that fit, so the scroll keys never depend on the terminal size directly.
    Help {
        scroll: usize,
        max_scroll: usize,
        page: usize,
    },
}

impl Mode {
    // The single-line input that each typing mode edits.
    fn text_input_mut(&mut self) -> Option<&mut TextInput> {
        match self {
            Mode::Adding(input, _)
            | Mode::Editing { input, .. }
            | Mode::Searching { input, .. }
            | Mode::RenamingSection { input }
            | Mode::FuzzySection { input, .. } => Some(input),
            Mode::Normal | Mode::ConfirmClear | Mode::Help { .. } => None,
        }
    }
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
    view: SectionView,
    // The view the change from this snapshot left behind. Undo and redo return to `view`
    // only while the user is still there, so a later manual tab switch is kept.
    left_view: SectionView,
    // Describes the change from this snapshot to the next state, e.g. "delete task".
    label: String,
}

// The navigation that tasks.json holds, so a clean exit only writes when the selection or tab
// has changed since, or when the file still uses an older schema.
struct Persisted {
    selected: Option<usize>,
    view: SectionView,
    needs_migration: bool,
}

impl Persisted {
    // Write a complete document, and only once that succeeds, record what it holds.
    fn save(
        &mut self,
        storage: &Storage,
        tasks: &[Task],
        section_order: &[String],
        selected: Option<usize>,
        view: &SectionView,
    ) -> Result<()> {
        storage.save_view(tasks, section_order, selected, view.saved_key())?;

        self.selected = crate::storage::valid_selection(tasks, selected);
        self.view = view.clone();
        self.needs_migration = false;

        Ok(())
    }
}

pub struct App {
    pub tasks: Vec<Task>,
    // Lowercase keys of the named section tabs in display order; see `settled_section_order`.
    pub section_order: Vec<String>,
    pub list: ListState,
    pub mode: Mode,
    pub wrap: bool,
    pub status_messages: bool,
    pub status_message_duration: Duration,
    pub show_help_hint: bool,
    pub border_style: BorderStyle,
    pub marker_style: MarkerStyle,
    pub styles: crate::styles::Styles,
    pub theme: crate::themes::BuiltinTheme,
    pub style_overrides: crate::config::StyleOverrides,
    pub cli_accent: Option<ratatui::style::Color>,
    pub status_message: Option<StatusMessage>,
    pub view: SectionView,

    // Layout from the last draw, which `ui::draw` writes so that the next key or click acts on
    // what is on screen.
    //
    // `scroll` is the first task-list row shown. Input may move it; drawing clamps it, and
    // scrolls to the selection or inline input when that is off screen or `reveal_selection`
    // asks, then clears that request.
    pub scroll: usize,
    pub reveal_selection: bool,
    // The task on each list row, for paging and clicks; an inline add row has none.
    pub row_tasks: Vec<Option<usize>>,
    // The list rows that fit, for paging.
    pub viewport_height: u16,
    // Clickable tab regions.
    pub tab_targets: Vec<(Rect, SectionView)>,
    // The first visible tab when they overflow; kept between draws so tabs don't slide.
    pub tab_scroll: usize,

    // A Vim-style count typed before a motion, with when its last digit was typed; transient
    // and never saved.
    pending_count: Option<(usize, Instant)>,
    // Set by `s` / Alt+S, with when it was typed: the next key picks a tab by number.
    // Transient and never saved.
    pending_section_jump: Option<Instant>,
    completion: Option<Completion>,
    task_register: Option<String>,
    history: VecDeque<UndoState>,
    redo_history: VecDeque<UndoState>,
    persisted: Persisted,
}

impl App {
    pub fn new(tasks: Vec<Task>) -> Self {
        let selected = (!tasks.is_empty()).then_some(0);
        // Without a saved order, tabs start alphabetically, as they were before orders were
        // saved.
        let mut section_order = settled_section_order(&[], &tasks);
        section_order.sort();

        Self {
            tasks,
            section_order,
            list: ListState::default().with_selected(selected),
            mode: Mode::Normal,
            wrap: true,
            status_messages: true,
            status_message_duration: crate::status_message::DEFAULT_DURATION,
            show_help_hint: true,
            border_style: BorderStyle::default(),
            marker_style: MarkerStyle::default(),
            styles: crate::styles::Styles::default(),
            theme: crate::themes::BuiltinTheme::default(),
            style_overrides: crate::config::StyleOverrides::default(),
            cli_accent: None,
            status_message: None,
            view: SectionView::All,
            scroll: 0,
            reveal_selection: true,
            row_tasks: Vec::new(),
            viewport_height: 0,
            tab_targets: Vec::new(),
            tab_scroll: 0,
            pending_count: None,
            pending_section_jump: None,
            completion: None,
            task_register: None,
            history: VecDeque::new(),
            redo_history: VecDeque::new(),
            persisted: Persisted {
                selected,
                view: SectionView::All,
                needs_migration: false,
            },
        }
    }

    pub fn from_document(document: crate::storage::TaskDocument) -> Self {
        let mut app = Self::new(document.tasks);
        app.section_order = document.section_order;
        app.list.select(document.selected);
        app.view = SectionView::from_saved_key(document.section);
        app.persisted = Persisted {
            selected: document.selected,
            view: app.view.clone(),
            needs_migration: document.needs_migration,
        };

        if !app.section_tabs().iter().any(|(view, _)| *view == app.view) {
            app.view = SectionView::All;
        }

        app.settle_view();

        app
    }

    pub fn save_selection(&mut self, storage: &Storage) -> Result<()> {
        // Temporary search and section-picker focus is never saved.
        let (selected, view) = match &self.mode {
            Mode::Searching { original, .. } => (*original, self.view.clone()),
            Mode::FuzzySection { original, .. } => (original.selected, original.view.clone()),
            _ => (self.list.selected(), self.view.clone()),
        };

        let selected = crate::storage::valid_selection(&self.tasks, selected);

        if self.persisted.needs_migration
            || selected != self.persisted.selected
            || view != self.persisted.view
        {
            self.persisted
                .save(storage, &self.tasks, &self.section_order, selected, &view)?;
        }

        Ok(())
    }

    pub fn section_tabs(&self) -> Vec<(SectionView, String)> {
        section_tabs_of(&self.tasks, &self.section_order)
    }

    pub fn is_visible(&self, index: usize) -> bool {
        view_shows(&self.view, &self.tasks[index])
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

    // Keep a loaded view valid; see `settled_view`.
    fn settle_view(&mut self) {
        self.view = settled_view(
            &self.tasks,
            &self.section_order,
            &self.view,
            self.list.selected(),
        );
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

    fn cycle_section(&mut self, forward: bool, count: Option<usize>) {
        let tabs = self.section_tabs();

        if tabs.is_empty() {
            self.show_status_message("No sections yet (start a task with \"Name: \")");

            return;
        }

        let current = tabs
            .iter()
            .position(|(view, _)| *view == self.view)
            .unwrap_or(0);

        // Like `navigate`, a counted motion stops at the ends; a plain motion wraps around.
        let last = tabs.len() - 1;
        let next = match (count, forward) {
            (Some(count), true) => current.saturating_add(count).min(last),
            (Some(count), false) => current.saturating_sub(count),
            (None, true) if current == last => 0,
            (None, true) => current + 1,
            (None, false) if current == 0 => last,
            (None, false) => current - 1,
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
            self.show_status_message("No sections yet (start a task with \"Name: \")");

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

    // Move the current section tab `count` places. [*] and [-] stay where they are, and the
    // ends do not wrap around.
    fn move_section(&mut self, right: bool, count: usize, storage: &Storage) -> Result<()> {
        let SectionView::Named(key) = &self.view else {
            return Ok(());
        };

        let Some(position) = self.section_order.iter().position(|named| named == key) else {
            return Ok(());
        };

        let target = if right {
            position
                .saturating_add(count)
                .min(self.section_order.len() - 1)
        } else {
            position.saturating_sub(count)
        };

        if target == position {
            return Ok(());
        }

        // One remove/insert keeps a counted move to a single save and undo step.
        let mut section_order = self.section_order.clone();
        let moved = section_order.remove(position);
        section_order.insert(target, moved);

        // Only the tab order changes; the tasks and their order in [*] stay as they are.
        self.replace_with(
            self.tasks.clone(),
            section_order,
            self.view.clone(),
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
                self.show_status_message("No sections yet (start a task with \"Name: \")");
            } else {
                self.show_status_message("No matching sections");
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

    // Moves the selection and the view by `rows` visual rows: a full viewport for Page Up/Down,
    // half of one for Ctrl+U/D.
    fn page(&mut self, forward: bool, rows: usize) {
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

        // A very short (or not yet drawn) viewport still moves by at least one row.
        let rows = rows.max(1);

        let target = if forward {
            anchor.saturating_add(rows)
        } else {
            anchor.saturating_sub(rows)
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
                self.scroll.saturating_add(rows)
            } else {
                self.scroll.saturating_sub(rows)
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
        let view = self.view.clone();

        self.replace_with(tasks, section_order, view, selected, label, storage)
    }

    // Update memory only after a successful save. Any save error exits the event loop;
    // after a post-replacement error, the next run reloads the file already on disk.
    // `view` is the tab the change switches to (e.g. [*] once a tab empties); it is saved
    // with the tasks and, like them, only takes effect once the save succeeds.
    // `section_order` is settled against `tasks`, so new sections join at the end.
    fn replace_with(
        &mut self,
        tasks: Vec<Task>,
        section_order: Vec<String>,
        view: SectionView,
        selected: Option<usize>,
        label: impl Into<String>,
        storage: &Storage,
    ) -> Result<()> {
        let section_order = settled_section_order(&section_order, &tasks);
        let view = settled_view(&tasks, &section_order, &view, selected);

        self.persisted
            .save(storage, &tasks, &section_order, selected, &view)?;

        let previous_tasks = std::mem::replace(&mut self.tasks, tasks);
        let previous_order = std::mem::replace(&mut self.section_order, section_order);
        let previous_selected = self.list.selected();
        let previous_view = std::mem::replace(&mut self.view, view);
        self.list.select(selected);
        self.reveal_selection = true;

        if self.history.len() == HISTORY_LIMIT {
            self.history.pop_front();
        }

        self.history.push_back(UndoState {
            tasks: previous_tasks,
            section_order: previous_order,
            selected: previous_selected,
            view: previous_view,
            left_view: self.view.clone(),
            label: label.into(),
        });
        self.redo_history.clear();

        Ok(())
    }

    #[cfg(test)]
    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    #[cfg(test)]
    pub fn can_redo(&self) -> bool {
        !self.redo_history.is_empty()
    }

    pub fn section_jump_pending(&self) -> bool {
        self.section_jump_expires_in(Instant::now()).is_some()
    }

    // How long a pending `s` prefix has left, so the event loop can redraw without its hint.
    pub fn section_jump_expires_in(&self, now: Instant) -> Option<Duration> {
        let typed_at = self.pending_section_jump?;
        let remaining = PREFIX_TIMEOUT.saturating_sub(now.saturating_duration_since(typed_at));

        (!remaining.is_zero()).then_some(remaining)
    }

    pub fn search_query(&self) -> Option<&str> {
        match &self.mode {
            Mode::Searching { input, .. } => Some(input.text()),
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

    // Refocuses search from its anchor, but only when the query changed: cursor moves, Tab,
    // and ignored keys keep whichever match is focused.
    fn refresh_search(&mut self, previous_query: Option<&str>) {
        let Mode::Searching { input, original } = &self.mode else {
            return;
        };

        if previous_query == Some(input.text()) {
            return;
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

    fn finish_search(&mut self, accept: bool) {
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);

        if let Mode::Searching { original, .. } = mode {
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
        let source = if redo {
            &mut self.redo_history
        } else {
            &mut self.history
        };

        let Some(previous) = source.back() else {
            self.show_status_message(if redo {
                "Nothing to redo"
            } else {
                "Nothing to undo"
            });

            return Ok(());
        };

        let selected = previous
            .selected
            .filter(|&index| index < previous.tasks.len());

        // Return to the snapshot's tab and selection only if the user hasn't switched tabs
        // since the change. Otherwise keep their tab ([*] if it no longer exists) and move
        // the selection into it, so a task from another tab cannot pull the view away.
        let (view, selected) = if self.view == previous.left_view {
            let view = settled_view(
                &previous.tasks,
                &previous.section_order,
                &previous.view,
                selected,
            );

            (view, selected)
        } else {
            let view = settled_view(&previous.tasks, &previous.section_order, &self.view, None);

            let selected = selected
                .filter(|&index| view_shows(&view, &previous.tasks[index]))
                .or_else(|| {
                    previous
                        .tasks
                        .iter()
                        .position(|task| view_shows(&view, task))
                });

            (view, selected)
        };

        self.persisted.save(
            storage,
            &previous.tasks,
            &previous.section_order,
            selected,
            &view,
        )?;
        let previous = source.pop_back().unwrap();

        let verb = if redo { "Redid" } else { "Undid" };
        let message = format!("{verb}: {}", previous.label);

        let current_tasks = std::mem::replace(&mut self.tasks, previous.tasks);
        let current_order = std::mem::replace(&mut self.section_order, previous.section_order);
        let current_selected = self.list.selected();
        let current_view = std::mem::replace(&mut self.view, view);
        self.list.select(selected);
        self.reveal_selection = true;

        // No eviction is needed: mutations cap undo history and clear redo history, and undo
        // and redo only move one entry, so both together never exceed HISTORY_LIMIT.
        let destination = if redo {
            &mut self.history
        } else {
            &mut self.redo_history
        };

        // The label describes the same change in either direction, so it moves with it.
        destination.push_back(UndoState {
            tasks: current_tasks,
            section_order: current_order,
            selected: current_selected,
            view: current_view,
            left_view: self.view.clone(),
            label: previous.label,
        });
        self.show_status_message(message);

        Ok(())
    }

    fn toggle(&mut self, index: usize, storage: &Storage) -> Result<()> {
        let (status, label) = match self.tasks[index].status {
            TaskStatus::Pending | TaskStatus::Carried => (TaskStatus::Completed, "complete task"),
            TaskStatus::Completed => (TaskStatus::Pending, "uncomplete task"),
        };

        let mut tasks = self.tasks.clone();
        tasks[index].status = status;

        // The task stays in place, so do not force the viewport to scroll to it.
        self.replace(tasks, Some(index), label, storage)?;
        self.reveal_selection = false;

        Ok(())
    }

    fn toggle_carryover(&mut self, storage: &Storage) -> Result<()> {
        let Some(index) = self.list.selected() else {
            return Ok(());
        };

        let (status, label) = match self.tasks[index].status {
            TaskStatus::Pending => (TaskStatus::Carried, "mark carryover"),
            TaskStatus::Carried => (TaskStatus::Pending, "unmark carryover"),
            TaskStatus::Completed => {
                self.show_status_message("Completed tasks can't be carried");

                return Ok(());
            }
        };

        let mut tasks = self.tasks.clone();
        tasks[index].status = status;

        self.replace(tasks, Some(index), label, storage)
    }

    fn clear_unmarked(&mut self, storage: &Storage) -> Result<()> {
        // Only the current tab's unmarked tasks are cleared; other tabs are untouched.
        let survivors: Vec<usize> = (0..self.tasks.len())
            .filter(|&index| {
                !self.is_visible(index) || self.tasks[index].status == TaskStatus::Carried
            })
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
        let view = if visible_survivors.is_empty() {
            SectionView::All
        } else {
            self.view.clone()
        };

        let section_order = self.section_order.clone();
        let label = format!("clear {cleared}");
        self.replace_with(tasks, section_order, view, selected, label, storage)?;

        if kept == 0 {
            self.show_status_message(format!("Cleared {cleared}"));
        } else {
            self.show_status_message(format!("Cleared {cleared}, kept {kept} carried"));
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
            .filter(|&index| {
                !self.is_visible(index) || self.tasks[index].status != TaskStatus::Completed
            })
            .collect();
        let removed = self.tasks.len() - survivors.len();

        if removed == 0 {
            let section = self.scope_label().map(|label| format!("{label} "));
            self.show_status_message(format!("No completed {}tasks", section.unwrap_or_default()));

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
        let view = if visible_survivors.is_empty() {
            SectionView::All
        } else {
            self.view.clone()
        };

        let section_order = self.section_order.clone();
        let label = format!("clear {cleared}");
        self.replace_with(tasks, section_order, view, selected, label, storage)?;
        self.show_status_message(format!("Cleared {cleared}"));

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
            self.show_status_message("No sections yet (start a task with \"Name: \")");

            return;
        }

        let (SectionView::Named(_), Some(label)) = (&self.view, self.scope_label()) else {
            self.show_status_message("Switch to a section tab to rename it");

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

        // `section_order` lists exactly the keys that tasks use.
        let key = name.to_lowercase();
        let taken = key != *current && self.section_order.contains(&key);

        taken.then_some("Section name already taken")
    }

    // Shown on every key while renaming, even with status messages off, because it explains
    // why Enter does nothing.
    fn explain_rename_problem(&mut self) {
        if let Some(problem) = self.rename_problem() {
            self.show_important_status_message(problem);
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
        self.replace_with(
            tasks,
            section_order,
            SectionView::Named(new_key),
            self.list.selected(),
            "rename section",
            storage,
        )?;

        self.show_status_message(format!(
            "Renamed {old_label} to {name} ({renamed} {})",
            task_noun(renamed)
        ));

        Ok(())
    }

    fn cycle_theme(&mut self, forward: bool) {
        self.theme = self.theme.cycle(forward);
        self.styles =
            crate::styles::Styles::resolve(self.theme, &self.style_overrides, self.cli_accent);
        self.show_status_message(format!("Theme: {}", self.theme.id()));
    }

    fn yank_task(&mut self) {
        if let Some(task) = self.list.selected().and_then(|index| self.tasks.get(index)) {
            self.task_register = Some(task.display_text());
            self.show_status_message("Copied task");
        }
    }

    fn delete_task(&mut self, index: usize, label: &str, storage: &Storage) -> Result<()> {
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
        let view = if visible_neighbor.is_none() {
            SectionView::All
        } else {
            self.view.clone()
        };

        let selected =
            visible_neighbor.or_else(|| (!tasks.is_empty()).then(|| index.min(tasks.len() - 1)));

        let section_order = self.section_order.clone();
        self.replace_with(tasks, section_order, view, selected, label, storage)
    }

    fn paste_task(&mut self, placement: AddPlacement, storage: &Storage) -> Result<()> {
        let Some(text) = self.task_register.clone() else {
            self.show_status_message("Nothing to paste");

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
            Ok(()) => self.show_status_message("Copy sent to terminal"),
            // Failures are always reported, even when confirmations are disabled.
            Err(error) => self.show_important_status_message(format!("Copy failed: {error}")),
        }
    }

    fn show_status_message(&mut self, message: impl Into<String>) {
        if self.status_messages {
            self.show_important_status_message(message);
        }
    }

    // Errors and validation feedback ignore `status_messages`, but share the configured
    // lifetime with routine messages.
    fn show_important_status_message(&mut self, message: impl Into<String>) {
        self.status_message = Some(StatusMessage::new(message, self.status_message_duration));
    }

    pub fn key(&mut self, key: KeyEvent, storage: &Storage) -> Result<bool> {
        let key = self.normalize_key(key);

        if key.kind == KeyEventKind::Release
            || (key.kind == KeyEventKind::Repeat && !self.repeat_allowed(key))
        {
            return Ok(false);
        }

        self.status_message = None;

        // Every accepted key consumes a pending count, even shortcuts that ignore it. An
        // expired count is simply gone, so the key acts as if none was typed.
        let now = Instant::now();
        let count = self
            .pending_count
            .take()
            .filter(|(_, typed_at)| now.saturating_duration_since(*typed_at) < PREFIX_TIMEOUT)
            .map(|(count, _)| count);

        // After `s` / Alt+S, a digit picks a tab. Any other key only cancels the jump, so a
        // mistyped follow-up never runs its own, possibly destructive, shortcut. An expired
        // jump is forgotten, and the key runs normally.
        let section_jump = self.section_jump_expires_in(now).is_some();
        self.pending_section_jump = None;

        if section_jump {
            if let KeyCode::Char(digit @ '0'..='9') = key.code
                && key.modifiers.is_empty()
            {
                self.jump_to_section(digit);
            }

            return Ok(false);
        }

        let previous_query = self.search_query().map(str::to_owned);

        let quit = if self.handle_modified_key(key, storage)? {
            false
        } else {
            match self.mode {
                Mode::Normal => self.handle_normal_key(key, count, storage)?,
                Mode::Adding(_, _)
                | Mode::Editing { .. }
                | Mode::Searching { .. }
                | Mode::RenamingSection { .. }
                | Mode::FuzzySection { .. } => {
                    self.handle_text_input_key(key, storage)?;
                    false
                }
                Mode::ConfirmClear => {
                    self.handle_confirm_clear_key(key, storage)?;
                    false
                }
                Mode::Help { .. } => {
                    self.handle_help_key(key);
                    false
                }
            }
        };

        self.refresh_search(previous_query.as_deref());
        self.refresh_section_picker();
        self.explain_rename_problem();

        Ok(quit)
    }

    fn normalize_key(&self, key: KeyEvent) -> KeyEvent {
        // Emacs-style aliases become the Normal-mode keys they mirror, so they share their
        // dispatch and repeat policy. Input modes keep ignoring these chords.
        if self.mode == Mode::Normal {
            let alias = match (key.modifiers, key.code) {
                (KeyModifiers::CONTROL, KeyCode::Char('o' | 'O')) => Some(KeyCode::Char('o')),
                (KeyModifiers::ALT, KeyCode::Char('e' | 'E')) => Some(KeyCode::Char('e')),
                (KeyModifiers::ALT, KeyCode::Char('g' | 'G')) => Some(KeyCode::Char('`')),
                (KeyModifiers::CONTROL, KeyCode::Char('v' | 'V')) => Some(KeyCode::PageDown),
                (KeyModifiers::ALT, KeyCode::Char('v' | 'V')) => Some(KeyCode::PageUp),
                // Org-mode's Alt+Up/Down move the task like K/J, counts included.
                (KeyModifiers::ALT, KeyCode::Up) => Some(KeyCode::Char('K')),
                (KeyModifiers::ALT, KeyCode::Down) => Some(KeyCode::Char('J')),
                _ => None,
            };

            if let Some(code) = alias {
                return KeyEvent {
                    code,
                    modifiers: KeyModifiers::NONE,
                    ..key
                };
            }
        }

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
            && matches!(key.code, KeyCode::Char('p' | 'P' | 'n' | 'N'))
        {
            // Ctrl+N/P step through whatever is selectable: tasks like Down/Up in Normal mode,
            // help lines like Down/Up in the keyboard reference, and search results, picker
            // matches, or section suggestions like Tab/Shift+Tab.
            let next = matches!(key.code, KeyCode::Char('n' | 'N'));
            let line_based = matches!(self.mode, Mode::Normal | Mode::Help { .. });

            let code = match (line_based, next) {
                (true, true) => KeyCode::Down,
                (true, false) => KeyCode::Up,
                (false, true) => KeyCode::Tab,
                (false, false) => KeyCode::BackTab,
            };

            KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
                ..key
            }
        } else {
            key
        }
    }

    fn repeat_allowed(&self, key: KeyEvent) -> bool {
        match &self.mode {
            Mode::Normal if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) => {
                key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
            }
            // Alt+B/F switch tabs like h/l; Alt+P/N reorder sections and fire once per press.
            Mode::Normal if key.modifiers == KeyModifiers::ALT => {
                matches!(key.code, KeyCode::Char('b' | 'B' | 'f' | 'F'))
            }
            // Ctrl+D/U move half a page like Page Down/Up.
            Mode::Normal if key.modifiers == KeyModifiers::CONTROL => {
                matches!(key.code, KeyCode::Char('d' | 'D' | 'u' | 'U'))
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
            // Stepping through search results, picker matches, or section suggestions repeats
            // like switching tabs in Normal mode; none of them commits anything.
            Mode::Adding(_, _)
            | Mode::Editing { .. }
            | Mode::Searching { .. }
            | Mode::FuzzySection { .. }
                if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) =>
            {
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
            Mode::ConfirmClear => false,
            // Holding a scroll key keeps scrolling; `?` and Esc open or close once per press.
            Mode::Help { .. } => {
                key.modifiers.is_empty()
                    && matches!(
                        key.code,
                        KeyCode::Char('j' | 'k')
                            | KeyCode::Up
                            | KeyCode::Down
                            | KeyCode::PageUp
                            | KeyCode::PageDown
                    )
            }
        }
    }

    // Every key held with Control, Alt, or Super ends here, listed or not, so the mode handlers
    // only see plain and Shift keys: an unlisted chord such as Ctrl+J does nothing rather than
    // acting as `j`. Returns false for keys without those modifiers.
    fn handle_modified_key(&mut self, key: KeyEvent, storage: &Storage) -> Result<bool> {
        if !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return Ok(false);
        }

        // These shortcuts edit the input directly, so a pending suggestion is dropped.
        self.completion = None;

        if self.mode == Mode::Normal {
            let half_page = usize::from(self.viewport_height / 2);

            // Alt+B/F switch sections the way they move between words in text inputs, where
            // Ctrl+D/U delete text instead of moving half a page.
            match (key.modifiers, key.code) {
                (KeyModifiers::ALT, KeyCode::Char('w' | 'W')) => self.yank_task(),
                (KeyModifiers::ALT, KeyCode::Char('b' | 'B')) => self.cycle_section(false, None),
                (KeyModifiers::ALT, KeyCode::Char('f' | 'F')) => self.cycle_section(true, None),
                (KeyModifiers::ALT, KeyCode::Char('p' | 'P')) => {
                    self.move_section(false, 1, storage)?
                }
                (KeyModifiers::ALT, KeyCode::Char('n' | 'N')) => {
                    self.move_section(true, 1, storage)?
                }
                (KeyModifiers::ALT, KeyCode::Char('s' | 'S')) => {
                    self.pending_section_jump = Some(Instant::now())
                }
                (KeyModifiers::CONTROL, KeyCode::Char('z' | 'Z')) => self.undo(storage)?,
                (KeyModifiers::CONTROL, KeyCode::Char('r' | 'R')) => self.redo(storage)?,
                (KeyModifiers::CONTROL, KeyCode::Char('y' | 'Y')) => {
                    self.paste_task(AddPlacement::Below, storage)?
                }
                (KeyModifiers::CONTROL, KeyCode::Char('f' | 'F' | 's' | 'S')) => {
                    self.start_search()
                }
                (KeyModifiers::CONTROL, KeyCode::Char('d' | 'D')) => self.page(true, half_page),
                (KeyModifiers::CONTROL, KeyCode::Char('u' | 'U')) => self.page(false, half_page),
                _ => {}
            }

            return Ok(true);
        }

        // The clear confirmation and the keyboard reference ignore modified keys.
        let Some(input) = self.mode.text_input_mut() else {
            return Ok(true);
        };

        match (key.modifiers, key.code) {
            (KeyModifiers::ALT, KeyCode::Char('b' | 'B')) => input.move_word_left(),
            (KeyModifiers::ALT, KeyCode::Char('f' | 'F')) => input.move_word_right(),
            (KeyModifiers::ALT, KeyCode::Char('d' | 'D')) => input.delete_word_after(),
            (KeyModifiers::ALT, KeyCode::Backspace) => input.delete_word_before(),
            (KeyModifiers::CONTROL, KeyCode::Char('a' | 'A')) => input.set_cursor(0),
            (KeyModifiers::CONTROL, KeyCode::Char('e' | 'E')) => {
                input.set_cursor(input.text().len())
            }
            (KeyModifiers::CONTROL, KeyCode::Left) => input.move_word_left(),
            (KeyModifiers::CONTROL, KeyCode::Right) => input.move_word_right(),
            (KeyModifiers::CONTROL, KeyCode::Backspace | KeyCode::Char('w' | 'W')) => {
                input.delete_word_before()
            }
            (KeyModifiers::CONTROL, KeyCode::Char('b' | 'B')) => input.move_left(),
            (KeyModifiers::CONTROL, KeyCode::Char('f' | 'F')) => input.move_right(),
            (KeyModifiers::CONTROL, KeyCode::Char('d' | 'D')) => input.delete_after(),
            (KeyModifiers::CONTROL, KeyCode::Char('k' | 'K')) => input.delete_to_end(),
            (KeyModifiers::CONTROL, KeyCode::Char('u' | 'U')) => input.delete_to_start(),
            // Windows reports AltGr text as Ctrl+Alt, which the backend cannot tell apart from
            // a literal Ctrl+Alt shortcut, so it is accepted as typed text in inputs only.
            (modifiers, KeyCode::Char(character))
                if cfg!(windows)
                    && modifiers.difference(KeyModifiers::SHIFT)
                        == (KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && !character.is_control() =>
            {
                input.insert(single_line_character(character).encode_utf8(&mut [0; 4]));
            }
            _ => {}
        }

        Ok(true)
    }

    fn handle_text_input_key(&mut self, key: KeyEvent, storage: &Storage) -> Result<()> {
        if matches!(self.mode, Mode::Searching { .. }) {
            if key.code == KeyCode::Esc {
                self.finish_search(false);
                return Ok(());
            }

            if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                let forward =
                    key.code == KeyCode::Tab && !key.modifiers.contains(KeyModifiers::SHIFT);
                self.search_next(forward);

                return Ok(());
            }

            // With no match focused, Enter keeps search open so the query can be fixed; an
            // empty query has nothing to find and just cancels.
            if key.code == KeyCode::Enter {
                if self.search_query() == Some("") {
                    self.finish_search(false);
                } else if self.list.selected().is_some() {
                    self.finish_search(true);
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
                // Enter needs a focused match: none is focused when nothing matches, or
                // when the picker opened on [*] and no edit or Tab has moved yet.
                KeyCode::Enter => {
                    if matches.contains(&self.view) {
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
                KeyCode::Backspace => {
                    self.completion = None;
                    return Ok(());
                }
                // Esc lands here too: it drops the suggestion, then cancels the add or edit
                // below like it does without one.
                _ => self.completion = None,
            }
        }

        let Some(input) = self.mode.text_input_mut() else {
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
            KeyCode::Enter => self.submit_input(storage)?,
            _ => {}
        }

        Ok(())
    }

    // Enter in an add, edit, or rename input; search and the section picker handle their own
    // Enter above. An add or edit closes even when it changes nothing, but a rename stays open
    // until the name can be used.
    fn submit_input(&mut self, storage: &Storage) -> Result<()> {
        match &self.mode {
            Mode::Adding(input, placement) => {
                let placement = *placement;

                if let Some(text) = submitted_task_text(input) {
                    self.insert_task(text, placement, "add task", storage)?;
                }
            }
            Mode::Editing { index, input } => {
                let index = *index;

                // Editing re-reads the "Name: " syntax, so it can change the section.
                if let Some(text) = submitted_task_text(input) {
                    let (section, body) = split_section(&text);

                    if self.tasks[index].section != section || self.tasks[index].text != body {
                        let mut tasks = self.tasks.clone();
                        tasks[index].section = section;
                        tasks[index].text = body;
                        self.replace(tasks, Some(index), "edit task", storage)?;
                    }
                }
            }
            // A taken or invalid name keeps the input open; the notice says why.
            Mode::RenamingSection { input } => {
                if self.rename_problem().is_some() {
                    return Ok(());
                }

                let name = input.text().trim().to_owned();
                self.rename_section(&name, storage)?;
            }
            Mode::Searching { .. }
            | Mode::FuzzySection { .. }
            | Mode::Normal
            | Mode::ConfirmClear
            | Mode::Help { .. } => return Ok(()),
        }

        self.mode = Mode::Normal;
        self.reveal_selection = true;

        Ok(())
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

    fn handle_help_key(&mut self, key: KeyEvent) {
        let Mode::Help { page, .. } = self.mode else {
            return;
        };

        // Paging keeps one line of the previous page in view for context.
        let page_step = page.saturating_sub(1).max(1);

        match key.code {
            // Closing leaves the checklist exactly as it was, so the selection is not revealed.
            KeyCode::Char('?') | KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Char('j') | KeyCode::Down => self.scroll_help(true, 1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_help(false, 1),
            KeyCode::PageDown => self.scroll_help(true, page_step),
            KeyCode::PageUp => self.scroll_help(false, page_step),
            KeyCode::Home => self.scroll_help(false, usize::MAX),
            KeyCode::End => self.scroll_help(true, usize::MAX),
            _ => {}
        }
    }

    // Scrolls the keyboard reference without wrapping, within the bounds of its last draw.
    fn scroll_help(&mut self, down: bool, rows: usize) {
        let Mode::Help {
            scroll, max_scroll, ..
        } = &mut self.mode
        else {
            return;
        };

        *scroll = if down {
            scroll.saturating_add(rows).min(*max_scroll)
        } else {
            scroll.saturating_sub(rows)
        };
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
                self.pending_count = Some((count, Instant::now()));
            }
            KeyCode::Char('/') => self.start_search(),
            KeyCode::Char('`') => self.start_section_picker(),
            // Terminals may report `?` with or without Shift; nothing else reaches here.
            KeyCode::Char('?') => {
                self.mode = Mode::Help {
                    scroll: 0,
                    max_scroll: 0,
                    page: 0,
                };
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
                self.show_status_message(if self.wrap { "Wrap on" } else { "Wrap off" });
            }
            KeyCode::Char('m') => self.toggle_carryover(storage)?,
            // Clearing several tasks takes Shift, so a stray `x` does nothing.
            KeyCode::Char('X') => self.clear_completed(storage)?,
            KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.clear_completed(storage)?;
            }
            KeyCode::Char('J') => self.move_selected(true, count.unwrap_or(1), storage)?,
            KeyCode::Char('K') => self.move_selected(false, count.unwrap_or(1), storage)?,
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.move_selected(true, count.unwrap_or(1), storage)?;
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.move_selected(false, count.unwrap_or(1), storage)?;
            }
            KeyCode::Char('j') | KeyCode::Down => self.navigate(true, count),
            KeyCode::Char('k') | KeyCode::Up => self.navigate(false, count),
            KeyCode::Char('H') => self.move_section(false, count.unwrap_or(1), storage)?,
            KeyCode::Char('L') => self.move_section(true, count.unwrap_or(1), storage)?,
            KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.move_section(false, count.unwrap_or(1), storage)?;
            }
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.move_section(true, count.unwrap_or(1), storage)?;
            }
            KeyCode::Char('h') => self.cycle_section(false, count),
            KeyCode::Char('l') => self.cycle_section(true, count),
            KeyCode::Char('s') => self.pending_section_jump = Some(Instant::now()),
            KeyCode::Tab | KeyCode::BackTab
                if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() =>
            {
                self.cycle_section(
                    key.code == KeyCode::Tab && !key.modifiers.contains(KeyModifiers::SHIFT),
                    None,
                );
            }
            KeyCode::PageUp => self.page(false, usize::from(self.viewport_height)),
            KeyCode::PageDown => self.page(true, usize::from(self.viewport_height)),
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
                    self.delete_task(index, "delete task", storage)?;
                    self.show_status_message("Deleted task");
                }
            }
            KeyCode::Char('D')
                if (0..self.tasks.len()).any(|index| {
                    self.is_visible(index) && self.tasks[index].status != TaskStatus::Carried
                }) =>
            {
                self.mode = Mode::ConfirmClear
            }
            _ => {}
        }

        Ok(false)
    }

    pub fn paste(&mut self, text: &str) {
        self.status_message = None;
        self.pending_count = None;
        self.pending_section_jump = None;
        self.accept_completion();
        let previous_query = self.search_query().map(str::to_owned);

        if let Some(input) = self.mode.text_input_mut() {
            let normalized: String = text
                .replace("\r\n", "\n")
                .chars()
                .map(single_line_character)
                .collect();

            input.insert(&normalized);
        }

        self.refresh_search(previous_query.as_deref());
        self.refresh_section_picker();
        self.explain_rename_problem();
    }

    pub fn mouse(&mut self, event: MouseEvent, rows: Rect, storage: &Storage) -> Result<()> {
        self.status_message = None;
        self.pending_count = None;
        self.pending_section_jump = None;

        // The keyboard reference covers the checklist: the wheel scrolls it, and clicks never
        // reach the tasks or tabs underneath.
        if matches!(self.mode, Mode::Help { .. }) {
            match event.kind {
                MouseEventKind::ScrollDown => self.scroll_help(true, 1),
                MouseEventKind::ScrollUp => self.scroll_help(false, 1),
                _ => {}
            }

            return Ok(());
        }

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
mod tests;
