// The templates screen opened with `i`: named templates on top, the focused template's
// entries below. A template is added to the current tab as new tasks, and its entries are
// edited in place, saved to templates.json after every change. `I` opens the same screen to
// replace a section tab's tasks with a template instead.

use std::collections::VecDeque;

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;

use super::{
    AddPlacement, App, HISTORY_LIMIT, Mode, edit_text_input, repeats_while_typing, task_noun,
};
use crate::Result;
use crate::input::TextInput;
use crate::model::{SectionView, Task, TaskStatus, Template, input_task_body};
use crate::storage::Storage;

const EMPTY_TEMPLATE_NAME: &str = "Type a template name";

// What using a template does to the current tab.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TemplateAction {
    // Adds its entries as new tasks (`i`).
    Add,
    // Replaces the section's tasks, except carried ones, with its entries (`I`).
    Replace,
}

#[derive(Debug, PartialEq)]
pub struct TemplatePicker {
    pub(crate) action: TemplateAction,
    // Narrows the list to fuzzy name matches while it has text. Typed after `/`, and kept
    // when typing ends with Enter.
    pub(crate) filter: TextInput,
    // The focused template, as an index into `App::templates`. Kept among the filter's matches
    // whenever there are any; see `settle_template_selection`.
    pub(crate) selected: usize,
    pub(crate) focus: TemplateFocus,
    // The first name row and the first entry row shown. Drawing clamps both, and keeps the
    // focused name (and, while editing, the focused entry) in view.
    pub(crate) list_scroll: usize,
    pub(crate) entries_scroll: usize,
}

#[derive(Debug, PartialEq)]
pub enum TemplateFocus {
    List,
    Filtering,
    // Typing the name of a new template.
    Naming(TextInput),
    Renaming(TextInput),
    ConfirmDelete,
    Editing(TemplateEditor),
}

// Editing the focused template's entries, which works like a section without checkboxes.
#[derive(Debug, PartialEq)]
pub struct TemplateEditor {
    pub(crate) selected: Option<usize>,
    pub(crate) input: Option<(TextInput, EntryInput)>,
    // Undo steps for this editor only, dropped when it closes; the checklist's own history
    // never includes template edits.
    history: VecDeque<EditorSnapshot>,
    redo_history: VecDeque<EditorSnapshot>,
}

#[derive(Debug, PartialEq)]
struct EditorSnapshot {
    entries: Vec<String>,
    selected: Option<usize>,
    // Describes the change from this snapshot to the next state, e.g. "delete entry".
    label: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EntryInput {
    Add(AddPlacement),
    Edit(usize),
}

// Where the templates screen's panels were last drawn, so the mouse acts on what is on screen. Both
// row lists describe only the visible rows, top to bottom.
#[derive(Debug, Default)]
pub(crate) struct TemplateViewport {
    pub(crate) list: Rect,
    pub(crate) template_by_row: Vec<usize>,
    pub(crate) entries: Rect,
    pub(crate) entry_by_row: Vec<Option<usize>>,
}

// Templates whose names fuzzy-match `query`, best first, as indices into `templates`. An
// empty query keeps them all, most recently used first.
pub(crate) fn template_matches(templates: &[Template], query: &str) -> Vec<usize> {
    let names: Vec<&str> = templates
        .iter()
        .map(|template| template.name.as_str())
        .collect();

    crate::search::fuzzy_rank(query, &names)
}

// Where a new entry goes, like a new task: after the last entry, or beside the selected one.
pub(crate) fn entry_insertion_index(
    selected: Option<usize>,
    count: usize,
    placement: AddPlacement,
) -> usize {
    match placement {
        AddPlacement::End => count,
        AddPlacement::Below => selected.map_or(count, |index| index + 1),
        AddPlacement::Above => selected.unwrap_or(count),
    }
}

impl TemplatePicker {
    fn new(action: TemplateAction) -> Self {
        Self {
            action,
            filter: TextInput::default(),
            selected: 0,
            focus: TemplateFocus::List,
            list_scroll: 0,
            entries_scroll: 0,
        }
    }

    pub(crate) fn text_input_mut(&mut self) -> Option<&mut TextInput> {
        match &mut self.focus {
            TemplateFocus::Filtering => Some(&mut self.filter),
            TemplateFocus::Naming(input) | TemplateFocus::Renaming(input) => Some(input),
            TemplateFocus::Editing(editor) => editor.input.as_mut().map(|(input, _)| input),
            TemplateFocus::List | TemplateFocus::ConfirmDelete => None,
        }
    }

