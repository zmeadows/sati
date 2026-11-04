use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{layout::Rect, widgets::ListState};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::storage::{Result, Storage};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub text: String,
    pub completed: bool,
}

#[derive(Debug, PartialEq)]
pub enum Mode {
    Normal,
    Adding(String),
    Editing { index: usize, input: String },
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
    pub rounded: bool,
    pub add_to_bottom: bool,
    pub accent: ratatui::style::Color,
    pub scroll: usize,
    pub row_tasks: Vec<usize>,
    pub reveal_selection: bool,
    previous: Option<UndoState>,
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
            rounded: false,
            add_to_bottom: false,
            accent: crate::ui::DEFAULT_ACCENT,
            scroll: 0,
            row_tasks: Vec::new(),
            reveal_selection: true,
            previous: None,
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

    // Update memory only after a fully synced save. Any save error exits the event loop;
    // after a post-replacement error, the next run reloads the file already on disk.
    fn replace(
        &mut self,
        tasks: Vec<Task>,
        selected: Option<usize>,
        storage: &Storage,
    ) -> Result<()> {
        storage.save(&tasks)?;
        self.previous = Some(UndoState {
            tasks: std::mem::replace(&mut self.tasks, tasks),
            selected: self.list.selected(),
        });
        self.list.select(selected);
        self.reveal_selection = true;
        Ok(())
    }

    fn undo(&mut self, storage: &Storage) -> Result<()> {
        if let Some(previous) = &self.previous {
            storage.save(&previous.tasks)?;
            self.tasks = previous.tasks.clone();
            self.list.select(previous.selected);
            self.reveal_selection = true;
            self.previous = None;
        }
        Ok(())
    }

