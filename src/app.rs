use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{layout::Rect, widgets::ListState};
use serde::{Deserialize, Serialize};

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
    ConfirmClear,
}

pub struct App {
    pub tasks: Vec<Task>,
    pub list: ListState,
    pub mode: Mode,
}

impl App {
    pub fn new(tasks: Vec<Task>) -> Self {
        let selected = (!tasks.is_empty()).then_some(0);
        Self {
            tasks,
            list: ListState::default().with_selected(selected),
            mode: Mode::Normal,
        }
    }

    pub fn navigate(&mut self, forward: bool, wrap: bool) {
        if self.tasks.is_empty() {
            return;
        }
        let last = self.tasks.len() - 1;
        let current = self.list.selected().unwrap_or(0);
        let next = if forward {
            if current == last {
                if wrap { 0 } else { last }
            } else {
                current + 1
            }
        } else if current == 0 {
            if wrap { last } else { 0 }
        } else {
            current - 1
        };
        self.list.select(Some(next));
    }

    // Install the new list only after it has been saved successfully.
    fn replace(
        &mut self,
        tasks: Vec<Task>,
        selected: Option<usize>,
        storage: &Storage,
    ) -> Result<()> {
        storage.save(&tasks)?;
        self.tasks = tasks;
        self.list.select(selected);
        Ok(())
    }

    fn toggle(&mut self, storage: &Storage) -> Result<()> {
        if let Some(index) = self.list.selected() {
            let mut tasks = self.tasks.clone();
            tasks[index].completed = !tasks[index].completed;
            self.replace(tasks, Some(index), storage)?;
        }
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

    pub fn key(&mut self, key: KeyEvent, storage: &Storage) -> Result<bool> {
        // Modified shortcuts must not turn control/alt combinations into edits.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return Ok(false);
        }
        match &mut self.mode {
            Mode::Adding(input) => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(character) if !character.is_control() => input.push(character),
                KeyCode::Enter => {
                    let text = input.trim().to_owned();
                    if !text.is_empty() {
                        let mut tasks = self.tasks.clone();
                        tasks.push(Task {
                            text,
                            completed: false,
                        });
                        let selected = Some(tasks.len() - 1);
                        self.replace(tasks, selected, storage)?;
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
                KeyCode::Char('j') | KeyCode::Down => self.navigate(true, true),
                KeyCode::Char('k') | KeyCode::Up => self.navigate(false, true),
                KeyCode::Char(' ') | KeyCode::Enter => self.toggle(storage)?,
                KeyCode::Char('a') => self.mode = Mode::Adding(String::new()),
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

    pub fn mouse(&mut self, event: MouseEvent, rows: Rect, storage: &Storage) -> Result<()> {
        if self.mode != Mode::Normal || !rows.contains((event.column, event.row).into()) {
            return Ok(());
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let index = self.list.offset() + usize::from(event.row - rows.y);
                if index < self.tasks.len() {
                    self.list.select(Some(index));
                    self.toggle(storage)?;
                }
            }
            MouseEventKind::ScrollDown => self.navigate(true, false),
            MouseEventKind::ScrollUp => self.navigate(false, false),
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
    fn mouse_uses_visible_rows_and_ignores_borders_and_modal_input() {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(10);
        *app.list.offset_mut() = 5;
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
        assert_eq!(app.list.selected(), Some(8));
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
