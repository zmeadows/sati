use super::*;
use crate::model::TaskStatus;
use crate::status_message::StatusMessage;
use crate::storage::tests::Fixture;
use ratatui::crossterm::event::KeyEventKind;

fn template(name: &str, entries: &[&str]) -> Template {
    Template {
        name: name.to_owned(),
        entries: entries.iter().map(|entry| (*entry).to_owned()).collect(),
    }
}

// An app with `tasks` as typed and `templates`, both already saved, so changes can be
// compared with what is on disk.
fn saved_app(fixture: &Fixture, tasks: &[&str], templates: Vec<Template>) -> App {
    let mut app = App::new(tasks.iter().map(|text| Task::new(*text)).collect());
    app.templates = templates;

    fixture.storage.save(&app.tasks).unwrap();
    fixture.storage.save_templates(&app.templates).unwrap();

    app
}

fn press(app: &mut App, fixture: &Fixture, key: impl Into<KeyEvent>) {
    assert!(!app.handle_key(key.into(), &fixture.storage).unwrap());
}

fn type_text(app: &mut App, fixture: &Fixture, text: &str) {
    for character in text.chars() {
        press(app, fixture, KeyCode::Char(character));
    }
}

fn texts(app: &App) -> Vec<String> {
    app.tasks.iter().map(Task::full_text).collect()
}

fn names(templates: &[Template]) -> Vec<&str> {
    templates
        .iter()
        .map(|template| template.name.as_str())
        .collect()
}

fn notice(app: &App) -> Option<&str> {
    app.status_message.as_ref().map(StatusMessage::message)
}

fn picker(app: &App) -> &TemplatePicker {
    match &app.mode {
        Mode::Templates(picker) => picker,
        mode => panic!("expected the template picker, found {mode:?}"),
    }
}

fn editor(app: &App) -> &TemplateEditor {
    match &picker(app).focus {
        TemplateFocus::Editing(editor) => editor,
        focus => panic!("expected the template editor, found {focus:?}"),
    }
}

fn entries(app: &App) -> &[String] {
    &app.templates[picker(app).selected].entries
}

fn saved_entries(fixture: &Fixture, name: &str) -> Vec<String> {
    fixture
        .storage
        .load_templates()
        .unwrap()
        .into_iter()
        .find(|template| template.name == name)
        .unwrap()
        .entries
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
fn i_and_alt_i_open_and_close_templates_without_changing_anything() {
    let fixture = Fixture::new();
    let mut app = saved_app(&fixture, &["one"], vec![template("Daily", &["a"])]);

    press(&mut app, &fixture, KeyCode::Char('i'));
    assert_eq!(picker(&app).focus, TemplateFocus::List);
    assert_eq!(app.focused_template(), Some(0));

    press(&mut app, &fixture, KeyCode::Char('i'));
    assert_eq!(app.mode, Mode::Normal);

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('i'), KeyModifiers::ALT),
    );
    assert!(matches!(app.mode, Mode::Templates(_)));

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('i'), KeyModifiers::ALT),
    );
    assert_eq!(app.mode, Mode::Normal);

    for close in [KeyCode::Esc, KeyCode::Char('q')] {
        press(&mut app, &fixture, KeyCode::Char('i'));
        press(&mut app, &fixture, close);
        assert_eq!(app.mode, Mode::Normal);
    }

    assert_eq!(texts(&app), ["one"]);
    assert!(!app.can_undo());
}