    fn toggle(&mut self, index: usize, storage: &Storage) -> Result<()> {
        let mut tasks = self.tasks.clone();
        tasks[index].completed = !tasks[index].completed;
        self.replace(tasks, Some(index), storage)?;
        self.reveal_selection = false;
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
                        KeyCode::Char('j' | 'k') | KeyCode::Up | KeyCode::Down
                    )
            }
            Mode::Adding(_) | Mode::Editing { .. } => {
                matches!(key.code, KeyCode::Char(_) | KeyCode::Backspace)
            }
            Mode::ConfirmClear => false,
        };
        if key.kind == KeyEventKind::Release
            || (key.kind == KeyEventKind::Repeat && !repeat_allowed)
        {
            return Ok(false);
        }
        // Modified shortcuts must not turn control/alt combinations into edits.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return Ok(false);
        }
        let edited_task = match &self.mode {
            Mode::Editing { index, .. } => Some(*index),
            _ => None,
        };
        match &mut self.mode {
            Mode::Adding(input) | Mode::Editing { input, .. } => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Backspace => {
                    if let Some((index, _)) = input.grapheme_indices(true).next_back() {
                        input.truncate(index);
                    }
                }
                KeyCode::Char(character) if !character.is_control() => input.push(character),
                KeyCode::Enter => {
                    let text = input.trim().to_owned();
                    if !text.is_empty() {
                        let mut tasks = self.tasks.clone();
                        if let Some(index) = edited_task {
                            if tasks[index].text != text {
                                tasks[index].text = text;
                                self.replace(tasks, Some(index), storage)?;
                            }
                        } else {
                            let index = if self.add_to_bottom {
                                tasks.len()
                            } else {
                                self.list.selected().map_or(tasks.len(), |index| index + 1)
                            };
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
            },
            Mode::ConfirmClear => match key.code {
                KeyCode::Enter => {
                    self.replace(Vec::new(), None, storage)?;
                    self.mode = Mode::Normal;
                }
                KeyCode::Esc => self.mode = Mode::Normal,
                _ => {}
            },
            Mode::Normal => match key.code {
                KeyCode::Char('q') => return Ok(true),
                KeyCode::Char('u') => self.undo(storage)?,
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
                KeyCode::Char(' ') | KeyCode::Enter => {
                    if let Some(index) = self.list.selected() {
                        self.toggle(index, storage)?;
                    }
                }
                KeyCode::Char('a') => self.mode = Mode::Adding(String::new()),
                KeyCode::Char('e') => {
                    if let Some(index) = self.list.selected() {
                        self.mode = Mode::Editing {
                            index,
                            input: self.tasks[index].text.clone(),
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
        }
        Ok(false)
    }

    pub fn paste(&mut self, text: &str) {
        if let Mode::Adding(input) | Mode::Editing { input, .. } = &mut self.mode {
            input.extend(text.replace("\r\n", "\n").chars().map(|character| {
                if character.is_control() || matches!(character, '\u{2028}' | '\u{2029}') {
                    ' '
                } else {
                    character
                }
            }));
        }
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
    fn backspace_removes_whole_graphemes_in_add_and_edit_modes() {
        for start in [KeyCode::Char('a'), KeyCode::Char('e')] {
            let fixture = Fixture::new();
            let mut app = app_with_tasks(1);
            let original = app.tasks.clone();
            press(&mut app, &fixture, start);
            let prefix = match &app.mode {
                Mode::Adding(input) | Mode::Editing { input, .. } => input.clone(),
                _ => unreachable!(),
            };
            for grapheme in ["e\u{301}", "👩‍💻", "👨‍👩‍👧‍👦", "👍🏽", "🇺🇳"]
            {
                for character in grapheme.chars() {
                    press(&mut app, &fixture, KeyCode::Char(character));
                }
                press(&mut app, &fixture, KeyCode::Backspace);
                match &app.mode {
                    Mode::Adding(input) | Mode::Editing { input, .. } => assert_eq!(input, &prefix),
                    _ => unreachable!(),
                }
            }
            app.paste("e\u{301}👩‍💻");
            repeat(&mut app, &fixture, KeyCode::Backspace, KeyModifiers::NONE);
            match &app.mode {
                Mode::Adding(input) | Mode::Editing { input, .. } => {
                    assert_eq!(input, &format!("{prefix}e\u{301}"))
                }
                _ => unreachable!(),
            }
            for _ in 0..prefix.graphemes(true).count() + 2 {
                repeat(&mut app, &fixture, KeyCode::Backspace, KeyModifiers::NONE);
            }
            match &app.mode {
                Mode::Adding(input) | Mode::Editing { input, .. } => assert!(input.is_empty()),
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
                    assert!(input.ends_with("uqd日本"))
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
        assert!(app.previous.is_none());
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
            Mode::Adding(String::new()),
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
                Mode::Adding(input) | Mode::Editing { input, .. } => assert!(input.ends_with('x')),
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
            assert!(app.previous.is_none());
            press(&mut app, &fixture, Char('u'));
            assert_eq!(app.tasks, original);
        }
    }

    #[test]
    fn undo_has_one_level_and_no_history_after_restart() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(2);
        press(&mut app, &fixture, KeyCode::Char('d'));
        let preceding = app.tasks.clone();
        press(&mut app, &fixture, KeyCode::Char('d'));
        assert!(app.tasks.is_empty());
        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, preceding);
        assert_eq!(app.list.selected(), Some(0));
        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, preceding);
        let mut restarted = App::new(fixture.storage.load().unwrap());
        press(&mut restarted, &fixture, KeyCode::Char('u'));
        assert_eq!(restarted.tasks, preceding);
        assert!(restarted.previous.is_none());
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
                Mode::Adding(input) | Mode::Editing { input, .. } => assert!(input.ends_with('u')),
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
        let modified = app.tasks.clone();
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
            assert_eq!(app.list.selected(), Some(0));
            assert_eq!(app.previous.as_ref().unwrap().tasks, original);
        }
        std::fs::remove_file(data.clone()).unwrap();
        std::fs::rename(backup, data).unwrap();
        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(app.tasks, original);
        assert_eq!(fixture.storage.load().unwrap(), original);
        assert!(app.previous.is_none());
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
        for mode in [Mode::Adding(String::new()), Mode::ConfirmClear] {
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
                assert_eq!(input, "JK");
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
        assert!(app.previous.is_none());
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
            assert_eq!(app.previous.is_some(), has_previous);
            if let Some(previous) = &app.previous {
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
        app.mode = Mode::Adding(String::new());
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
