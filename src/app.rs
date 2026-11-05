use std::collections::VecDeque;

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{layout::Rect, widgets::ListState};
use serde::{Deserialize, Serialize};

use crate::input::TextInput;
use crate::storage::{Result, Storage};

const UNDO_LIMIT: usize = 10;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub text: String,
    pub completed: bool,
}

#[derive(Debug, PartialEq)]
pub enum Mode {
    Normal,
    Adding(TextInput),
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
    ConfirmClear,
}

struct UndoState {
    tasks: Vec<Task>,
    selected: Option<usize>,
}

pub struct App {
    pub tasks: Vec<Task>,
    pub list: ListState,
    pub mode: Mode,
    pub wrap: bool,
    pub logo: bool,
    pub keybinds: bool,
    pub rounded: bool,
    pub add_to_bottom: bool,
    pub completed_at_bottom: bool,
    pub accent: ratatui::style::Color,
    pub scroll: usize,
    pub row_tasks: Vec<usize>,
    pub viewport_height: u16,
    pub reveal_selection: bool,
    history: VecDeque<UndoState>,
}

impl App {
    pub fn new(tasks: Vec<Task>) -> Self {
        let selected = (!tasks.is_empty()).then_some(0);
        Self {
            tasks,
            list: ListState::default().with_selected(selected),
            mode: Mode::Normal,
            wrap: true,
            logo: true,
            keybinds: true,
            rounded: false,
            add_to_bottom: false,
            completed_at_bottom: false,
            accent: crate::ui::DEFAULT_ACCENT,
            scroll: 0,
            row_tasks: Vec::new(),
            viewport_height: 0,
            reveal_selection: true,
            history: VecDeque::new(),
        }
    }

    pub fn navigate(&mut self, forward: bool) {
        if self.tasks.is_empty() {
            return;
        }
        let last = self.tasks.len() - 1;
        let current = self.list.selected().unwrap_or(0);
        let next = if forward {
            if current == last { 0 } else { current + 1 }
        } else if current == 0 {
            last
        } else {
            current - 1
        };
        self.list.select(Some(next));
        self.reveal_selection = true;
    }