#[test]
fn applying_on_a_section_tab_adds_after_its_last_task_as_one_undo_step() {
    let fixture = Fixture::new();
    let mut app = saved_app(
        &fixture,
        &["Home: dishes", "Work: email", "Home: laundry", "loose"],
        vec![
            template("Weekend", &["sleep"]),
            template("Daily", &["Stretch", "Plan tomorrow"]),
        ],
    );
    app.select_tab(SectionView::Named("home".to_owned()));

    press(&mut app, &fixture, KeyCode::Char('i'));
    press(&mut app, &fixture, KeyCode::Char('j'));
    press(&mut app, &fixture, KeyCode::Char(' '));

    assert_eq!(
        texts(&app),
        [
            "Home: dishes",
            "Work: email",
            "Home: laundry",
            "Home: Stretch",
            "Home: Plan tomorrow",
            "loose",
        ]
    );
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.list.selected(), Some(3));
    assert_eq!(app.view, SectionView::Named("home".to_owned()));
    assert!(
        app.tasks[3..5]
            .iter()
            .all(|task| task.status == TaskStatus::Pending)
    );
    assert_eq!(notice(&app), Some("Added 2 tasks from Daily"));

    // Both files hold the change, and the template used moves to the top.
    assert_eq!(fixture.storage.load().unwrap(), app.tasks);
    assert_eq!(names(&app.templates), ["Daily", "Weekend"]);
    assert_eq!(fixture.storage.load_templates().unwrap(), app.templates);

    press(&mut app, &fixture, KeyCode::Char('u'));
    assert_eq!(
        texts(&app),
        ["Home: dishes", "Work: email", "Home: laundry", "loose"]
    );
    assert_eq!(notice(&app), Some("Undid: add template Daily"));
}

#[test]
fn applying_on_all_or_unsectioned_appends_entries_as_typed() {
    for view in [SectionView::All, SectionView::Unsectioned] {
        let fixture = Fixture::new();
        let mut app = saved_app(
            &fixture,
            &["Home: dishes", "loose"],
            vec![template("Mixed", &["Pack bag", "Work: file report"])],
        );
        app.select_tab(view.clone());

        press(&mut app, &fixture, KeyCode::Char('i'));
        press(&mut app, &fixture, KeyCode::Enter);

        assert_eq!(
            texts(&app),
            ["Home: dishes", "loose", "Pack bag", "Work: file report"],
            "{view:?}"
        );
        assert_eq!(app.list.selected(), Some(2));
        assert_eq!(app.view, view);
    }
}

#[test]
fn an_empty_template_or_no_match_adds_nothing() {
    let fixture = Fixture::new();
    let mut app = saved_app(&fixture, &["one"], vec![template("Empty", &[])]);

    press(&mut app, &fixture, KeyCode::Char('i'));
    press(&mut app, &fixture, KeyCode::Enter);
    assert_eq!(notice(&app), Some("Template is empty"));
    assert!(matches!(app.mode, Mode::Templates(_)));

    press(&mut app, &fixture, KeyCode::Char('/'));
    type_text(&mut app, &fixture, "zzz");
    press(&mut app, &fixture, KeyCode::Enter);
    assert_eq!(app.focused_template(), None);
    press(&mut app, &fixture, KeyCode::Char(' '));

    assert_eq!(texts(&app), ["one"]);
    assert!(!app.can_undo());
}

#[test]
fn a_failed_save_leaves_tasks_and_templates_as_they_were() {
    let fixture = Fixture::new();
    let mut app = saved_app(&fixture, &["one"], vec![template("Daily", &["a"])]);
    let tasks = app.tasks.clone();
    let templates = app.templates.clone();

    press(&mut app, &fixture, KeyCode::Char('i'));
    block_saves(&fixture);

    let result = app.handle_key(KeyCode::Enter.into(), &fixture.storage);
    assert!(result.is_err());
    assert_eq!(app.tasks, tasks);
    assert_eq!(app.templates, templates);
    assert!(!app.can_undo());

    app.handle_key(KeyCode::Char('e').into(), &fixture.storage)
        .unwrap();
    let result = app.handle_key(KeyCode::Char('d').into(), &fixture.storage);
    assert!(result.is_err());
    assert_eq!(app.templates, templates);

    restore_saves(&fixture);
}

