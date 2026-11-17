// The task finder opened with `f`: a fuzzy search of every task, whatever the current tab,
// drawn in place of the checklist. Enter goes to the focused match without changing it.

use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::{App, Mode, edit_text_input, repeats_while_typing, task_display_text};
use crate::input::TextInput;
use crate::model::{SectionView, Task, TaskStatus, view_includes_task};

#[derive(Debug, PartialEq)]
pub struct TaskFinder {
    pub(crate) input: TextInput,
    // Indices into `App::tasks` that match `matched_query`, best first. Tasks can't change
    // while the finder is open, so the list stays valid until the query does.
    pub(crate) matches: Vec<usize>,
    // The query `matches` were found for, which tells edits apart from cursor moves.
    matched_query: String,
    // The focused match, as a position in `matches`.
    pub(crate) focused: usize,
    // The first result row shown. Drawing clamps it and keeps the focused match in view.
    pub(crate) scroll: usize,
}

// Where the finder's results were last drawn, so the mouse acts on what is on screen and the
// page keys move by what fits. `task_by_row` describes only the visible rows, top to bottom.
#[derive(Debug, Default)]
pub(crate) struct FinderViewport {
    pub(crate) results: Rect,
    pub(crate) task_by_row: Vec<usize>,
}

// Tasks whose text, shown as on [*] with any section prefix, fuzzy-matches `query`, best
// first. Of equally good matches, unfinished tasks come before completed ones, each group in
// list order. A blank query matches nothing.
pub(crate) fn finder_matches(tasks: &[Task], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return Vec::new();
    }

    // `fuzzy_rank` keeps equal scores in the given order.
    let (completed, unfinished): (Vec<usize>, Vec<usize>) =
        (0..tasks.len()).partition(|&index| tasks[index].status == TaskStatus::Completed);
    let candidates: Vec<usize> = unfinished.into_iter().chain(completed).collect();

    let texts: Vec<String> = candidates
        .iter()
        .map(|&index| task_display_text(&tasks[index], true).text)
        .collect();
    let names: Vec<&str> = texts.iter().map(String::as_str).collect();

    crate::search::fuzzy_rank(query, &names)
        .into_iter()
        .map(|rank| candidates[rank])
        .collect()
}

impl TaskFinder {
    pub(crate) fn repeat_allowed(&self, key: KeyEvent) -> bool {
        repeats_while_typing(key.code)
            || matches!(
                key.code,
                KeyCode::Tab
                    | KeyCode::BackTab
                    | KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::PageUp
                    | KeyCode::PageDown
            )
    }

    // A single step wraps around the ends, and a page stops there.
    fn move_focus(&mut self, forward: bool, by: Option<usize>) {
        if self.matches.is_empty() {
            return;
        }

        let last = self.matches.len() - 1;
        let current = self.focused.min(last);

        self.focused = match (by, forward) {
            (Some(by), true) => current.saturating_add(by).min(last),
            (Some(by), false) => current.saturating_sub(by),
            (None, true) if current == last => 0,
            (None, true) => current + 1,
            (None, false) if current == 0 => last,
            (None, false) => current - 1,
        };
    }
}

impl Mode {
    fn task_finder(&self) -> Option<&TaskFinder> {
        match self {
            Mode::FindingTask(finder) => Some(finder),
            _ => None,
        }
    }

    fn task_finder_mut(&mut self) -> Option<&mut TaskFinder> {
        match self {
            Mode::FindingTask(finder) => Some(finder),
            _ => None,
        }
    }
}

impl App {
    // Nothing moves until a match is accepted, so the checklist stays as it was underneath.
    pub(super) fn open_task_finder(&mut self) {
        self.mode = Mode::FindingTask(TaskFinder {
            input: TextInput::default(),
            matches: Vec::new(),
            matched_query: String::new(),
            focused: 0,
            scroll: 0,
        });
    }

    // After a key or paste: a changed query finds its matches again and focuses the best one.
    // Cursor moves leave the query, and so the focused match, alone.
    pub(super) fn refresh_task_finder(&mut self) {
        let Some(finder) = self.mode.task_finder() else {
            return;
        };

        if finder.input.text() == finder.matched_query {
            return;
        }

        let query = finder.input.text().to_owned();
        let matches = finder_matches(&self.tasks, &query);

        if let Some(finder) = self.mode.task_finder_mut() {
            finder.matches = matches;
            finder.matched_query = query;
            finder.focused = 0;
            finder.scroll = 0;
        }
    }

    pub(super) fn handle_task_finder_key(&mut self, key: KeyEvent) {
        let page = usize::from(self.viewport.finder.results.height).max(1);

        let Some(finder) = self.mode.task_finder_mut() else {
            return;
        };

        match key.code {
            KeyCode::Esc => self.finish_task_finder(None),
            // As in search, an empty query has nothing to go to and just closes, and a query
            // without matches stays open so it can be fixed.
            KeyCode::Enter => {
                if finder.input.text().trim().is_empty() {
                    self.finish_task_finder(None);
                } else if let Some(&index) = finder.matches.get(finder.focused) {
                    self.finish_task_finder(Some(index));
                }
            }
            KeyCode::Tab | KeyCode::Down => finder.move_focus(true, None),
            KeyCode::BackTab | KeyCode::Up => finder.move_focus(false, None),
            KeyCode::PageDown => finder.move_focus(true, Some(page)),
            KeyCode::PageUp => finder.move_focus(false, Some(page)),
            code => edit_text_input(&mut finder.input, code),
        }
    }

    // Close the finder, and with a task, go to it: on the current tab when that shows it,
    // otherwise on the task's own tab. Like other jumps, `'` returns to where it started.
    // Going there is navigation only: nothing is saved now or added to undo history.
    fn finish_task_finder(&mut self, target: Option<usize>) {
        self.mode = Mode::Normal;

        let Some(index) = target else {
            return;
        };

        let from = self.position();
        let task = &self.tasks[index];

        if !view_includes_task(&self.view, task) {
            self.view = match &task.section {
                Some(section) => SectionView::Named(section.to_lowercase()),
                None => SectionView::Unsectioned,
            };
        }

        self.list.select(Some(index));
        self.viewport.reveal_selection = true;
        self.record_jump(from);
    }

    // The wheel moves the focus, and a click goes to the task under it.
    pub(super) fn handle_task_finder_mouse(&mut self, event: MouseEvent) {
        let viewport = &self.viewport.finder;
        let clicked = viewport
            .results
            .contains((event.column, event.row).into())
            .then(|| usize::from(event.row - viewport.results.y))
            .and_then(|row| viewport.task_by_row.get(row).copied());

        let Some(finder) = self.mode.task_finder_mut() else {
            return;
        };

        match event.kind {
            MouseEventKind::ScrollDown => finder.move_focus(true, Some(1)),
            MouseEventKind::ScrollUp => finder.move_focus(false, Some(1)),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(index) = clicked {
                    self.finish_task_finder(Some(index));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