    fn page(&mut self, forward: bool) {
        if self.tasks.is_empty() {
            return;
        }
        let last = self.tasks.len() - 1;
        let current = self.list.selected().unwrap_or(0);
        let start = self
            .row_tasks
            .iter()
            .position(|&task| task == current)
            .unwrap_or(0);
        // Anchor within a tall task when its beginning has scrolled offscreen.
        let anchor = if self.row_tasks.get(self.scroll) == Some(&current) {
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
            .unwrap_or(current)
            .min(last);
        // A page landing in the same wrapped task must still make task-level progress.
        let next = if forward {
            candidate.max((current + 1).min(last))
        } else {
            candidate.min(current.saturating_sub(1))
        };
        if next != current {
            self.list.select(Some(next));
            self.scroll = if forward {
                self.scroll.saturating_add(height)
            } else {
                self.scroll.saturating_sub(height)
            };
            self.reveal_selection = true;
        }
    }

    // Update memory only after a successful save. Any save error exits the event loop;
    // after a post-replacement error, the next run reloads the file already on disk.
    fn replace(
        &mut self,
        tasks: Vec<Task>,
        selected: Option<usize>,
        storage: &Storage,
    ) -> Result<()> {
        storage.save(&tasks)?;
        if self.history.len() == UNDO_LIMIT {
            self.history.pop_front();
        }
        self.history.push_back(UndoState {
            tasks: std::mem::replace(&mut self.tasks, tasks),
            selected: self.list.selected(),
        });
        self.list.select(selected);
        self.reveal_selection = true;
        Ok(())
    }

    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    pub fn search_query(&self) -> Option<&str> {
        match &self.mode {
            Mode::Searching { input, .. } | Mode::SearchResults { input, .. } => Some(input.text()),
            _ => None,
        }
    }

    fn refresh_search(&mut self) {
        let Mode::Searching { input, original } = &self.mode else {
            return;
        };
        let anchor = original.unwrap_or(0).min(self.tasks.len());
        let selected = (anchor..self.tasks.len())
            .chain(0..anchor)
            .find(|&index| crate::search::matches(&self.tasks[index].text, input.text()));
        self.list.select(selected);
        self.reveal_selection = true;
    }

    fn search_next(&mut self, forward: bool) {
        let Some(query) = self.search_query() else {
            return;
        };
        let Some(current) = self.list.selected() else {
            return;
        };
        let selected = if forward {
            (current + 1..self.tasks.len())
                .chain(0..=current)
                .find(|&index| crate::search::matches(&self.tasks[index].text, query))
        } else {
            (0..current)
                .rev()
                .chain((current..self.tasks.len()).rev())
                .find(|&index| crate::search::matches(&self.tasks[index].text, query))
        };
        self.list.select(selected);
        self.reveal_selection = true;
    }

    fn finish_search(&mut self, accept: bool) {
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        if let Mode::Searching { original, .. } | Mode::SearchResults { original, .. } = mode {
            if !accept {
                self.list.select(original);
            }
            self.reveal_selection = true;
        }
    }

    fn undo(&mut self, storage: &Storage) -> Result<()> {
        if let Some(previous) = self.history.back() {
            storage.save(&previous.tasks)?;
            self.tasks = previous.tasks.clone();
            self.list.select(previous.selected);
            self.reveal_selection = true;
            self.history.pop_back();
        }
        Ok(())
    }

    fn toggle(&mut self, index: usize, storage: &Storage) -> Result<()> {
        let mut tasks = self.tasks.clone();
        tasks[index].completed = !tasks[index].completed;
        let selected = if self.completed_at_bottom {
            let task = tasks.remove(index);
            let boundary = tasks
                .iter()
                .position(|task| task.completed)
                .unwrap_or(tasks.len());
            tasks.insert(boundary, task);
            boundary
        } else {
            index
        };
        self.replace(tasks, Some(selected), storage)?;
        self.reveal_selection = selected != index;
        Ok(())
    }

    fn move_selected(&mut self, down: bool, storage: &Storage) -> Result<()> {
        let Some(index) = self.list.selected() else {
            return Ok(());
        };
        if (down && index == self.tasks.len() - 1) || (!down && index == 0) {
            return Ok(());
        }
        let next = if down { index + 1 } else { index - 1 };
        if self.completed_at_bottom && self.tasks[index].completed != self.tasks[next].completed {
            return Ok(());
        }
        let mut tasks = self.tasks.clone();
        tasks.swap(index, next);
        self.replace(tasks, Some(next), storage)
    }

    fn clear_completed(&mut self, storage: &Storage) -> Result<()> {
        if !self.tasks.iter().any(|task| task.completed) {
            return Ok(());
        }
        // Keep the selected task if it survives, otherwise choose the next survivor
        // or the previous one when there are no survivors after the selection.
        let before = self.tasks[..self.list.selected().unwrap_or(0)]
            .iter()
            .filter(|task| !task.completed)
            .count();
        let mut tasks = self.tasks.clone();
        tasks.retain(|task| !task.completed);
        let selected = if tasks.is_empty() {
            None
        } else {
            Some(before.min(tasks.len() - 1))
        };
        self.replace(tasks, selected, storage)
    }

    pub fn key(&mut self, key: KeyEvent, storage: &Storage) -> Result<bool> {
        let repeat_allowed = match &self.mode {
            Mode::Normal => {
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
            Mode::Adding(_) | Mode::Editing { .. } | Mode::Searching { .. } => {
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
        };
        if key.kind == KeyEventKind::Release
            || (key.kind == KeyEventKind::Repeat && !repeat_allowed)
        {
            return Ok(false);
        }
        // Windows reports AltGr text as Ctrl+Alt; the backend cannot distinguish
        // it from a literal Ctrl+Alt character shortcut. Accept it only in input modes.
        if cfg!(windows)
            && key.modifiers.difference(KeyModifiers::SHIFT)
                == (KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            if let (
                Mode::Adding(input) | Mode::Editing { input, .. } | Mode::Searching { input, .. },
                KeyCode::Char(character),
            ) = (&mut self.mode, key.code)
            {
                if !character.is_control() {
                    input.insert(character.encode_utf8(&mut [0; 4]));
                    self.refresh_search();
                    return Ok(false);
                }
            }
        }
        // Only the explicitly supported input shortcuts may consume other Control events.
        if key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return Ok(false);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if key.modifiers == KeyModifiers::CONTROL {
                if let Mode::Adding(input)
                | Mode::Editing { input, .. }
                | Mode::Searching { input, .. } = &mut self.mode
                {
                    match key.code {
                        KeyCode::Char('a' | 'A') => input.set_cursor(0),
                        KeyCode::Char('e' | 'E') => input.set_cursor(input.text().len()),
                        KeyCode::Left => input.move_word_left(),
                        KeyCode::Right => input.move_word_right(),
                        KeyCode::Backspace => input.delete_word_before(),
                        _ => {}
                    }
                }
            }
            self.refresh_search();
            return Ok(false);
        }
        if matches!(
            self.mode,
            Mode::Searching { .. } | Mode::SearchResults { .. }
        ) {
            if key.code == KeyCode::Esc {
                self.finish_search(false);
                return Ok(false);
            }
            if matches!(self.mode, Mode::SearchResults { .. }) {
                match key.code {
                    KeyCode::Enter => self.finish_search(true),
                    KeyCode::Char('N') | KeyCode::Up => self.search_next(false),
                    KeyCode::Char('n') => {
                        self.search_next(!key.modifiers.contains(KeyModifiers::SHIFT))
                    }
                    KeyCode::Down => self.search_next(true),
                    _ => {}
                }
                return Ok(false);
            }
            if key.code == KeyCode::Enter {
                if self.search_query() == Some("") {
                    self.finish_search(false);
                } else if self.list.selected().is_some() {
                    if let Mode::Searching { input, original } =
                        std::mem::replace(&mut self.mode, Mode::Normal)
                    {
                        self.mode = Mode::SearchResults { input, original };
                    }
                }
                return Ok(false);
            }
        }
        let edited_task = match &self.mode {
            Mode::Editing { index, .. } => Some(*index),
            _ => None,
        };
        match &mut self.mode {
            Mode::Adding(input) | Mode::Editing { input, .. } | Mode::Searching { input, .. } => {
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Normal,
                    KeyCode::Backspace => input.delete_before(),
                    KeyCode::Delete => input.delete_after(),
                    KeyCode::Left => input.move_left(),
                    KeyCode::Right => input.move_right(),
                    KeyCode::Home => input.set_cursor(0),
                    KeyCode::End => input.set_cursor(input.text().len()),
                    KeyCode::Char(character) if !character.is_control() => {
                        input.insert(character.encode_utf8(&mut [0; 4]))
                    }
                    KeyCode::Enter => {
                        let text = input.text().trim().to_owned();
                        if !text.is_empty() {
                            let mut tasks = self.tasks.clone();
                            if let Some(index) = edited_task {
                                if tasks[index].text != text {
                                    tasks[index].text = text;
                                    self.replace(tasks, Some(index), storage)?;
                                }
                            } else {
                                let mut index = if self.add_to_bottom {
                                    tasks.len()
                                } else {
                                    self.list.selected().map_or(tasks.len(), |index| index + 1)
                                };
                                if self.completed_at_bottom {
                                    let boundary = tasks
                                        .iter()
                                        .position(|task| task.completed)
                                        .unwrap_or(tasks.len());
                                    index = index.min(boundary);
                                }
                                tasks.insert(
                                    index,
                                    Task {
                                        text,
                                        completed: false,
                                    },
                                );
                                self.replace(tasks, Some(index), storage)?;
                            }
                        }
                        self.mode = Mode::Normal;
                    }
                    _ => {}
                }
            }
            Mode::ConfirmClear => match key.code {
                KeyCode::Enter => {
                    self.replace(Vec::new(), None, storage)?;
                    self.mode = Mode::Normal;
                }
                KeyCode::Esc => self.mode = Mode::Normal,
                _ => {}
            },
            Mode::Normal => match key.code {
                KeyCode::Char('/') => {
                    self.mode = Mode::Searching {
                        input: TextInput::default(),
                        original: self.list.selected(),
                    };
                    self.list.select(None);
                }
                KeyCode::Char('q') => return Ok(true),
                KeyCode::Char('u') => self.undo(storage)?,
                KeyCode::Char('w') => {
                    self.wrap = !self.wrap;
                    self.reveal_selection = true;
                }
                KeyCode::Char('x') => self.clear_completed(storage)?,
                KeyCode::Char('J') => self.move_selected(true, storage)?,
                KeyCode::Char('K') => self.move_selected(false, storage)?,
                KeyCode::Char('j') | KeyCode::Down
                    if key.modifiers.contains(KeyModifiers::SHIFT) =>
                {
                    self.move_selected(true, storage)?;
                }
                KeyCode::Char('k') | KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.move_selected(false, storage)?;
                }
                KeyCode::Char('j') | KeyCode::Down => self.navigate(true),
                KeyCode::Char('k') | KeyCode::Up => self.navigate(false),
                KeyCode::PageUp => self.page(false),
                KeyCode::PageDown => self.page(true),
                KeyCode::Char('g' | 'G') | KeyCode::Home | KeyCode::End
                    if !self.tasks.is_empty() =>
                {
                    let first = key.code == KeyCode::Home
                        || (key.code == KeyCode::Char('g')
                            && !key.modifiers.contains(KeyModifiers::SHIFT));
                    let index = if first { 0 } else { self.tasks.len() - 1 };
                    self.list.select(Some(index));
                    self.reveal_selection = true;
                }
                KeyCode::Char(' ') | KeyCode::Enter => {
                    if let Some(index) = self.list.selected() {
                        self.toggle(index, storage)?;
                    }
                }
                KeyCode::Char('a') => self.mode = Mode::Adding(TextInput::default()),
                KeyCode::Char('e') => {
                    if let Some(index) = self.list.selected() {
                        self.mode = Mode::Editing {
                            index,
                            input: self.tasks[index].text.clone().into(),
                        };
                    }
                }
                KeyCode::Char('d') => {
                    if let Some(index) = self.list.selected() {
                        let mut tasks = self.tasks.clone();
                        tasks.remove(index);
                        let selected = if tasks.is_empty() {
                            None
                        } else {
                            Some(index.min(tasks.len() - 1))
                        };
                        self.replace(tasks, selected, storage)?;
                    }
                }
                KeyCode::Char('D') if !self.tasks.is_empty() => self.mode = Mode::ConfirmClear,
                _ => {}
            },
            Mode::SearchResults { .. } => unreachable!("results keys handled above"),
        }
        self.refresh_search();
        Ok(false)
    }