#[test]
fn the_list_is_most_recently_used_first_and_the_filter_ranks_fuzzily() {
    let fixture = Fixture::new();
    let mut app = saved_app(
        &fixture,
        &[],
        vec![
            template("Packing list", &["a"]),
            template("Daily", &["b"]),
            template("Daily (weekend)", &["c"]),
        ],
    );

    press(&mut app, &fixture, KeyCode::Char('i'));
    assert_eq!(app.focused_template(), Some(0));

    // Typing focuses the best match as the filter changes.
    press(&mut app, &fixture, KeyCode::Char('/'));
    type_text(&mut app, &fixture, "dai");
    assert_eq!(
        template_matches(&app.templates, picker(&app).filter.text()),
        [1, 2]
    );
    assert_eq!(app.focused_template(), Some(1));

    // Arrows, Tab, and Ctrl+N/P step through the matches while typing.
    press(&mut app, &fixture, KeyCode::Down);
    assert_eq!(app.focused_template(), Some(2));
    press(&mut app, &fixture, KeyCode::Tab);
    assert_eq!(app.focused_template(), Some(1));
    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
    );
    assert_eq!(app.focused_template(), Some(2));

    // Enter keeps the filter for j and k; Esc then clears it before closing.
    press(&mut app, &fixture, KeyCode::Enter);
    assert_eq!(picker(&app).focus, TemplateFocus::List);
    press(&mut app, &fixture, KeyCode::Char('j'));
    assert_eq!(app.focused_template(), Some(1));

    press(&mut app, &fixture, KeyCode::Esc);
    assert_eq!(picker(&app).filter.text(), "");
    assert_eq!(app.focused_template(), Some(1));

    press(&mut app, &fixture, KeyCode::Char('G'));
    assert_eq!(app.focused_template(), Some(2));
    press(&mut app, &fixture, KeyCode::Char('g'));
    assert_eq!(app.focused_template(), Some(0));

    // Esc while typing drops the filter.
    press(&mut app, &fixture, KeyCode::Char('/'));
    type_text(&mut app, &fixture, "pack");
    press(&mut app, &fixture, KeyCode::Esc);
    assert_eq!(picker(&app).filter.text(), "");

    press(&mut app, &fixture, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn pasting_into_the_filter_refocuses_the_best_match() {
    let fixture = Fixture::new();
    let mut app = saved_app(
        &fixture,
        &[],
        vec![template("Daily", &[]), template("Packing", &[])],
    );

    press(&mut app, &fixture, KeyCode::Char('i'));
    press(&mut app, &fixture, KeyCode::Char('/'));
    app.handle_paste("pack");

    assert_eq!(app.focused_template(), Some(1));
}

#[test]
fn a_new_template_needs_a_unique_name_and_opens_ready_for_entries() {
    let fixture = Fixture::new();
    let mut app = saved_app(&fixture, &[], vec![template("Daily", &["a"])]);

    press(&mut app, &fixture, KeyCode::Char('i'));
    press(&mut app, &fixture, KeyCode::Char('/'));
    type_text(&mut app, &fixture, "dai");
    press(&mut app, &fixture, KeyCode::Enter);

    press(&mut app, &fixture, KeyCode::Char('a'));
    assert!(matches!(picker(&app).focus, TemplateFocus::Naming(_)));
    assert_eq!(notice(&app), None);

    press(&mut app, &fixture, KeyCode::Enter);
    assert_eq!(notice(&app), Some("Type a template name"));

    type_text(&mut app, &fixture, "DAILY");
    assert_eq!(notice(&app), Some("Template name already taken"));
    press(&mut app, &fixture, KeyCode::Enter);
    assert!(matches!(picker(&app).focus, TemplateFocus::Naming(_)));

    for _ in 0.."DAILY".len() {
        press(&mut app, &fixture, KeyCode::Backspace);
    }

    type_text(&mut app, &fixture, " Weekend chores ");
    press(&mut app, &fixture, KeyCode::Enter);

    assert_eq!(names(&app.templates), ["Weekend chores", "Daily"]);
    assert_eq!(fixture.storage.load_templates().unwrap(), app.templates);
    assert_eq!(picker(&app).filter.text(), "");
    assert_eq!(app.focused_template(), Some(0));
    assert_eq!(
        editor(&app).input,
        Some((TextInput::default(), EntryInput::Add(AddPlacement::End)))
    );

    type_text(&mut app, &fixture, "Vacuum");
    press(&mut app, &fixture, KeyCode::Enter);
    assert_eq!(entries(&app), ["Vacuum"]);

    // Esc cancels naming without creating anything.
    press(&mut app, &fixture, KeyCode::Esc);
    press(&mut app, &fixture, KeyCode::Char('a'));
    type_text(&mut app, &fixture, "Other");
    press(&mut app, &fixture, KeyCode::Esc);
    assert_eq!(picker(&app).focus, TemplateFocus::List);
    assert_eq!(app.templates.len(), 2);
}

#[test]
fn renaming_keeps_the_place_and_may_change_only_the_case() {
    let fixture = Fixture::new();
    let mut app = saved_app(
        &fixture,
        &[],
        vec![template("Daily", &["a"]), template("Weekend", &["b"])],
    );

    press(&mut app, &fixture, KeyCode::Char('i'));
    press(&mut app, &fixture, KeyCode::Char('E'));
    assert_eq!(picker(&app).focus, TemplateFocus::Renaming("Daily".into()));

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    type_text(&mut app, &fixture, "weekend");
    assert_eq!(notice(&app), Some("Template name already taken"));
    press(&mut app, &fixture, KeyCode::Enter);
    assert!(matches!(picker(&app).focus, TemplateFocus::Renaming(_)));

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    type_text(&mut app, &fixture, "DAILY");
    press(&mut app, &fixture, KeyCode::Enter);

    assert_eq!(names(&app.templates), ["DAILY", "Weekend"]);
    assert_eq!(fixture.storage.load_templates().unwrap(), app.templates);
    assert_eq!(picker(&app).focus, TemplateFocus::List);
    assert_eq!(notice(&app), Some("Renamed template to DAILY"));
}

#[test]
fn deleting_asks_first_and_focuses_a_neighbor() {
    let fixture = Fixture::new();
    let mut app = saved_app(
        &fixture,
        &[],
        vec![
            template("One", &[]),
            template("Two", &[]),
            template("Three", &[]),
        ],
    );

    press(&mut app, &fixture, KeyCode::Char('i'));
    press(&mut app, &fixture, KeyCode::Char('j'));
    press(&mut app, &fixture, KeyCode::Char('d'));
    assert_eq!(picker(&app).focus, TemplateFocus::ConfirmDelete);

    // Other keys, including y, neither confirm nor cancel.
    press(&mut app, &fixture, KeyCode::Char('y'));
    press(&mut app, &fixture, KeyCode::Esc);
    assert_eq!(picker(&app).focus, TemplateFocus::List);
    assert_eq!(app.templates.len(), 3);

    press(&mut app, &fixture, KeyCode::Delete);
    press(&mut app, &fixture, KeyCode::Enter);
    assert_eq!(names(&app.templates), ["One", "Three"]);
    assert_eq!(fixture.storage.load_templates().unwrap(), app.templates);
    assert_eq!(app.focused_template(), Some(1));
    assert_eq!(notice(&app), Some("Deleted template Two"));

    // The last one falls back to the previous match.
    press(&mut app, &fixture, KeyCode::Char('d'));
    press(&mut app, &fixture, KeyCode::Enter);
    assert_eq!(names(&app.templates), ["One"]);
    assert_eq!(app.focused_template(), Some(0));

    press(&mut app, &fixture, KeyCode::Char('d'));
    press(&mut app, &fixture, KeyCode::Enter);
    assert!(app.templates.is_empty());
    assert_eq!(app.focused_template(), None);

    // With nothing focused, d does nothing.
    press(&mut app, &fixture, KeyCode::Char('d'));
    assert_eq!(picker(&app).focus, TemplateFocus::List);
}

#[test]
fn the_editor_adds_edits_moves_and_deletes_entries_saving_each_change() {
    let fixture = Fixture::new();
    let mut app = saved_app(&fixture, &["task"], vec![template("Daily", &["b", "d"])]);

    press(&mut app, &fixture, KeyCode::Char('i'));
    press(&mut app, &fixture, KeyCode::Char('e'));
    assert_eq!(editor(&app).selected, Some(0));

    // o adds below the selection, O above it, and a at the end.
    press(&mut app, &fixture, KeyCode::Char('o'));
    type_text(&mut app, &fixture, "c");
    press(&mut app, &fixture, KeyCode::Enter);
    assert_eq!(entries(&app), ["b", "c", "d"]);
    assert_eq!(editor(&app).selected, Some(1));

    press(&mut app, &fixture, KeyCode::Char('g'));
    press(&mut app, &fixture, KeyCode::Char('O'));
    type_text(&mut app, &fixture, "a");
    press(&mut app, &fixture, KeyCode::Enter);

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
    );
    press(&mut app, &fixture, KeyCode::Esc);
    assert_eq!(editor(&app).input, None);

    press(&mut app, &fixture, KeyCode::Char('a'));
    type_text(&mut app, &fixture, "  e  ");
    press(&mut app, &fixture, KeyCode::Enter);
    assert_eq!(entries(&app), ["a", "b", "c", "d", "e"]);
    assert_eq!(saved_entries(&fixture, "Daily"), ["a", "b", "c", "d", "e"]);

    // A blank entry adds nothing.
    press(&mut app, &fixture, KeyCode::Char('a'));
    press(&mut app, &fixture, KeyCode::Enter);
    assert_eq!(entries(&app).len(), 5);

    // Editing replaces the text; Alt+E is the alias.
    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::ALT),
    );
    assert_eq!(editor(&app).input, Some(("e".into(), EntryInput::Edit(4))));
    press(&mut app, &fixture, KeyCode::Backspace);
    type_text(&mut app, &fixture, "E");
    press(&mut app, &fixture, KeyCode::Enter);
    assert_eq!(entries(&app)[4], "E");

    // K and Alt+Up move the entry up; reordering stops at the ends.
    press(&mut app, &fixture, KeyCode::Char('K'));
    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Up, KeyModifiers::ALT),
    );
    assert_eq!(entries(&app), ["a", "b", "E", "c", "d"]);
    assert_eq!(editor(&app).selected, Some(2));
    press(&mut app, &fixture, KeyCode::Char('g'));
    press(&mut app, &fixture, KeyCode::Char('K'));
    assert_eq!(entries(&app)[0], "a");
    press(&mut app, &fixture, KeyCode::Char('J'));
    assert_eq!(entries(&app), ["b", "a", "E", "c", "d"]);

    // Deleting selects the next entry, or the previous one at the end.
    press(&mut app, &fixture, KeyCode::Char('d'));
    assert_eq!(entries(&app), ["b", "E", "c", "d"]);
    assert_eq!(editor(&app).selected, Some(1));
    press(&mut app, &fixture, KeyCode::Char('G'));
    press(&mut app, &fixture, KeyCode::Delete);
    assert_eq!(entries(&app), ["b", "E", "c"]);
    assert_eq!(editor(&app).selected, Some(2));

    // Space and Enter don't check anything off, and nothing reached the checklist.
    press(&mut app, &fixture, KeyCode::Char(' '));
    press(&mut app, &fixture, KeyCode::Enter);
    assert_eq!(saved_entries(&fixture, "Daily"), ["b", "E", "c"]);
    assert_eq!(texts(&app), ["task"]);
    assert!(!app.can_undo());

    press(&mut app, &fixture, KeyCode::Esc);
    assert_eq!(picker(&app).focus, TemplateFocus::List);
}