    // Whether letters are shortcuts here, as in Normal mode: in the list, and in the editor
    // while nothing is being typed.
    pub(crate) fn takes_shortcuts(&self) -> bool {
        match &self.focus {
            TemplateFocus::List => true,
            TemplateFocus::Editing(editor) => editor.input.is_none(),
            _ => false,
        }
    }

    // Like Normal mode, only moving and typing repeat while a key is held: never applying,
    // creating, deleting, confirming, or reordering.
    pub(crate) fn repeat_allowed(&self, key: KeyEvent) -> bool {
        let moving = key.modifiers.is_empty()
            && matches!(
                key.code,
                KeyCode::Char('j' | 'k')
                    | KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::PageUp
                    | KeyCode::PageDown
            );

        match &self.focus {
            TemplateFocus::List => moving,
            TemplateFocus::Filtering => {
                repeats_while_typing(key.code)
                    || matches!(
                        key.code,
                        KeyCode::Up | KeyCode::Down | KeyCode::Tab | KeyCode::BackTab
                    )
            }
            TemplateFocus::Naming(_) | TemplateFocus::Renaming(_) => repeats_while_typing(key.code),
            TemplateFocus::ConfirmDelete => false,
            TemplateFocus::Editing(editor) if editor.input.is_some() => {
                repeats_while_typing(key.code)
            }
            TemplateFocus::Editing(_) => {
                let half_page = key.modifiers == KeyModifiers::CONTROL
                    && matches!(key.code, KeyCode::Char('d' | 'D' | 'u' | 'U'));

                moving || half_page
            }
        }
    }

    fn select(&mut self, index: usize) {
        if self.selected != index {
            self.selected = index;
            self.entries_scroll = 0;
        }
    }
}

impl TemplateEditor {
    fn new(entry_count: usize) -> Self {
        Self {
            selected: (entry_count > 0).then_some(0),
            input: None,
            history: VecDeque::new(),
            redo_history: VecDeque::new(),
        }
    }

    // Like task selection, a single step wraps around the ends and a longer move stops there.
    fn move_selection(&mut self, entry_count: usize, forward: bool, by: Option<usize>) {
        if entry_count == 0 {
            return;
        }

        let last = entry_count - 1;
        let current = self.selected.unwrap_or(0).min(last);

        let next = match (by, forward) {
            (Some(by), true) => current.saturating_add(by).min(last),
            (Some(by), false) => current.saturating_sub(by),
            (None, true) if current == last => 0,
            (None, true) => current + 1,
            (None, false) if current == 0 => last,
            (None, false) => current - 1,
        };

        self.selected = Some(next);
    }
}

// The templates screen's state lives in the mode, so these borrow only `App::mode` and leave the
// templates themselves free to read alongside.
impl Mode {
    fn template_picker(&self) -> Option<&TemplatePicker> {
        match self {
            Mode::Templates(picker) => Some(picker),
            _ => None,
        }
    }

    fn template_picker_mut(&mut self) -> Option<&mut TemplatePicker> {
        match self {
            Mode::Templates(picker) => Some(picker),
            _ => None,
        }
    }

    fn template_editor_mut(&mut self) -> Option<&mut TemplateEditor> {
        match &mut self.template_picker_mut()?.focus {
            TemplateFocus::Editing(editor) => Some(editor),
            _ => None,
        }
    }
}

impl App {
    // Replacing needs a section to replace; on [*] and [-] it explains why nothing opened.
    pub(super) fn open_templates(&mut self, action: TemplateAction) {
        if action == TemplateAction::Replace && !matches!(self.view, SectionView::Named(_)) {
            self.show_important_message("Replacing works only on a section tab");

            return;
        }

        self.mode = Mode::Templates(TemplatePicker::new(action));
    }

    // The template the list focuses and the preview shows: none while nothing matches.
    pub(crate) fn focused_template(&self) -> Option<usize> {
        let picker = self.mode.template_picker()?;
        let matches = template_matches(&self.templates, picker.filter.text());

        matches
            .contains(&picker.selected)
            .then_some(picker.selected)
    }

    pub(super) fn template_filter(&self) -> Option<&str> {
        self.mode
            .template_picker()
            .map(|picker| picker.filter.text())
    }