    pub fn paste(&mut self, text: &str) {
        if let Mode::Adding(input) | Mode::Editing { input, .. } | Mode::Searching { input, .. } =
            &mut self.mode
        {
            let normalized: String = text
                .replace("\r\n", "\n")
                .chars()
                .map(|character| {
                    if character.is_control() || matches!(character, '\u{2028}' | '\u{2029}') {
                        ' '
                    } else {
                        character
                    }
                })
                .collect();
            input.insert(&normalized);
        }
        self.refresh_search();
    }

    pub fn mouse(&mut self, event: MouseEvent, rows: Rect, storage: &Storage) -> Result<()> {
        if self.mode != Mode::Normal || !rows.contains((event.column, event.row).into()) {
            return Ok(());
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(&index) = self
                    .row_tasks
                    .get(self.scroll + usize::from(event.row - rows.y))
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
                if let Some(selected) = self.list.selected() {
                    if !visible.contains(&selected) {
                        self.list
                            .select(if event.kind == MouseEventKind::ScrollDown {
                                visible.last().copied()
                            } else {
                                visible.first().copied()
                            });
                    }
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
                .map(|index| Task {
                    text: format!("task {index}"),
                    completed: false,
                })
                .collect(),
        )
    }

    fn press(app: &mut App, fixture: &Fixture, code: KeyCode) -> bool {
        app.key(KeyEvent::new(code, KeyModifiers::NONE), &fixture.storage)
            .unwrap()
    }

    fn repeat(app: &mut App, fixture: &Fixture, code: KeyCode, modifiers: KeyModifiers) {
        let key = KeyEvent {
            kind: KeyEventKind::Repeat,
            ..KeyEvent::new(code, modifiers)
        };
        assert!(!app.key(key, &fixture.storage).unwrap());
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
                        Mode::Adding(input)
                    };
                    app.key(
                        KeyEvent::new_with_kind(code, modifiers, kind),
                        &fixture.storage,
                    )
                    .unwrap();
                    let input = match &app.mode {
                        Mode::Adding(input) | Mode::Editing { input, .. } => input,
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
                Mode::Adding(text.into())
            };
            let original_mode = format!("{:?}", app.mode);
            for (code, modifiers) in [
                (KeyCode::Delete, KeyModifiers::CONTROL),
                (KeyCode::Char('u'), KeyModifiers::CONTROL),
                (KeyCode::Left, KeyModifiers::CONTROL | KeyModifiers::ALT),
                (
                    KeyCode::Char('e'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                ),
                (KeyCode::Left, KeyModifiers::ALT),
                (KeyCode::Char('Q'), KeyModifiers::SUPER),
            ] {
                app.key(KeyEvent::new(code, modifiers), &fixture.storage)
                    .unwrap();
                assert_eq!(format!("{:?}", app.mode), original_mode);
            }
            app.key(
                KeyEvent::new_with_kind(KeyCode::Home, KeyModifiers::NONE, KeyEventKind::Release),
                &fixture.storage,
            )
            .unwrap();
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
                app.key(KeyEvent::new(code, KeyModifiers::CONTROL), &fixture.storage)
                    .unwrap();
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
                text: text.into(),
                completed,
            })
            .into(),
        );
        app.completed_at_bottom = true;
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
        app.key(
            KeyEvent::new(
                KeyCode::Char('€'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ),
            &fixture.storage,
        )
        .unwrap();
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
            press(&mut app, &fixture, KeyCode::Enter);
            for code in [
                KeyCode::Char('n'),
                KeyCode::Char('N'),
                KeyCode::Down,
                KeyCode::Up,
            ] {
                press(&mut app, &fixture, code);
            }
            assert_eq!(app.list.selected(), selection);
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
            app.key(
                KeyEvent::new_with_kind(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Release),
                &fixture.storage,
            )
            .unwrap();
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
    fn completed_group_is_a_stack_and_each_toggle_undoes_as_one_change() {
        let fixture = Fixture::new();
        let original: Vec<_> = [("a", false), ("b", false), ("c", true), ("d", true)]
            .map(|(text, completed)| Task {
                text: text.into(),
                completed,
            })
            .into();
        let mut app = App::new(original.clone());
        app.completed_at_bottom = true;
        for (index, order, selected) in [
            (0, vec!["b", "a", "c", "d"], 1),
            (0, vec!["b", "a", "c", "d"], 0),
            (2, vec!["c", "b", "a", "d"], 0),
        ] {
            app.toggle(index, &fixture.storage).unwrap();
            assert_eq!(
                app.tasks
                    .iter()
                    .map(|task| task.text.as_str())
                    .collect::<Vec<_>>(),
                order
            );
            assert_eq!(app.list.selected(), Some(selected));
            assert_eq!(fixture.storage.load().unwrap(), app.tasks);
        }
        assert_eq!(app.history.len(), 3);
        for selected in [0, 1, 0] {
            press(&mut app, &fixture, KeyCode::Char('u'));
            assert_eq!(app.list.selected(), Some(selected));
            assert_eq!(fixture.storage.load().unwrap(), app.tasks);
        }
        assert_eq!(app.tasks, original);
        assert!(!app.can_undo());

        // A failed toggle or undo must not change the ordering, selection, or history.
        app.toggle(0, &fixture.storage).unwrap();
        let before = app.tasks.clone();
        let selected = app.list.selected();
        let data = fixture.directory.join("data");
        let backup = fixture.directory.join("backup");
        std::fs::rename(&data, &backup).unwrap();
        std::fs::write(&data, "blocked directory").unwrap();
        assert!(app.toggle(0, &fixture.storage).is_err());
        assert!(app.undo(&fixture.storage).is_err());
        assert_eq!(app.tasks, before);
        assert_eq!(app.list.selected(), selected);
        assert_eq!(app.history.len(), 1);
        std::fs::remove_file(&data).unwrap();
        std::fs::rename(backup, data).unwrap();
        app.undo(&fixture.storage).unwrap();
        assert_eq!(app.tasks, original);
    }

    #[test]
    fn additions_and_reordering_stay_within_completion_groups() {
        let fixture = Fixture::new();
        for add_to_bottom in [false, true] {
            let mut app = app_with_tasks(4);
            app.tasks[2].completed = true;
            app.tasks[3].completed = true;
            app.completed_at_bottom = true;
            app.add_to_bottom = add_to_bottom;
            let original = app.tasks.clone();
            app.list.select(Some(1));
            press(&mut app, &fixture, KeyCode::Char('J'));
            app.list.select(Some(2));
            press(&mut app, &fixture, KeyCode::Char('K'));
            assert_eq!(app.tasks, original);
            assert!(!app.can_undo());
            press(&mut app, &fixture, KeyCode::Char('J'));
            assert_eq!(app.tasks[3], original[2]);
            press(&mut app, &fixture, KeyCode::Char('u'));
            assert_eq!(app.tasks, original);
            press(&mut app, &fixture, KeyCode::Char('a'));
            press(&mut app, &fixture, KeyCode::Char('n'));
            press(&mut app, &fixture, KeyCode::Enter);
            assert_eq!(app.list.selected(), Some(2));
            assert_eq!(app.tasks[2].text, "n");
            assert!(!app.tasks[2].completed);
            assert!(app.tasks[3..].iter().all(|task| task.completed));
            assert_eq!(fixture.storage.load().unwrap(), app.tasks);
            press(&mut app, &fixture, KeyCode::Char('u'));
            assert_eq!(app.tasks, original);
        }
        let mut app = app_with_tasks(1);
        app.completed_at_bottom = true;
        for _ in 0..2 {
            press(&mut app, &fixture, KeyCode::Enter);
        }
        assert!(!app.tasks[0].completed);
        assert_eq!(app.list.selected(), Some(0));
        app.tasks.clear();
        app.list.select(None);
        press(&mut app, &fixture, KeyCode::Enter);
        assert!(app.tasks.is_empty());
    }

    #[test]
    fn altgr_characters_are_windows_input_only_and_respect_event_kinds() {
        let fixture = Fixture::new();
        let ctrl_alt = KeyModifiers::CONTROL | KeyModifiers::ALT;
        for edit in [false, true] {
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
                    for character in ['@', '{', '€', 'ł', 'A', 'a', 'e', '\u{301}', '\n', '\0'] {
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
                            expected.insert(character.encode_utf8(&mut [0; 4]));
                        }
                        app.mode = if edit {
                            Mode::Editing { index: 0, input }
                        } else {
                            Mode::Adding(input)
                        };
                        assert!(
                            !app.key(
                                KeyEvent::new_with_kind(KeyCode::Char(character), modifiers, kind),
                                &fixture.storage,
                            )
                            .unwrap()
                        );
                        let actual = match &app.mode {
                            Mode::Adding(input) | Mode::Editing { input, .. } => input,
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
                assert!(
                    !app.key(
                        KeyEvent::new(KeyCode::Char(character), ctrl_alt),
                        &fixture.storage
                    )
                    .unwrap()
                );
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
                Mode::Adding(input)
            };
            press(&mut app, &fixture, KeyCode::Char('界'));
            app.paste("e\u{301}\r\nq\tD\u{2028}");
            match &app.mode {
                Mode::Adding(input) | Mode::Editing { input, .. } => {
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
                Mode::Adding(input) | Mode::Editing { input, .. } => input.text().to_owned(),
                _ => unreachable!(),
            };
            for grapheme in ["e\u{301}", "👩‍💻", "👨‍👩‍👧‍👦", "👍🏽", "🇺🇳"]
            {
                for character in grapheme.chars() {
                    press(&mut app, &fixture, KeyCode::Char(character));
                }
                press(&mut app, &fixture, KeyCode::Backspace);
                match &app.mode {
                    Mode::Adding(input) | Mode::Editing { input, .. } => {
                        assert_eq!(input.text(), prefix)
                    }
                    _ => unreachable!(),
                }
            }
            app.paste("e\u{301}👩‍💻");
            repeat(&mut app, &fixture, KeyCode::Backspace, KeyModifiers::NONE);
            match &app.mode {
                Mode::Adding(input) | Mode::Editing { input, .. } => {
                    assert_eq!(input.text(), &format!("{prefix}e\u{301}"))
                }
                _ => unreachable!(),
            }
            for _ in 0..prefix.graphemes(true).count() + 2 {
                repeat(&mut app, &fixture, KeyCode::Backspace, KeyModifiers::NONE);
            }
            match &app.mode {
                Mode::Adding(input) | Mode::Editing { input, .. } => {
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
            assert_eq!(app.mode, Mode::Adding(expected.into()));
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
            app.row_tasks = (0..count).collect();
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
                Mode::Adding("input".into()),
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
                    Mode::Adding(input) | Mode::Editing { input, .. } => {
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
            app.key(KeyEvent::new(code, KeyModifiers::SHIFT), &fixture.storage)
                .unwrap();
            assert_eq!(app.list.selected(), Some(2));
        }
        app.list.select(Some(1));
        for mode in [
            Mode::Adding(TextInput::default()),
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
                Mode::Adding(input) | Mode::Editing { input, .. } => assert_eq!(input.text(), "gG"),
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
                Mode::Adding(input) | Mode::Editing { input, .. } => {
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
            Mode::Adding(TextInput::default()),
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
                Mode::Adding(input) | Mode::Editing { input, .. } => {
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
        for keys in [
            vec![Char('d')],
            vec![Char('x')],
            vec![Char(' ')],
            vec![Char('J')],
            vec![Char('K')],
            vec![Char('D'), Enter],
            vec![Char('a'), Char('x'), Enter],
            vec![Char('e'), Char('x'), Enter],
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
            app.navigate(true);
            press(&mut app, &fixture, Char('u'));
            assert_eq!(app.tasks, original);
            assert_eq!(app.list.selected(), Some(1));
            assert_eq!(fixture.storage.load().unwrap(), original);
            assert!(app.reveal_selection);
            assert!(app.history.is_empty());
            press(&mut app, &fixture, Char('u'));
            assert_eq!(app.tasks, original);
        }
    }

    #[test]
    fn undo_history_discards_oldest_entries_after_ten_mutations() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(0);
        for index in 0..12 {
            press(&mut app, &fixture, KeyCode::Char('a'));
            app.paste(&format!("task {index}"));
            press(&mut app, &fixture, KeyCode::Enter);
            assert_eq!(app.history.len(), (index + 1).min(10));
        }
        for remaining in (2..12).rev() {
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
                assert!(app.row_tasks[app.scroll..end].contains(&selected));
            }
        }
        assert!(app.history.is_empty());
    }

    #[test]
    fn undo_walks_back_through_history_and_does_not_survive_restart() {
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
                Mode::Adding(input) | Mode::Editing { input, .. } => {
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
        let history: Vec<_> = app
            .history
            .iter()
            .map(|state| (state.tasks.clone(), state.selected))
            .collect();
        let data = fixture.directory.join("data");
        let backup = fixture.directory.join("backup");
        std::fs::rename(&data, &backup).unwrap();
        std::fs::write(&data, "blocking file").unwrap();
        for code in [KeyCode::Char('d'), KeyCode::Char('x'), KeyCode::Char('u')] {
            assert!(
                app.key(KeyEvent::new(code, KeyModifiers::NONE), &fixture.storage)
                    .is_err()
            );
            assert_eq!(app.tasks, modified);
            assert_eq!(app.list.selected(), Some(1));
            assert_eq!(
                app.history
                    .iter()
                    .map(|state| (state.tasks.clone(), state.selected))
                    .collect::<Vec<_>>(),
                history
            );
        }
        std::fs::remove_file(data.clone()).unwrap();
        std::fs::rename(backup, data).unwrap();
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
            app.key(KeyEvent::new(down, modifiers), &fixture.storage)
                .unwrap();
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
            app.key(KeyEvent::new(up, modifiers), &fixture.storage)
                .unwrap();
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
        for mode in [Mode::Adding(TextInput::default()), Mode::ConfirmClear] {
            app.mode = mode;
            for code in [
                KeyCode::Char('J'),
                KeyCode::Char('K'),
                KeyCode::Down,
                KeyCode::Up,
            ] {
                app.key(KeyEvent::new(code, KeyModifiers::SHIFT), &fixture.storage)
                    .unwrap();
            }
            assert_eq!(app.tasks, original);
            assert_eq!(app.list.selected(), Some(1));
            assert!(!fixture.task_path().exists());
            if let Mode::Adding(input) = &app.mode {
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
    fn adding_inserts_below_focus_or_at_bottom_and_undo_restores_order() {
        for add_to_bottom in [false, true] {
            for (count, selected) in [
                (0, None),
                (3, Some(0)),
                (3, Some(1)),
                (3, Some(2)),
                (3, None),
            ] {
                let fixture = Fixture::new();
                let mut app = app_with_tasks(count);
                app.add_to_bottom = add_to_bottom;
                app.list.select(selected);
                if count > 0 {
                    app.tasks[0].completed = true;
                }
                let original = app.tasks.clone();
                let expected_index = if add_to_bottom {
                    count
                } else {
                    selected.map_or(count, |index| index + 1)
                };
                let mut expected = original.clone();
                expected.insert(
                    expected_index,
                    Task {
                        text: "New task".into(),
                        completed: false,
                    },
                );
                press(&mut app, &fixture, KeyCode::Char('a'));
                app.paste("New task");
                press(&mut app, &fixture, KeyCode::Enter);
                assert_eq!(app.tasks, expected);
                assert_eq!(app.list.selected(), Some(expected_index));
                assert_eq!(fixture.storage.load().unwrap(), expected);
                assert!(app.reveal_selection);
                press(&mut app, &fixture, KeyCode::Char('u'));
                assert_eq!(app.tasks, original);
                assert_eq!(app.list.selected(), selected);
                assert_eq!(fixture.storage.load().unwrap(), original);
            }
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
        app.row_tasks = vec![0, 1, 2];
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
            app.row_tasks = vec![0, 1, 2];
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
        app.row_tasks = (0..10).collect();
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
        app.mode = Mode::Adding(TextInput::default());
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