#[test]
fn editor_undo_and_redo_save_and_last_only_while_it_is_open() {
    let fixture = Fixture::new();
    let mut app = saved_app(&fixture, &[], vec![template("Daily", &["a", "b"])]);

    press(&mut app, &fixture, KeyCode::Char('i'));
    press(&mut app, &fixture, KeyCode::Char('e'));
    press(&mut app, &fixture, KeyCode::Char('j'));
    press(&mut app, &fixture, KeyCode::Char('d'));
    assert_eq!(entries(&app), ["a"]);

    press(&mut app, &fixture, KeyCode::Char('u'));
    assert_eq!(entries(&app), ["a", "b"]);
    assert_eq!(saved_entries(&fixture, "Daily"), ["a", "b"]);
    assert_eq!(editor(&app).selected, Some(1));
    assert_eq!(notice(&app), Some("Undid: delete entry"));

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    assert_eq!(entries(&app), ["a"]);
    assert_eq!(saved_entries(&fixture, "Daily"), ["a"]);
    assert_eq!(notice(&app), Some("Redid: delete entry"));

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
    );
    assert_eq!(entries(&app), ["a", "b"]);

    // A new change clears redo.
    press(&mut app, &fixture, KeyCode::Char('a'));
    type_text(&mut app, &fixture, "c");
    press(&mut app, &fixture, KeyCode::Enter);
    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    assert_eq!(notice(&app), Some("Nothing to redo"));
    assert_eq!(entries(&app), ["a", "b", "c"]);

    // Reopening the editor starts a fresh history.
    press(&mut app, &fixture, KeyCode::Esc);
    press(&mut app, &fixture, KeyCode::Char('e'));
    press(&mut app, &fixture, KeyCode::Char('u'));
    assert_eq!(notice(&app), Some("Nothing to undo"));
    assert_eq!(entries(&app), ["a", "b", "c"]);
}