    // After a key or paste: a changed filter focuses its best match, and a focus the filter
    // or a deletion left behind moves to the first match. Clearing the filter keeps the focus.
    pub(super) fn refresh_template_filter(&mut self, previous_filter: Option<&str>) {
        let Some(picker) = self.mode.template_picker() else {
            return;
        };

        let query = picker.filter.text();
        let matches = template_matches(&self.templates, query);
        let changed =
            !query.is_empty() && previous_filter.is_some_and(|previous| previous != query);
        let lost = !matches.contains(&picker.selected);

        if let Some(&best) = matches.first()
            && (changed || lost)
            && let Some(picker) = self.mode.template_picker_mut()
        {
            picker.select(best);
        }
    }

    // Saves a proposed template list, then makes it current. As with tasks, memory changes
    // only after the save succeeds, and a save error ends the session.
    fn commit_templates(&mut self, templates: Vec<Template>, storage: &Storage) -> Result<()> {
        storage.save_templates(&templates)?;
        self.templates = templates;

        Ok(())
    }

    pub(super) fn handle_templates_key(&mut self, key: KeyEvent, storage: &Storage) -> Result<()> {
        let Some(picker) = self.mode.template_picker() else {
            return Ok(());
        };

        match &picker.focus {
            TemplateFocus::List => self.handle_template_list_key(key, storage)?,
            TemplateFocus::Filtering => self.handle_template_filter_key(key),
            TemplateFocus::Naming(_) | TemplateFocus::Renaming(_) => {
                self.handle_template_name_key(key, storage)?
            }
            TemplateFocus::ConfirmDelete => match key.code {
                KeyCode::Enter => self.delete_template(storage)?,
                KeyCode::Esc => self.set_template_focus(TemplateFocus::List),
                _ => {}
            },
            TemplateFocus::Editing(editor) if editor.input.is_some() => {
                self.handle_entry_input_key(key, storage)?
            }
            TemplateFocus::Editing(_) => self.handle_template_editor_key(key, storage)?,
        }

        Ok(())
    }

    fn set_template_focus(&mut self, focus: TemplateFocus) {
        if let Some(picker) = self.mode.template_picker_mut() {
            picker.focus = focus;
        }
    }

    fn handle_template_list_key(&mut self, key: KeyEvent, storage: &Storage) -> Result<()> {
        let focused = self.focused_template();

        match key.code {
            // Esc first drops a kept filter, then closes.
            KeyCode::Esc => {
                if let Some(picker) = self.mode.template_picker_mut()
                    && !picker.filter.text().is_empty()
                {
                    picker.filter = TextInput::default();
                } else {
                    self.mode = Mode::Normal;
                }
            }
            KeyCode::Char('q' | 'i' | 'I') => self.mode = Mode::Normal,
            KeyCode::Char('j') | KeyCode::Down => self.move_template_selection(true),
            KeyCode::Char('k') | KeyCode::Up => self.move_template_selection(false),
            KeyCode::Char('g' | 'G') | KeyCode::Home | KeyCode::End => {
                let first = key.code == KeyCode::Home
                    || (key.code == KeyCode::Char('g')
                        && !key.modifiers.contains(KeyModifiers::SHIFT));
                let matches = template_matches(&self.templates, self.template_filter().unwrap());
                let target = if first {
                    matches.first()
                } else {
                    matches.last()
                };

                if let Some(&target) = target
                    && let Some(picker) = self.mode.template_picker_mut()
                {
                    picker.select(target);
                }
            }
            KeyCode::Char(' ') | KeyCode::Enter => self.apply_template(storage)?,
            KeyCode::Char('/') => self.set_template_focus(TemplateFocus::Filtering),
            KeyCode::Char('a') => {
                self.set_template_focus(TemplateFocus::Naming(TextInput::default()));
            }
            KeyCode::Char('E') => {
                if let Some(index) = focused {
                    let name = self.templates[index].name.as_str().into();
                    self.set_template_focus(TemplateFocus::Renaming(name));
                }
            }
            KeyCode::Char('e') => {
                if let Some(index) = focused {
                    let editor = TemplateEditor::new(self.templates[index].entries.len());
                    self.set_template_focus(TemplateFocus::Editing(editor));
                }
            }
            KeyCode::Char('d') | KeyCode::Delete if focused.is_some() => {
                self.set_template_focus(TemplateFocus::ConfirmDelete);
            }
            _ => {}
        }

        Ok(())
    }