#[test]
fn entries_and_tasks_copy_into_each_other_through_the_register() {
    let fixture = Fixture::new();
    let mut app = saved_app(
        &fixture,
        &["Work: file report"],
        vec![template("Daily", &["walk"])],
    );
    app.select_tab(SectionView::All);

    // A copied task loses its section in a template, since the tab decides it.
    press(&mut app, &fixture, KeyCode::Char('y'));
    press(&mut app, &fixture, KeyCode::Char('i'));
    press(&mut app, &fixture, KeyCode::Char('e'));
    press(&mut app, &fixture, KeyCode::Char('p'));
    assert_eq!(entries(&app), ["walk", "file report"]);
    press(&mut app, &fixture, KeyCode::Char('P'));
    assert_eq!(entries(&app), ["walk", "file report", "file report"]);
    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
    );
    assert_eq!(entries(&app).len(), 4);

    press(&mut app, &fixture, KeyCode::Char('g'));
    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT),
    );
    assert_eq!(notice(&app), Some("Copied entry"));

    press(&mut app, &fixture, KeyCode::Esc);
    press(&mut app, &fixture, KeyCode::Esc);
    press(&mut app, &fixture, KeyCode::Char('p'));
    assert_eq!(texts(&app), ["Work: file report", "walk"]);
}

#[test]
fn held_keys_move_but_never_apply_create_delete_or_reorder() {
    let fixture = Fixture::new();
    let mut app = saved_app(
        &fixture,
        &["task"],
        vec![template("One", &["a", "b"]), template("Two", &[])],
    );
    let held = |code| KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Repeat);

    press(&mut app, &fixture, KeyCode::Char('i'));
    press(&mut app, &fixture, held(KeyCode::Char('j')));
    assert_eq!(app.focused_template(), Some(1));

    for code in [KeyCode::Char('k'), KeyCode::Char(' '), KeyCode::Enter] {
        press(&mut app, &fixture, held(code));
    }

    assert_eq!(app.focused_template(), Some(0));
    assert_eq!(texts(&app), ["task"]);

    for code in [KeyCode::Char('a'), KeyCode::Char('d'), KeyCode::Char('e')] {
        press(&mut app, &fixture, held(code));
        assert_eq!(picker(&app).focus, TemplateFocus::List);
    }

    press(&mut app, &fixture, KeyCode::Char('e'));
    press(&mut app, &fixture, held(KeyCode::Char('j')));
    assert_eq!(editor(&app).selected, Some(1));

    for code in [KeyCode::Char('K'), KeyCode::Char('d'), KeyCode::Char('u')] {
        press(&mut app, &fixture, held(code));
    }

    assert_eq!(entries(&app), ["a", "b"]);

    // Typing repeats in an input.
    press(&mut app, &fixture, KeyCode::Char('a'));
    press(&mut app, &fixture, held(KeyCode::Char('x')));
    press(&mut app, &fixture, held(KeyCode::Char('x')));
    assert_eq!(
        editor(&app).input,
        Some(("xx".into(), EntryInput::Add(AddPlacement::End)))
    );
}

#[test]
fn quitting_with_templates_open_needs_closing_them_first() {
    let fixture = Fixture::new();
    let mut app = saved_app(&fixture, &[], vec![template("One", &[])]);

    press(&mut app, &fixture, KeyCode::Char('i'));
    press(&mut app, &fixture, KeyCode::Char('q'));
    assert_eq!(app.mode, Mode::Normal);

    assert!(
        app.handle_key(KeyCode::Char('q').into(), &fixture.storage)
            .unwrap()
    );
}