    // The arrows and Tab step through matches while the filter is typed; Enter keeps it.
    fn handle_template_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                if let Some(picker) = self.mode.template_picker_mut() {
                    picker.filter = TextInput::default();
                    picker.focus = TemplateFocus::List;
                }
            }
            KeyCode::Enter => self.set_template_focus(TemplateFocus::List),
            KeyCode::Down | KeyCode::Tab => self.move_template_selection(true),
            KeyCode::Up | KeyCode::BackTab => self.move_template_selection(false),
            code => {
                if let Some(picker) = self.mode.template_picker_mut() {
                    edit_text_input(&mut picker.filter, code);
                }
            }
        }
    }

    // Moves the focus to the next or previous match, wrapping around like task selection.
    fn move_template_selection(&mut self, forward: bool) {
        let Some(picker) = self.mode.template_picker() else {
            return;
        };

        let matches = template_matches(&self.templates, picker.filter.text());

        if matches.is_empty() {
            return;
        }

        let count = matches.len();
        let current = matches
            .iter()
            .position(|&index| index == picker.selected)
            .unwrap_or(0);

        let next = if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };

        if let Some(picker) = self.mode.template_picker_mut() {
            picker.select(matches[next]);
        }
    }

    // Adds a pending task for each entry, as if each were typed on the current tab: on a
    // section tab into that section, after its last task; on [*] and [-] at the end of the
    // list, where an entry that starts with "Name: " joins that section like typed input.
    //
    // Replacing first removes the section's tasks, except carried ones, which stay as `D`
    // leaves them; the entries follow them, or take the place of the first removed task.
    fn apply_template(&mut self, storage: &Storage) -> Result<()> {
        let (Some(index), Some(picker)) = (self.focused_template(), self.mode.template_picker())
        else {
            return Ok(());
        };

        let action = picker.action;
        let template = self.templates[index].clone();

        if template.entries.is_empty() {
            self.show_routine_message("Template is empty");

            return Ok(());
        }

        let section = match &self.view {
            SectionView::Named(_) => self.scope_label(),
            SectionView::All | SectionView::Unsectioned => None,
        };

        let added: Vec<Task> = template
            .entries
            .iter()
            .map(|entry| match &section {
                Some(label) => Task::new(format!("{label}: {entry}")),
                None => Task::new(entry.as_str()),
            })
            .collect();
        let count = added.len();

        let (mut tasks, position, label, message) = match action {
            TemplateAction::Add => {
                let position = match &self.view {
                    SectionView::Named(key) => self
                        .tasks
                        .iter()
                        .rposition(|task| task.section_key().as_ref() == Some(key))
                        .map_or(self.tasks.len(), |last| last + 1),
                    SectionView::All | SectionView::Unsectioned => self.tasks.len(),
                };

                let label = format!("add template {}", template.name);
                let message = format!("Added {count} {} from {}", task_noun(count), template.name);

                (self.tasks.clone(), position, label, message)
            }
            TemplateAction::Replace => {
                // `open_templates` only replaces on a section tab.
                let Some(section) = section else {
                    return Ok(());
                };

                let mut survivors = Vec::new();
                let mut first_removed = None;
                let mut last_kept = None;
                let mut kept = 0;

                for (old_index, task) in self.tasks.iter().enumerate() {
                    let in_section = self.task_in_active_view(old_index);

                    if in_section && task.status != TaskStatus::Carried {
                        first_removed.get_or_insert(survivors.len());
                        continue;
                    }

                    if in_section {
                        last_kept = Some(survivors.len());
                        kept += 1;
                    }

                    survivors.push(task.clone());
                }

                let position = match (last_kept, first_removed) {
                    (Some(last), _) => last + 1,
                    (None, Some(first)) => first,
                    (None, None) => survivors.len(),
                };

                let label = format!("replace {section} with {}", template.name);
                let mut message = format!(
                    "Replaced {section} with {} ({count} {})",
                    template.name,
                    task_noun(count)
                );

                if kept > 0 {
                    message.push_str(&format!(", kept {kept} carried"));
                }

                (survivors, position, label, message)
            }
        };

        tasks.splice(position..position, added);
        self.commit_task_change(tasks, Some(position), label, storage)?;

        // The template just used moves to the top of the picker.
        let mut templates = self.templates.clone();
        let used = templates.remove(index);
        templates.insert(0, used);
        self.commit_templates(templates, storage)?;

        self.mode = Mode::Normal;
        self.show_routine_message(message);

        Ok(())
    }

    fn handle_template_name_key(&mut self, key: KeyEvent, storage: &Storage) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.set_template_focus(TemplateFocus::List),
            KeyCode::Enter => self.submit_template_name(storage)?,
            code => {
                if let Some(input) = self
                    .mode
                    .template_picker_mut()
                    .and_then(|p| p.text_input_mut())
                {
                    edit_text_input(input, code);
                }
            }
        }

        Ok(())
    }

    // Why the name being typed for a new or renamed template can't be used, if it can't. The
    // renamed template's own name in another case is fine: that only changes the spelling.
    pub(crate) fn template_name_problem(&self) -> Option<&'static str> {
        let picker = self.mode.template_picker()?;

        let (input, renamed) = match &picker.focus {
            TemplateFocus::Naming(input) => (input, None),
            TemplateFocus::Renaming(input) => (input, Some(picker.selected)),
            _ => return None,
        };

        let name = input.text().trim();

        if name.is_empty() {
            return Some(EMPTY_TEMPLATE_NAME);
        }

        let key = name.to_lowercase();
        let taken =
            self.templates.iter().enumerate().any(|(index, template)| {
                Some(index) != renamed && template.name.to_lowercase() == key
            });

        taken.then_some("Template name already taken")
    }

    // Shown on every key while naming, even with status messages off, because it explains
    // why Enter does nothing. An empty name is only explained on Enter, so the prompt doesn't
    // open with a complaint.
    pub(super) fn explain_template_name_problem(&mut self) {
        if let Some(problem) = self.template_name_problem()
            && problem != EMPTY_TEMPLATE_NAME
        {
            self.show_important_message(problem);
        }
    }

    fn submit_template_name(&mut self, storage: &Storage) -> Result<()> {
        if let Some(problem) = self.template_name_problem() {
            self.show_important_message(problem);

            return Ok(());
        }

        let Some(picker) = self.mode.template_picker() else {
            return Ok(());
        };

        let index = picker.selected;
        let (name, renaming) = match &picker.focus {
            TemplateFocus::Naming(input) => (input.text().trim().to_owned(), false),
            TemplateFocus::Renaming(input) => (input.text().trim().to_owned(), true),
            _ => return Ok(()),
        };

        if renaming {
            self.rename_template(index, name, storage)
        } else {
            self.create_template(name, storage)
        }
    }

    // A new template starts first in the list, with the filter cleared so it shows, and opens
    // in the editor ready for its first entry.
    fn create_template(&mut self, name: String, storage: &Storage) -> Result<()> {
        let mut templates = self.templates.clone();
        templates.insert(
            0,
            Template {
                name,
                entries: Vec::new(),
            },
        );

        self.commit_templates(templates, storage)?;

        if let Some(picker) = self.mode.template_picker_mut() {
            let mut editor = TemplateEditor::new(0);
            editor.input = Some((TextInput::default(), EntryInput::Add(AddPlacement::End)));

            picker.filter = TextInput::default();
            picker.selected = 0;
            picker.list_scroll = 0;
            picker.entries_scroll = 0;
            picker.focus = TemplateFocus::Editing(editor);
        }

        Ok(())
    }

    fn rename_template(&mut self, index: usize, name: String, storage: &Storage) -> Result<()> {
        if self.templates[index].name != name {
            let mut templates = self.templates.clone();
            templates[index].name = name.clone();
            self.commit_templates(templates, storage)?;
            self.show_routine_message(format!("Renamed template to {name}"));
        }

        self.set_template_focus(TemplateFocus::List);

        Ok(())
    }

    // Deleting can't be undone, which is why it asks first. The focus moves to the next
    // match, or the previous one at the end.
    fn delete_template(&mut self, storage: &Storage) -> Result<()> {
        let Some(index) = self.focused_template() else {
            self.set_template_focus(TemplateFocus::List);

            return Ok(());
        };

        let matches = template_matches(&self.templates, self.template_filter().unwrap());
        let position = matches.iter().position(|&found| found == index).unwrap();
        let neighbor = matches
            .get(position + 1)
            .or_else(|| position.checked_sub(1).map(|previous| &matches[previous]))
            .map(|&neighbor| {
                if neighbor > index {
                    neighbor - 1
                } else {
                    neighbor
                }
            });

        let mut templates = self.templates.clone();
        let deleted = templates.remove(index);
        self.commit_templates(templates, storage)?;

        if let Some(picker) = self.mode.template_picker_mut() {
            picker.select(neighbor.unwrap_or(0));
            picker.entries_scroll = 0;
            picker.focus = TemplateFocus::List;
        }

        self.show_routine_message(format!("Deleted template {}", deleted.name));

        Ok(())
    }

    fn handle_template_editor_key(&mut self, key: KeyEvent, storage: &Storage) -> Result<()> {
        let Some(index) = self.focused_template() else {
            return Ok(());
        };

        let count = self.templates[index].entries.len();
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let page = usize::from(self.viewport.templates.entries.height).max(1);

        let Some(editor) = self.mode.template_editor_mut() else {
            return Ok(());
        };

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.set_template_focus(TemplateFocus::List),
            KeyCode::Char('J') => self.move_entry(true, storage)?,
            KeyCode::Char('K') => self.move_entry(false, storage)?,
            KeyCode::Char('j') if shift => self.move_entry(true, storage)?,
            KeyCode::Char('k') if shift => self.move_entry(false, storage)?,
            KeyCode::Char('j') | KeyCode::Down => editor.move_selection(count, true, None),
            KeyCode::Char('k') | KeyCode::Up => editor.move_selection(count, false, None),
            KeyCode::PageDown => editor.move_selection(count, true, Some(page)),
            KeyCode::PageUp => editor.move_selection(count, false, Some(page)),
            KeyCode::Char('g' | 'G') | KeyCode::Home | KeyCode::End if count > 0 => {
                let first = key.code == KeyCode::Home || (key.code == KeyCode::Char('g') && !shift);
                editor.selected = Some(if first { 0 } else { count - 1 });
            }
            KeyCode::Char('a' | 'o' | 'O') => {
                let placement = match key.code {
                    KeyCode::Char('a') => AddPlacement::End,
                    KeyCode::Char('O') => AddPlacement::Above,
                    _ if shift => AddPlacement::Above,
                    _ => AddPlacement::Below,
                };

                editor.input = Some((TextInput::default(), EntryInput::Add(placement)));
            }
            KeyCode::Char('e') => {
                if let Some(selected) = editor.selected {
                    let text = self.templates[index].entries[selected].as_str().into();
                    editor.input = Some((text, EntryInput::Edit(selected)));
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => self.delete_entry(storage)?,
            KeyCode::Char('y') => self.yank_entry(),
            KeyCode::Char('p' | 'P') => {
                let above = key.code == KeyCode::Char('P') || shift;
                let placement = if above {
                    AddPlacement::Above
                } else {
                    AddPlacement::Below
                };

                self.paste_entry(placement, storage)?;
            }
            KeyCode::Char('u') => self.restore_template_snapshot(false, storage)?,
            _ => {}
        }

        Ok(())
    }

    // Control and Alt shortcuts in the editor, as in Normal mode. The other aliases, such as
    // Alt+E and Ctrl+O, already arrive as their plain keys from `normalize_key`.
    pub(super) fn handle_template_editor_chord(
        &mut self,
        key: KeyEvent,
        storage: &Storage,
    ) -> Result<()> {
        let Some(index) = self.focused_template() else {
            return Ok(());
        };

        let count = self.templates[index].entries.len();
        let half_page = usize::from(self.viewport.templates.entries.height / 2).max(1);

        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('z' | 'Z')) => {
                self.restore_template_snapshot(false, storage)?
            }
            (KeyModifiers::CONTROL, KeyCode::Char('r' | 'R')) => {
                self.restore_template_snapshot(true, storage)?
            }
            (KeyModifiers::CONTROL, KeyCode::Char('y' | 'Y')) => {
                self.paste_entry(AddPlacement::Below, storage)?
            }
            (KeyModifiers::ALT, KeyCode::Char('w' | 'W')) => self.yank_entry(),
            (KeyModifiers::CONTROL, KeyCode::Char('d' | 'D')) => {
                if let Some(editor) = self.mode.template_editor_mut() {
                    editor.move_selection(count, true, Some(half_page));
                }
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u' | 'U')) => {
                if let Some(editor) = self.mode.template_editor_mut() {
                    editor.move_selection(count, false, Some(half_page));
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_entry_input_key(&mut self, key: KeyEvent, storage: &Storage) -> Result<()> {
        let Some(editor) = self.mode.template_editor_mut() else {
            return Ok(());
        };

        match key.code {
            KeyCode::Esc => editor.input = None,
            KeyCode::Enter => self.submit_entry_input(storage)?,
            code => {
                if let Some((input, _)) = &mut editor.input {
                    edit_text_input(input, code);
                }
            }
        }

        Ok(())
    }

    // Enter in an entry input closes it, even when a blank or unchanged entry changes nothing.
    fn submit_entry_input(&mut self, storage: &Storage) -> Result<()> {
        let (Some(index), Some(editor)) =
            (self.focused_template(), self.mode.template_editor_mut())
        else {
            return Ok(());
        };

        let Some((input, kind)) = editor.input.take() else {
            return Ok(());
        };

        let text = input.text().trim().to_owned();

        if text.is_empty() {
            return Ok(());
        }

        match kind {
            EntryInput::Add(placement) => self.insert_entry(text, placement, "add entry", storage),
            EntryInput::Edit(position) => {
                let mut entries = self.templates[index].entries.clone();

                if entries[position] == text {
                    return Ok(());
                }

                entries[position] = text;
                self.commit_template_entries(entries, Some(position), "edit entry", storage)
            }
        }
    }

    fn insert_entry(
        &mut self,
        text: String,
        placement: AddPlacement,
        label: &'static str,
        storage: &Storage,
    ) -> Result<()> {
        let (Some(index), Some(editor)) =
            (self.focused_template(), self.mode.template_editor_mut())
        else {
            return Ok(());
        };

        let selected = editor.selected;
        let mut entries = self.templates[index].entries.clone();
        let position = entry_insertion_index(selected, entries.len(), placement);
        entries.insert(position, text);

        self.commit_template_entries(entries, Some(position), label, storage)
    }

    fn delete_entry(&mut self, storage: &Storage) -> Result<()> {
        let (Some(index), Some(editor)) =
            (self.focused_template(), self.mode.template_editor_mut())
        else {
            return Ok(());
        };

        let Some(selected) = editor.selected else {
            return Ok(());
        };

        let mut entries = self.templates[index].entries.clone();
        entries.remove(selected);
        let next = (!entries.is_empty()).then(|| selected.min(entries.len() - 1));

        self.commit_template_entries(entries, next, "delete entry", storage)
    }

    // Moves the selected entry one place; reordering doesn't wrap around the ends.
    fn move_entry(&mut self, down: bool, storage: &Storage) -> Result<()> {
        let (Some(index), Some(editor)) =
            (self.focused_template(), self.mode.template_editor_mut())
        else {
            return Ok(());
        };

        let Some(selected) = editor.selected else {
            return Ok(());
        };

        let mut entries = self.templates[index].entries.clone();
        let target = if down {
            selected + 1
        } else {
            selected.wrapping_sub(1)
        };

        if target >= entries.len() {
            return Ok(());
        }

        entries.swap(selected, target);

        self.commit_template_entries(entries, Some(target), "move entry", storage)
    }

    // Entries share the task register, so tasks and entries copy into each other.
    fn yank_entry(&mut self) {
        let (Some(index), Some(picker)) = (self.focused_template(), self.mode.template_picker())
        else {
            return;
        };

        let TemplateFocus::Editing(editor) = &picker.focus else {
            return;
        };

        if let Some(selected) = editor.selected {
            self.task_register = Some(self.templates[index].entries[selected].clone());
            self.show_routine_message("Copied entry");
        }
    }

    // A copied task brings its "Name: " prefix along. An entry keeps only the body, since the
    // tab a template is added to decides the section.
    fn paste_entry(&mut self, placement: AddPlacement, storage: &Storage) -> Result<()> {
        let Some(text) = self.task_register.clone() else {
            self.show_routine_message("Nothing to paste");

            return Ok(());
        };

        let body = input_task_body(&text).to_owned();

        self.insert_entry(body, placement, "paste entry", storage)
    }

    // Saves the focused template with `entries`, then records the editor's undo step. Like
    // the checklist's history, it is bounded, and a new change clears redo.
    fn commit_template_entries(
        &mut self,
        entries: Vec<String>,
        selected: Option<usize>,
        label: &'static str,
        storage: &Storage,
    ) -> Result<()> {
        let Some(index) = self.focused_template() else {
            return Ok(());
        };

        let mut templates = self.templates.clone();
        let previous = std::mem::replace(&mut templates[index].entries, entries);
        self.commit_templates(templates, storage)?;

        let Some(editor) = self.mode.template_editor_mut() else {
            return Ok(());
        };

        if editor.history.len() == HISTORY_LIMIT {
            editor.history.pop_front();
        }

        editor.history.push_back(EditorSnapshot {
            entries: previous,
            selected: editor.selected,
            label,
        });
        editor.redo_history.clear();
        editor.selected = selected;

        Ok(())
    }

    // Undo and redo in the editor save the restored entries before showing them.
    fn restore_template_snapshot(&mut self, redo: bool, storage: &Storage) -> Result<()> {
        let (Some(index), Some(editor)) =
            (self.focused_template(), self.mode.template_editor_mut())
        else {
            return Ok(());
        };

        let source = if redo {
            &editor.redo_history
        } else {
            &editor.history
        };

        let Some(snapshot) = source.back() else {
            self.show_routine_message(if redo {
                "Nothing to redo"
            } else {
                "Nothing to undo"
            });

            return Ok(());
        };

        let restored = snapshot.entries.clone();
        let mut templates = self.templates.clone();
        let current = std::mem::replace(&mut templates[index].entries, restored);
        self.commit_templates(templates, storage)?;

        let Some(editor) = self.mode.template_editor_mut() else {
            return Ok(());
        };

        let (source, destination) = if redo {
            (&mut editor.redo_history, &mut editor.history)
        } else {
            (&mut editor.history, &mut editor.redo_history)
        };

        // No eviction is needed, for the same reason as in the checklist's undo.
        let snapshot = source.pop_back().unwrap();
        destination.push_back(EditorSnapshot {
            entries: current,
            selected: editor.selected,
            label: snapshot.label,
        });
        editor.selected = snapshot.selected;

        let verb = if redo { "Redid" } else { "Undid" };
        self.show_routine_message(format!("{verb}: {}", snapshot.label));

        Ok(())
    }

    // The wheel moves through whichever panel it is over and a click focuses a name or an
    // entry; anything else, and the mouse while typing, does nothing.
    pub(super) fn handle_templates_mouse(&mut self, event: MouseEvent) {
        let position = (event.column, event.row).into();
        let viewport = &self.viewport.templates;

        let list_row = viewport
            .list
            .contains(position)
            .then(|| usize::from(event.row - viewport.list.y));
        let entry_row = viewport
            .entries
            .contains(position)
            .then(|| usize::from(event.row - viewport.entries.y));

        let clicked_template = list_row.and_then(|row| viewport.template_by_row.get(row).copied());
        let clicked_entry =
            entry_row.and_then(|row| viewport.entry_by_row.get(row).copied().flatten());

        let entry_count = self
            .focused_template()
            .map_or(0, |index| self.templates[index].entries.len());

        let Some(picker) = self.mode.template_picker_mut() else {
            return;
        };

        let wheel = match event.kind {
            MouseEventKind::ScrollDown => Some(true),
            MouseEventKind::ScrollUp => Some(false),
            _ => None,
        };
        let click = event.kind == MouseEventKind::Down(MouseButton::Left);

        match &mut picker.focus {
            TemplateFocus::List | TemplateFocus::Filtering => {
                if let Some(down) = wheel {
                    if list_row.is_some() {
                        self.move_template_selection(down);
                    } else if entry_row.is_some() {
                        // Drawing clamps the offset to the entries there are.
                        picker.entries_scroll = if down {
                            picker.entries_scroll.saturating_add(1)
                        } else {
                            picker.entries_scroll.saturating_sub(1)
                        };
                    }
                } else if click && let Some(index) = clicked_template {
                    picker.select(index);
                }
            }
            TemplateFocus::Editing(editor) if editor.input.is_none() => {
                if let Some(down) = wheel
                    && entry_row.is_some()
                {
                    editor.move_selection(entry_count, down, Some(1));
                } else if click && let Some(entry) = clicked_entry {
                    editor.selected = Some(entry);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
