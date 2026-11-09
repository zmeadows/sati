use super::*;
use crate::model::current_time;
use crate::status_message::DEFAULT_DURATION;
use crate::storage::tests::Fixture;

use unicode_segmentation::UnicodeSegmentation;
use uuid::Uuid;

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
    app.status_message.as_ref().map(StatusMessage::message)
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

    let tasks = app_with_tasks(3).tasks;
    fixture.storage.save(&tasks).unwrap();
    let original = std::fs::read(fixture.task_path()).unwrap();

    // Stands in for a document that a registered migration upgraded in memory.
    let mut document = fixture.storage.load_document().unwrap();
    document.needs_migration = true;
    let mut app = App::from_document(document);

    assert_eq!(std::fs::read(fixture.task_path()).unwrap(), original);

    block_saves(&fixture);
    assert!(app.save_selection(&fixture.storage).is_err());
    assert!(app.persisted.needs_migration);

    restore_saves(&fixture);
    app.save_selection(&fixture.storage).unwrap();
    assert!(!app.persisted.needs_migration && !app.can_undo());
    assert!(!fixture.storage.load_document().unwrap().needs_migration);

    // A clean quit writes the migrated, versioned document.
    let saved = std::fs::read_to_string(fixture.task_path()).unwrap();
    assert!(saved.contains("\"schema_version\": 5"), "{saved}");

    app.list.select(Some(2));
    assert_eq!(fixture.storage.load_document().unwrap().selected, Some(0));

    block_saves(&fixture);
    assert!(app.save_selection(&fixture.storage).is_err());
    assert_eq!(app.persisted.selected, Some(0));
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
    assert_eq!(app.persisted.selected, Some(1));
}

#[test]
fn ignored_modifier_keys_keep_the_focused_match_and_dismiss_status_messages() {
    let fixture = Fixture::new();

    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
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
            app.status_message = Some(StatusMessage::new("Copied task", DEFAULT_DURATION));
            press(
                &mut app,
                &fixture,
                KeyEvent::new_with_kind(KeyCode::Char('z'), modifiers, kind),
            );
            let accepted = kind != KeyEventKind::Release;

            assert_eq!(app.status_message.is_none(), accepted);
            assert_eq!(app.list.selected(), Some(2));
            assert!(matches!(&app.mode, Mode::Searching { input, .. } if input.text() == "hit"));
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
fn routine_and_important_status_messages_use_the_configured_duration() {
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
    let duration = Duration::from_millis(120);
    app.status_message_duration = duration;

    let expires_within_duration = |app: &App| {
        let message = app.status_message.as_ref().unwrap();
        let now = Instant::now();
        let wait = message.next_redraw(now).unwrap();

        wait <= duration && message.next_redraw(now + duration).is_none()
    };

    press(&mut app, &fixture, KeyCode::Char('w'));

    assert_eq!(notice(&app), Some("Wrap off"));
    assert!(expires_within_duration(&app));

    app.status_messages = false;
    app.copy_markdown(&mut BrokenOutput);

    assert!(notice(&app).is_some_and(|message| message.starts_with("Copy failed:")));
    assert!(expires_within_duration(&app));
}

#[test]
fn disabled_status_messages_hide_confirmations_but_still_report_copy_failures() {
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
    app.status_messages = false;
    let theme = app.theme;

    press(&mut app, &fixture, KeyCode::Char('y'));
    assert_eq!(app.task_register.as_deref(), Some("task 0"));
    assert!(app.status_message.is_none());

    press(&mut app, &fixture, KeyCode::Char('t'));
    assert_ne!(app.theme, theme);
    assert!(app.status_message.is_none());

    let mut output = Vec::new();
    app.copy_markdown(&mut output);
    let mut expected = Vec::new();
    crate::export::copy(&app.tasks, &mut expected).unwrap();

    assert_eq!(output, expected);
    assert!(app.status_message.is_none());

    for key in ['u', 'w', 'd', 'X', 'p'] {
        press(&mut app, &fixture, KeyCode::Char(key));
        assert!(app.status_message.is_none(), "{key}");
    }

    app.copy_markdown(&mut BrokenOutput);

    assert!(
        app.status_message
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
    app.viewport.scroll = 2;
    let mut output = Vec::new();
    app.copy_markdown(&mut output);
    let mut expected = Vec::new();
    crate::export::copy(&tasks, &mut expected).unwrap();

    assert_eq!(output, expected);
    assert_eq!(
        app.status_message.as_ref().map(StatusMessage::message),
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
            app.status_message.as_ref().map(StatusMessage::message),
            Some("Copy sent to terminal")
        );
    }

    app.copy_markdown(&mut BrokenOutput);
    assert_eq!(
        app.status_message.as_ref().map(StatusMessage::message),
        Some("Copy failed: output unavailable")
    );
    assert_eq!(app.tasks, tasks);
    assert_eq!(app.list.selected(), selected);
    assert_eq!(app.viewport.scroll, 2);
    assert_eq!(app.history.clone(), undo);
    assert_eq!(app.redo_history.clone(), redo);
    assert_eq!(fixture.storage.load().unwrap(), saved);

    press(&mut app, &fixture, KeyCode::Down);
    assert!(app.status_message.is_none());
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
        Mode::RenamingSection { input: "".into() },
        Mode::ConfirmClear,
        Mode::Help {
            scroll: 0,
            max_scroll: 0,
            page: 0,
        },
    ] {
        app.mode = mode;
        press(&mut app, &fixture, KeyCode::Char('c'));

        match &app.mode {
            Mode::Adding(input, _)
            | Mode::Editing { input, .. }
            | Mode::Searching { input, .. }
            | Mode::RenamingSection { input }
            | Mode::FuzzySection { input, .. } => assert_eq!(input.text(), "c"),
            Mode::ConfirmClear | Mode::Help { .. } => {}
            Mode::Normal => panic!("copy changed mode"),
        }

        assert!(app.status_message.is_none());
        assert_eq!(app.tasks, tasks);
        assert!(!app.can_undo() && !app.can_redo());
    }

    assert!(fixture.storage.load().unwrap().is_empty());
}

#[test]
fn task_register_copies_literal_text_and_pastes_unmarked_with_shared_placement() {
    let fixture = Fixture::new();
    let text = "Work: *日本語* [literal]";

    for status in [
        TaskStatus::Pending,
        TaskStatus::Completed,
        TaskStatus::Carried,
    ] {
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
                    status,
                    ..Task::new(text)
                };
                app.list.select(Some(2));
                fixture.storage.save(&app.tasks).unwrap();
                let before = app.tasks.clone();
                press(&mut app, &fixture, copy);

                assert_eq!(
                    app.status_message.as_ref().map(StatusMessage::message),
                    Some("Copied task")
                );
                assert_eq!(app.tasks, before);
                assert_eq!(fixture.storage.load().unwrap(), before);
                assert!(!app.can_undo() && !app.can_redo());

                let index = app.insertion_index(placement);
                press(&mut app, &fixture, paste);
                assert!(app.status_message.is_none());

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
                assert!(app.viewport.reveal_selection);
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

    assert!(app.status_message.is_none());
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
            app.status_message.as_ref().unwrap().message(),
            format!("Theme: {}", expected.id())
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

    assert_eq!(app.theme, BuiltinTheme::ModusVivendiTritanopia);

    press(&mut app, &fixture, KeyCode::Char('t'));
    assert_eq!(app.theme, BuiltinTheme::Default);

    for &(theme, id) in BuiltinTheme::ALL {
        for forward in [false, true] {
            app.theme = theme.cycle(!forward);
            press(
                &mut app,
                &fixture,
                KeyCode::Char(if forward { 't' } else { 'T' }),
            );

            assert_eq!(app.theme, theme);
            assert_eq!(
                app.status_message.as_ref().unwrap().message(),
                format!("Theme: {id}")
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
        Mode::ConfirmClear,
    ] {
        app.mode = mode;
        press(&mut app, &fixture, KeyCode::Char('t'));
        press(&mut app, &fixture, KeyCode::Char('T'));

        assert_eq!(app.theme, crate::themes::BuiltinTheme::Default);
        assert!(app.status_message.is_none());

        match &app.mode {
            Mode::Adding(input, _)
            | Mode::Editing { input, .. }
            | Mode::Searching { input, .. } => assert_eq!(input.text(), "tT"),
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

    let released = KeyEvent::new_with_kind(KeyCode::Tab, KeyModifiers::NONE, KeyEventKind::Release);
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
fn counted_section_switches_stop_at_the_ends_while_plain_ones_wrap() {
    let fixture = Fixture::new();
    // Tabs: [*], [-], Errands, Home, Work.
    let mut app = app_with_texts(&["Work: a", "Home: b", "Errands: c", "plain"]);

    type_keys(&mut app, &fixture, "3l");
    assert_eq!(app.view, SectionView::Named("home".into()));

    type_keys(&mut app, &fixture, "9l");
    assert_eq!(app.view, SectionView::Named("work".into()));

    type_keys(&mut app, &fixture, "2h");
    assert_eq!(app.view, SectionView::Named("errands".into()));

    type_keys(&mut app, &fixture, "9h");
    assert_eq!(app.view, SectionView::All);

    // The count was consumed, so a plain motion wraps again.
    type_keys(&mut app, &fixture, "h");
    assert_eq!(app.view, SectionView::Named("work".into()));

    assert!(!app.can_undo());
    assert!(!fixture.task_path().exists());
}

#[test]
fn held_section_switches_repeat_but_held_section_moves_do_not() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: a", "Home: b", "plain"]);

    repeat(&mut app, &fixture, KeyCode::Char('f'), KeyModifiers::ALT);
    repeat(&mut app, &fixture, KeyCode::Char('f'), KeyModifiers::ALT);
    assert_eq!(app.view, SectionView::Named("home".into()));

    repeat(&mut app, &fixture, KeyCode::Char('b'), KeyModifiers::ALT);
    assert_eq!(app.view, SectionView::Unsectioned);

    app.show_section(SectionView::Named("home".into()));

    for (code, modifiers) in [
        (KeyCode::Char('L'), KeyModifiers::SHIFT),
        (KeyCode::Char('n'), KeyModifiers::ALT),
    ] {
        repeat(&mut app, &fixture, code, modifiers);
    }

    assert_eq!(app.section_order, ["home", "work"]);
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
fn tab_and_control_n_p_step_through_picker_matches_and_wrap() {
    let fixture = Fixture::new();
    let ctrl = |character| KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL);

    // From [*], nothing is focused yet: Tab starts at the first match, Shift+Tab at the
    // last.
    let mut app = picker_app();
    press(&mut app, &fixture, KeyCode::Char('`'));

    for (key, view) in [
        (KeyEvent::from(KeyCode::Tab), SectionView::Unsectioned),
        (KeyEvent::from(KeyCode::BackTab), named("work")),
        (KeyEvent::from(KeyCode::Tab), SectionView::Unsectioned),
        (ctrl('n'), named("home")),
        (ctrl('p'), SectionView::Unsectioned),
        (ctrl('p'), named("work")),
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
fn control_arrows_move_by_word_in_the_picker_query() {
    let fixture = Fixture::new();
    let mut app = picker_app();
    press(&mut app, &fixture, KeyCode::Char('`'));
    app.paste("wo rk");

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL),
    );
    press(&mut app, &fixture, KeyCode::Backspace);

    let Mode::FuzzySection { input, .. } = &app.mode else {
        panic!("expected the section picker");
    };
    assert_eq!(input.text(), "work");

    press(&mut app, &fixture, KeyCode::Char('x'));
    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL),
    );
    press(&mut app, &fixture, KeyCode::Backspace);

    let Mode::FuzzySection { input, .. } = &app.mode else {
        panic!("expected the section picker");
    };
    assert_eq!(input.text(), "woxr");
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
        app.viewport.scroll = 2;
        app.viewport.tab_scroll = 1;
        app.viewport.reveal_selection = false;

        press(&mut app, &fixture, KeyCode::Char('`'));
        type_keys(&mut app, &fixture, "wo");
        press(&mut app, &fixture, KeyCode::Tab);
        app.viewport.scroll = 0;
        app.viewport.tab_scroll = 0;

        press(&mut app, &fixture, cancel);

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.view, SectionView::All);
        assert_eq!(app.list.selected(), Some(3));
        assert_eq!((app.viewport.scroll, app.viewport.tab_scroll), (2, 1));
        assert!(!app.viewport.reveal_selection);
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
fn enter_waits_for_a_focused_match_when_the_picker_opens_on_all() {
    let fixture = Fixture::new();
    let mut app = picker_app();
    press(&mut app, &fixture, KeyCode::Char('`'));

    press(&mut app, &fixture, KeyCode::Enter);

    assert!(matches!(app.mode, Mode::FuzzySection { .. }));
    assert_eq!(app.view, SectionView::All);

    press(&mut app, &fixture, KeyCode::Tab);
    press(&mut app, &fixture, KeyCode::Enter);

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view, SectionView::Unsectioned);
    assert!(!app.can_undo());
    assert!(!fixture.task_path().exists());
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
    app.viewport.row_tasks = vec![Some(0), Some(1)];
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
fn a_counted_section_move_is_one_saved_and_undoable_change_that_stops_at_the_ends() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: a", "Home: b", "Errands: c", "Gym: d"]);
    app.show_section(SectionView::Named("errands".into()));

    type_keys(&mut app, &fixture, "2L");

    assert_eq!(app.section_order, ["gym", "home", "errands", "work"]);
    assert_eq!(app.history.len(), 1);
    assert_eq!(
        fixture.storage.load_document().unwrap().section_order,
        app.section_order
    );

    type_keys(&mut app, &fixture, "9L");
    assert_eq!(app.section_order, ["gym", "home", "work", "errands"]);

    type_keys(&mut app, &fixture, "3H");
    assert_eq!(app.section_order, ["errands", "gym", "home", "work"]);
    assert_eq!(app.view, SectionView::Named("errands".into()));
    assert_eq!(app.history.len(), 3);

    // Already first, so a counted move left is a no-op that leaves history alone.
    type_keys(&mut app, &fixture, "2H");
    assert_eq!(app.history.len(), 3);

    type_keys(&mut app, &fixture, "u");
    assert_eq!(app.section_order, ["gym", "home", "work", "errands"]);
    assert_eq!(notice(&app), Some("Undid: move section"));

    type_keys(&mut app, &fixture, "uu");
    assert_eq!(app.section_order, ["errands", "gym", "home", "work"]);

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    assert_eq!(app.section_order, ["gym", "home", "errands", "work"]);
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
        app.tasks[index].status = TaskStatus::Completed;
    }
    app.show_section(SectionView::Named("work".into()));

    press(&mut app, &fixture, KeyCode::Char('X'));

    assert_eq!(texts(&app), ["Home: b", "Work: c"]);
    assert_eq!(app.list.selected(), Some(1));
    assert_eq!(app.view, SectionView::Named("work".into()));
    assert_eq!(notice(&app), Some("Cleared 2 completed Work tasks"));

    press(&mut app, &fixture, KeyCode::Char('X'));
    assert_eq!(notice(&app), Some("No completed Work tasks"));

    press(&mut app, &fixture, KeyCode::Char('u'));
    assert_eq!(notice(&app), Some("Undid: clear 2 completed Work tasks"));
    assert_eq!(app.tasks.len(), 4);

    app.show_section(SectionView::All);
    press(&mut app, &fixture, KeyCode::Char('X'));
    assert_eq!(texts(&app), ["Work: c"]);
    assert_eq!(notice(&app), Some("Cleared 3 completed tasks"));
}

#[test]
fn clearing_unmarked_in_a_section_keeps_other_sections_and_carried_tasks() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: a", "Home: b", "Work: c", "Work: d"]);
    app.tasks[2].status = TaskStatus::Carried;
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
        Mode::Adding(input, _) | Mode::Editing { input, .. } | Mode::RenamingSection { input } => {
            input.text()
        }
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
fn space_or_enter_accept_a_suggestion_and_backspace_dismisses_it() {
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

    press(&mut app, &fixture, KeyCode::Char('a'));
    press(&mut app, &fixture, KeyCode::Backspace);

    assert_eq!(input_text(&app), "");
    assert_eq!(app.completion_ghost(), None);

    press(&mut app, &fixture, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal);

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
fn esc_and_ctrl_g_cancel_an_add_or_edit_even_while_a_suggestion_shows() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: a", "Home: b"]);
    app.show_section(SectionView::Named("work".into()));
    let tasks = app.tasks.clone();
    let selected = app.list.selected();

    for cancel in [
        KeyEvent::from(KeyCode::Esc),
        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL),
    ] {
        for start in ['o', 'e'] {
            press(&mut app, &fixture, KeyCode::Char(start));

            // Editing erases the text first so a suggestion can show.
            if start == 'e' {
                press(
                    &mut app,
                    &fixture,
                    KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
                );
                press(&mut app, &fixture, KeyCode::Tab);
            }

            assert!(app.completion_ghost().is_some(), "{start}");

            press(&mut app, &fixture, cancel);

            assert_eq!(app.mode, Mode::Normal, "{cancel:?} after {start}");
            assert_eq!(app.completion_ghost(), None);
        }
    }

    assert_eq!(app.tasks, tasks);
    assert_eq!(app.list.selected(), selected);
    assert!(!app.can_undo() && !app.can_redo());
    assert!(!fixture.task_path().exists());
}

#[test]
fn control_n_and_p_cycle_section_suggestions_like_tab_without_accepting() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: a", "Home: b", "Errands: c"]);
    press(&mut app, &fixture, KeyCode::Char('a'));
    let ctrl = |character| KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL);

    for (key, ghost) in [
        (ctrl('n'), "Errands: "),
        (ctrl('n'), "Home: "),
        (ctrl('p'), "Errands: "),
        (ctrl('p'), "Work: "),
    ] {
        press(&mut app, &fixture, key);

        assert_eq!(app.completion_ghost().as_deref(), Some(ghost), "{key:?}");
        assert_eq!(input_text(&app), "");
    }

    // Held, Ctrl+N keeps cycling like a held Tab.
    repeat(
        &mut app,
        &fixture,
        KeyCode::Char('n'),
        KeyModifiers::CONTROL,
    );
    assert_eq!(app.completion_ghost().as_deref(), Some("Errands: "));

    assert_eq!(app.tasks.len(), 3);
    assert!(!app.can_undo());
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

    press(&mut app, &fixture, KeyCode::Backspace);
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
fn the_rename_problem_is_shown_even_with_status_messages_off() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: a", "Home: b"]);
    app.status_messages = false;
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
                        task.status = match i {
                            1 | 3 => TaskStatus::Completed,
                            2 => TaskStatus::Carried,
                            _ => TaskStatus::Pending,
                        };
                    }

                    app.list.select(selected);

                    app
                };

                let mut actual = setup();
                let mut expected = setup();
                let original = actual.tasks.clone();
                press(
                    &mut actual,
                    &fixture,
                    KeyEvent::new_with_kind(KeyCode::Char(alias), KeyModifiers::CONTROL, kind),
                );
                press(
                    &mut expected,
                    &fixture,
                    KeyEvent::new_with_kind(arrow, KeyModifiers::NONE, kind),
                );

                assert_eq!(actual.list.selected(), expected.list.selected());
                assert_eq!(actual.mode, expected.mode);

                let mut terminal =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(24, 6)).unwrap();
                let mut rows = Rect::default();
                terminal
                    .draw(|frame| rows = crate::ui::draw(frame, &mut actual))
                    .unwrap();

                if let Some(index) = actual.list.selected() {
                    assert!(
                        actual.viewport.row_tasks[actual.viewport.scroll
                            ..(actual.viewport.scroll + usize::from(rows.height))
                                .min(actual.viewport.row_tasks.len())]
                            .contains(&Some(index))
                    );
                }

                assert_eq!(actual.tasks, original);
                assert!(!actual.can_undo() && !actual.can_redo());
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
                        let mut input = TextInput::from(text);
                        input.set_cursor(cursor);
                        let mut expected = input.clone();

                        if kind != KeyEventKind::Release {
                            match command {
                                'k' => expected.delete_to_end(),
                                'u' => expected.delete_to_start(),
                                'w' => expected.delete_word_before(),
                                'b' if modifiers == KeyModifiers::ALT => expected.move_word_left(),
                                'b' => expected.move_left(),
                                'f' if modifiers == KeyModifiers::ALT => expected.move_word_right(),
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
        Mode::ConfirmClear,
    ] {
        let search = matches!(mode, Mode::Searching { .. });
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
fn tab_and_shift_tab_cycle_visible_matches_with_wrapping_while_typing() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["hit 0", "miss 1", "hit 2", "miss 3", "hit 4"]);
    app.list.select(Some(1));

    press(&mut app, &fixture, KeyCode::Char('/'));
    app.paste("hit");

    assert_eq!(app.list.selected(), Some(2));

    for expected in [4, 0, 2] {
        press(&mut app, &fixture, KeyCode::Tab);
        assert_eq!(app.list.selected(), Some(expected));
    }

    for expected in [0, 4] {
        press(&mut app, &fixture, KeyCode::BackTab);
        assert_eq!(app.list.selected(), Some(expected));
    }

    repeat(&mut app, &fixture, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(app.list.selected(), Some(0));

    assert_eq!(app.search_query(), Some("hit"));
}

#[test]
fn control_n_and_p_step_through_search_results_like_tab_and_esc_still_restores() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["hit 0", "miss 1", "hit 2", "miss 3", "hit 4"]);
    app.list.select(Some(1));
    let ctrl = |character| KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL);

    press(&mut app, &fixture, KeyCode::Char('/'));
    app.paste("hit");

    for (key, expected) in [(ctrl('n'), 4), (ctrl('n'), 0), (ctrl('p'), 4)] {
        press(&mut app, &fixture, key);

        assert_eq!(app.list.selected(), Some(expected), "{key:?}");
    }

    assert_eq!(app.search_query(), Some("hit"));

    press(&mut app, &fixture, KeyCode::Esc);

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.list.selected(), Some(1));
    assert!(!app.can_undo());
}

#[test]
fn moving_the_search_cursor_keeps_the_match_chosen_with_tab_until_the_query_changes() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["hit 0", "miss 1", "hit 2", "hit 3"]);
    app.list.select(Some(1));

    press(&mut app, &fixture, KeyCode::Char('/'));
    app.paste("hit");
    press(&mut app, &fixture, KeyCode::Tab);

    assert_eq!(app.list.selected(), Some(3));

    for key in [
        KeyEvent::from(KeyCode::Left),
        KeyEvent::from(KeyCode::Right),
        KeyEvent::from(KeyCode::Home),
        KeyEvent::from(KeyCode::End),
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT),
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
        KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
        KeyEvent::from(KeyCode::PageDown),
    ] {
        press(&mut app, &fixture, key);
        assert_eq!(app.list.selected(), Some(3), "{key:?}");
    }

    app.paste("");
    assert_eq!(app.list.selected(), Some(3));

    // Editing the query starts again from the task selected before searching.
    press(&mut app, &fixture, KeyCode::Char(' '));
    assert_eq!(app.list.selected(), Some(2));
    assert_eq!(app.search_query(), Some("hit "));
}

#[test]
fn enter_selects_the_focused_match_and_esc_restores_the_original_selection() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["hit 0", "miss 1", "hit 2"]);
    let original = task_state(&app);

    press(&mut app, &fixture, KeyCode::Char('/'));
    app.paste("hit");
    press(&mut app, &fixture, KeyCode::Tab);
    press(&mut app, &fixture, KeyCode::Esc);

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(task_state(&app), original);

    press(&mut app, &fixture, KeyCode::Char('/'));
    app.paste("hit");
    press(&mut app, &fixture, KeyCode::Tab);
    press(&mut app, &fixture, KeyCode::Enter);

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.search_query(), None);
    assert_eq!(app.list.selected(), Some(2));
    assert_eq!(app.tasks, original.tasks);
    assert!(!app.can_undo() && !app.can_redo());
    assert!(!fixture.task_path().exists());
}

#[test]
fn enter_keeps_search_open_without_matches_and_cancels_an_empty_query() {
    let fixture = Fixture::new();
    let mut app = app_with_tasks(2);
    app.list.select(Some(1));

    press(&mut app, &fixture, KeyCode::Char('/'));
    app.paste("missing");
    press(&mut app, &fixture, KeyCode::Tab);
    press(&mut app, &fixture, KeyCode::Enter);

    assert_eq!(app.search_query(), Some("missing"));
    assert_eq!(app.list.selected(), None);

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    press(&mut app, &fixture, KeyCode::Enter);

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.list.selected(), Some(1));
}

#[test]
fn n_and_shift_n_do_nothing_in_normal_mode_after_a_search() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["hit 0", "miss 1", "hit 2"]);
    press(&mut app, &fixture, KeyCode::Char('/'));
    app.paste("hit");
    press(&mut app, &fixture, KeyCode::Enter);
    let before = task_state(&app);

    for (code, modifiers) in [
        (KeyCode::Char('n'), KeyModifiers::NONE),
        (KeyCode::Char('N'), KeyModifiers::SHIFT),
    ] {
        press(&mut app, &fixture, KeyEvent::new(code, modifiers));

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(task_state(&app), before);
        assert!(app.status_message.is_none());
    }
}

// Each way to remove the last tasks of a tab, as keys pressed in Normal mode.
const TAB_EMPTYING_KEYS: [&[KeyCode]; 3] = [
    &[KeyCode::Char('d')],
    &[KeyCode::Char('X')],
    &[KeyCode::Char('D'), KeyCode::Enter],
];

fn app_on_a_tab_with_one_completed_task() -> App {
    let mut app = app_with_texts(&["Home: a", "Work: b"]);
    app.tasks[1].status = TaskStatus::Completed;
    app.show_section(SectionView::Named("work".into()));

    app
}

fn saved_view(fixture: &Fixture) -> SectionView {
    App::from_document(fixture.storage.load_document().unwrap()).view
}

#[test]
fn failed_saves_that_would_empty_a_tab_keep_its_view_tasks_and_history() {
    for keys in TAB_EMPTYING_KEYS {
        let fixture = Fixture::new();
        let mut app = app_on_a_tab_with_one_completed_task();
        fixture.storage.save(&app.tasks).unwrap();
        let before = task_state(&app);
        block_saves(&fixture);

        let (last, setup) = keys.split_last().unwrap();

        for &key in setup {
            press(&mut app, &fixture, key);
        }

        let result = app.key(KeyEvent::from(*last), &fixture.storage);

        assert!(result.is_err(), "{keys:?}");
        assert_eq!(app.view, SectionView::Named("work".into()), "{keys:?}");
        assert_eq!(app.persisted.view, SectionView::All, "{keys:?}");
        assert_eq!(task_state(&app), before, "{keys:?}");
    }
}

#[test]
fn undo_returns_to_the_tab_a_removal_emptied_and_redo_leaves_it_again() {
    for keys in TAB_EMPTYING_KEYS {
        let fixture = Fixture::new();
        let mut app = app_on_a_tab_with_one_completed_task();
        let original = app.tasks.clone();

        for &key in keys {
            press(&mut app, &fixture, key);
        }

        assert_eq!(app.view, SectionView::All, "{keys:?}");
        assert_eq!(app.tasks.len(), 1, "{keys:?}");

        press(&mut app, &fixture, KeyCode::Char('u'));

        assert_eq!(app.view, SectionView::Named("work".into()), "{keys:?}");
        assert_eq!(app.tasks, original, "{keys:?}");
        assert_eq!(app.list.selected(), Some(1), "{keys:?}");
        assert_eq!(saved_view(&fixture), app.view, "{keys:?}");

        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
        );

        assert_eq!(app.view, SectionView::All, "{keys:?}");
        assert_eq!(app.tasks.len(), 1, "{keys:?}");
        assert_eq!(saved_view(&fixture), SectionView::All, "{keys:?}");
    }
}

#[test]
fn undo_and_redo_of_a_rename_follow_the_section_between_its_names() {
    let fixture = Fixture::new();
    let mut app = app_on_a_tab_with_one_completed_task();

    app.rename_section("Job", &fixture.storage).unwrap();
    assert_eq!(app.view, SectionView::Named("job".into()));

    press(&mut app, &fixture, KeyCode::Char('u'));
    assert_eq!(app.view, SectionView::Named("work".into()));

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    assert_eq!(app.view, SectionView::Named("job".into()));
    assert_eq!(saved_view(&fixture), app.view);
}

#[test]
fn undo_keeps_a_tab_chosen_manually_after_the_change() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: a", "Work: b", "Home: c"]);
    app.show_section(SectionView::Named("work".into()));

    press(&mut app, &fixture, KeyCode::Char('d'));
    assert_eq!(app.view, SectionView::Named("work".into()));

    // The restored task is visible in [*], so nothing pulls the view back to Work.
    app.show_section(SectionView::All);
    press(&mut app, &fixture, KeyCode::Char('u'));

    assert_eq!(app.tasks.len(), 3);
    assert_eq!(app.view, SectionView::All);
    assert_eq!(app.list.selected(), Some(0));
    assert_eq!(saved_view(&fixture), SectionView::All);
}

#[test]
fn a_failed_undo_keeps_the_view_tasks_and_both_histories() {
    let fixture = Fixture::new();
    let mut app = app_on_a_tab_with_one_completed_task();
    press(&mut app, &fixture, KeyCode::Char('d'));
    let before = task_state(&app);
    block_saves(&fixture);

    assert!(app.undo(&fixture.storage).is_err());
    assert_eq!(app.view, SectionView::All);
    assert_eq!(task_state(&app), before);

    restore_saves(&fixture);
    app.undo(&fixture.storage).unwrap();
    block_saves(&fixture);

    assert!(app.redo(&fixture.storage).is_err());
    assert_eq!(app.view, SectionView::Named("work".into()));
    assert_eq!(app.tasks.len(), 2);
    assert!(app.can_redo());
}

fn app_on_work_with_three_sections() -> App {
    let mut app = app_with_texts(&["Work: a", "Home: b", "Personal: c"]);
    app.show_section(SectionView::Named("work".into()));

    app
}

#[test]
fn undo_keeps_a_manually_chosen_section_tab_and_selects_a_task_in_it() {
    let fixture = Fixture::new();
    let mut app = app_on_work_with_three_sections();

    press(&mut app, &fixture, KeyCode::Char('d'));
    assert_eq!(app.view, SectionView::All);

    app.show_section(SectionView::Named("personal".into()));
    press(&mut app, &fixture, KeyCode::Char('u'));

    assert_eq!(texts(&app), ["Work: a", "Home: b", "Personal: c"]);
    assert_eq!(app.view, SectionView::Named("personal".into()));
    assert_eq!(app.list.selected(), Some(2));
    assert_eq!(app.persisted.view, app.view);
    assert_eq!(saved_view(&fixture), app.view);
}

#[test]
fn redo_keeps_a_section_tab_chosen_between_undo_and_redo() {
    let fixture = Fixture::new();
    let mut app = app_on_work_with_three_sections();

    press(&mut app, &fixture, KeyCode::Char('d'));
    press(&mut app, &fixture, KeyCode::Char('u'));
    assert_eq!(app.view, SectionView::Named("work".into()));

    app.show_section(SectionView::Named("personal".into()));
    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );

    assert_eq!(texts(&app), ["Home: b", "Personal: c"]);
    assert_eq!(app.view, SectionView::Named("personal".into()));
    assert_eq!(app.list.selected(), Some(1));
    assert_eq!(saved_view(&fixture), app.view);
}

#[test]
fn editing_a_task_into_another_section_saves_the_tab_it_moves_to() {
    let fixture = Fixture::new();
    let mut app = app_on_work_with_three_sections();

    press(&mut app, &fixture, KeyCode::Char('e'));
    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    app.paste("Home: moved");
    press(&mut app, &fixture, KeyCode::Enter);

    assert_eq!(app.view, SectionView::Named("home".into()));
    assert_eq!(app.persisted.view, app.view);
    assert_eq!(saved_view(&fixture), app.view);
}

#[test]
fn a_failed_save_of_an_edit_that_moves_sections_keeps_the_view() {
    let fixture = Fixture::new();
    let mut app = app_on_work_with_three_sections();
    fixture.storage.save(&app.tasks).unwrap();

    press(&mut app, &fixture, KeyCode::Char('e'));
    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    app.paste("Home: moved");
    let before = task_state(&app);
    block_saves(&fixture);

    let result = app.key(KeyEvent::from(KeyCode::Enter), &fixture.storage);

    assert!(result.is_err());
    assert_eq!(app.view, SectionView::Named("work".into()));
    assert_eq!(app.persisted.view, SectionView::All);
    assert_eq!(task_state(&app), before);
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

    for code in [KeyCode::Char('D'), KeyCode::Char('X')] {
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
    mixed.tasks[0].status = TaskStatus::Carried;
    mixed.tasks[1].status = TaskStatus::Completed;
    press(&mut mixed, &fixture, KeyCode::Char('X'));

    assert_eq!(mixed.tasks.len(), 1);
    assert_eq!(mixed.tasks[0].status, TaskStatus::Carried);

    mixed.undo(&fixture.storage).unwrap();
    assert_eq!(mixed.tasks[0].status, TaskStatus::Carried);
    assert_eq!(mixed.tasks[1].status, TaskStatus::Completed);

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

        assert_eq!(app.tasks[0].status, TaskStatus::Carried);
        assert_eq!(app.history.len(), 1);

        press(&mut app, &fixture, KeyCode::Char('m'));
        assert_ne!(app.tasks[0].status, TaskStatus::Carried);

        app.undo(&fixture.storage).unwrap();
        assert_eq!(app.tasks[0].status, TaskStatus::Carried);
        assert!(app.can_redo());

        if action == KeyCode::Null {
            app.viewport.row_tasks = vec![Some(0), Some(1)];
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
        assert_eq!(app.tasks[index].status, TaskStatus::Completed);

        let history = app.history.len();
        press(&mut app, &fixture, KeyCode::Char('m'));
        assert_eq!(app.history.len(), history);

        app.undo(&fixture.storage).unwrap();
        assert_eq!(app.tasks[0].status, TaskStatus::Carried);

        app.redo(&fixture.storage).unwrap();
        let index = app.list.selected().unwrap();
        assert_eq!(app.tasks[index].status, TaskStatus::Completed);

        press(&mut app, &fixture, KeyCode::Enter);
        assert!(
            app.tasks
                .iter()
                .all(|task| task.status == TaskStatus::Pending)
        );
        assert_eq!(fixture.storage.load().unwrap(), app.tasks);
    }
}

#[test]
fn clearing_preserves_carried_order_selection_and_repeated_resets() {
    for selection in [None, Some(0), Some(1), Some(2), Some(3), Some(4)] {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(5);
        app.tasks[1].status = TaskStatus::Carried;
        app.tasks[2].status = TaskStatus::Completed;
        app.tasks[3].status = TaskStatus::Carried;
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

        press(&mut app, &fixture, KeyCode::Char('X'));
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

    assert_eq!(app.tasks[1].status, TaskStatus::Carried);

    press(&mut app, &fixture, KeyCode::Char('e'));
    app.paste(" edited");
    press(&mut app, &fixture, KeyCode::Enter);

    assert_eq!(app.tasks[1].status, TaskStatus::Carried);

    press(&mut app, &fixture, KeyCode::Char('a'));
    app.paste("new");
    press(&mut app, &fixture, KeyCode::Enter);

    assert_ne!(app.tasks[2].status, TaskStatus::Carried);

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
                    KeyModifiers::ALT,
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
            status: if completed {
                TaskStatus::Completed
            } else {
                TaskStatus::Pending
            },
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

    // A repeated Enter is not a deliberate selection.
    repeat(&mut app, &fixture, KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(app.mode, Mode::Searching { .. }));

    for (code, expected) in [
        (KeyCode::Tab, 4),
        (KeyCode::Tab, 0),
        (KeyCode::BackTab, 4),
        (KeyCode::BackTab, 3),
    ] {
        repeat(&mut app, &fixture, code, KeyModifiers::NONE);
        assert_eq!(app.list.selected(), Some(expected));
    }

    repeat(&mut app, &fixture, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(app.list.selected(), Some(4));

    press(&mut app, &fixture, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.list.selected(), Some(2));
    assert!(app.viewport.reveal_selection);

    press(&mut app, &fixture, KeyCode::Char('/'));
    app.paste("alpha");
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
fn empty_and_single_result_search_cycles_in_place_preserves_undo_and_ignores_mouse() {
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

        for code in [KeyCode::Tab, KeyCode::BackTab] {
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
            assert_ne!(app.tasks[0].status, TaskStatus::Completed);
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
            assert_ne!(app.tasks[1].status, TaskStatus::Completed);
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
    app.tasks[1].status = TaskStatus::Completed;
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
        app.viewport.row_tasks = (0..count).map(Some).collect();
        app.viewport.height = 2;
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
        app.viewport.reveal_selection = false;
        press(&mut app, &fixture, code);
        assert_eq!(app.list.selected(), Some(selected));
        assert!(app.viewport.reveal_selection);
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
        r#"{{"id":"00000000-0000-4000-8000-000000000001","created_at":"{created_at}","text":"t","status":"pending"}}"#
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
        crate::export::markdown(&app.tasks[..1], true),
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

// Pretends the pending count and `s` prefix were typed a full timeout ago.
fn let_prefixes_expire(app: &mut App) {
    let expired = Instant::now()
        .checked_sub(PREFIX_TIMEOUT)
        .expect("the clock is past the prefix timeout");

    app.pending_count = app.pending_count.map(|(count, _)| (count, expired));
    app.pending_section_jump = app.pending_section_jump.map(|_| expired);
}

#[test]
fn a_count_left_pending_past_the_timeout_is_forgotten() {
    let fixture = Fixture::new();
    let mut app = app_with_tasks(10);
    type_keys(&mut app, &fixture, "4");

    let_prefixes_expire(&mut app);
    press(&mut app, &fixture, KeyCode::Char('j'));

    assert_eq!(app.list.selected(), Some(1));
}

#[test]
fn an_s_prefix_left_pending_past_the_timeout_is_forgotten() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: a", "Home: b", "Work: c"]);
    press(&mut app, &fixture, KeyCode::Char('s'));
    assert!(app.section_jump_pending());

    let_prefixes_expire(&mut app);

    assert!(!app.section_jump_pending());

    // The digit starts an ordinary count instead of jumping to a tab.
    type_keys(&mut app, &fixture, "2j");

    assert_eq!(app.view, SectionView::All);
    assert_eq!(app.list.selected(), Some(2));
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
        KeyEvent::new(KeyCode::Up, KeyModifiers::ALT),
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
        KeyCode::Char('X'),
        KeyCode::Char('u'),
        KeyCode::Char('J'),
        KeyCode::Char('K'),
        KeyCode::Char('a'),
        KeyCode::Char('e'),
        KeyCode::Char('q'),
    ] {
        repeat(&mut app, &fixture, code, KeyModifiers::NONE);
    }

    for code in [KeyCode::Char('j'), KeyCode::Char('k')] {
        repeat(&mut app, &fixture, code, KeyModifiers::SHIFT);
    }

    for code in [KeyCode::Up, KeyCode::Down] {
        repeat(&mut app, &fixture, code, KeyModifiers::ALT);
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
        assert!(
            app.tasks
                .iter()
                .all(|task| task.status != TaskStatus::Completed)
        );

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
            app.tasks[index].status = TaskStatus::Completed;
        }

        app.list.select(Some(selected));
        let original = app.tasks.clone();

        let expected: Vec<_> = original
            .iter()
            .filter(|task| task.status != TaskStatus::Completed)
            .cloned()
            .collect();

        press(&mut app, &fixture, KeyCode::Char('X'));
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
    press(&mut app, &fixture, KeyCode::Char('X'));

    assert!(!fixture.task_path().exists());
    assert!(app.history.is_empty());

    press(&mut app, &fixture, KeyCode::Char('a'));
    press(&mut app, &fixture, KeyCode::Char('x'));
    press(&mut app, &fixture, KeyCode::Enter);
    press(&mut app, &fixture, KeyCode::Char('X'));

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
    app.tasks[0].status = TaskStatus::Completed;
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
        press(&mut app, &fixture, KeyCode::Char('X'));
        assert_eq!(app.tasks, original);

        match &app.mode {
            Mode::Adding(input, _) | Mode::Editing { input, .. } => {
                assert!(input.text().ends_with('X'))
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
        (vec![Char('X')], "clear 1 completed task"),
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
        app.tasks[1].status = TaskStatus::Completed;
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
        assert!(app.viewport.reveal_selection);
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
fn status_messages_explain_no_ops_without_changing_tasks() {
    let fixture = Fixture::new();
    let mut app = app_with_tasks(2);
    let original = app.tasks.clone();

    press(&mut app, &fixture, KeyCode::Char('p'));
    assert_eq!(notice(&app), Some("Nothing to paste"));

    press(&mut app, &fixture, KeyCode::Char('X'));
    assert_eq!(notice(&app), Some("No completed tasks"));

    assert_eq!(app.tasks, original);
    assert!(!app.can_undo());
    assert!(!fixture.task_path().exists());

    let mut done = app_with_tasks(2);
    done.tasks
        .iter_mut()
        .for_each(|task| task.status = TaskStatus::Completed);
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
fn status_messages_confirm_easy_to_miss_changes() {
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
            .for_each(|task| task.status = TaskStatus::Completed);

        press(&mut app, &fixture, KeyCode::Char('X'));
        assert_eq!(notice(&app), Some(message));
    }

    for (status, message) in [
        (TaskStatus::Pending, "Cleared 3 tasks"),
        (TaskStatus::Carried, "Cleared 2 tasks, kept 1 carried"),
    ] {
        let mut app = app_with_tasks(3);
        app.tasks[0].status = status;

        press(&mut app, &fixture, KeyCode::Char('D'));
        press(&mut app, &fixture, KeyCode::Enter);
        assert_eq!(notice(&app), Some(message));
    }
}

#[test]
fn failed_saves_never_confirm_a_change() {
    let fixture = Fixture::new();
    let mut app = app_with_tasks(3);
    app.tasks[0].status = TaskStatus::Completed;
    fixture.storage.save(&app.tasks).unwrap();
    block_saves(&fixture);

    for key in [KeyCode::Char('d'), KeyCode::Char('X')] {
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
        KeyCode::Char('X'),
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
    assert_eq!(app.tasks[0].status, TaskStatus::Completed);

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

    assert_eq!(app.tasks[0].status, TaskStatus::Completed);

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
    assert!(app.viewport.reveal_selection);
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
        vec![Char('X')],
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

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(18, 6)).unwrap();

    for (tasks, selected) in snapshots.into_iter().rev() {
        app.viewport.scroll = usize::MAX;
        press(&mut app, &fixture, Char('u'));
        assert_eq!(app.tasks, tasks);
        assert_eq!(app.list.selected(), selected);
        assert!(app.viewport.reveal_selection);
        assert_eq!(fixture.storage.load().unwrap(), tasks);

        let mut rows = Rect::default();
        terminal
            .draw(|frame| rows = crate::ui::draw(frame, &mut app))
            .unwrap();

        if let Some(selected) = selected {
            let end =
                (app.viewport.scroll + usize::from(rows.height)).min(app.viewport.row_tasks.len());
            assert!(app.viewport.row_tasks[app.viewport.scroll..end].contains(&Some(selected)));
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

    for code in [KeyCode::Char('d'), KeyCode::Char('X'), KeyCode::Char('u')] {
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
    app.tasks[1].status = TaskStatus::Completed;
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
    assert_eq!(app.tasks[1].status, TaskStatus::Completed);
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
        (KeyCode::Down, KeyCode::Up, KeyModifiers::ALT),
    ] {
        let fixture = Fixture::new();
        let mut app = app_with_tasks(3);
        app.tasks[1].status = TaskStatus::Completed;
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

        press(&mut app, &fixture, KeyCode::Char('u'));
        assert_eq!(notice(&app), Some("Undid: move task"));
        assert_eq!(app.list.selected(), Some(2));
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

        for code in [KeyCode::Char('J'), KeyCode::Char('K')] {
            press(&mut app, &fixture, KeyEvent::new(code, KeyModifiers::SHIFT));
        }

        for code in [KeyCode::Down, KeyCode::Up] {
            press(&mut app, &fixture, KeyEvent::new(code, KeyModifiers::ALT));
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
                if index == 0 {
                    task.status = TaskStatus::Completed;
                }
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
            assert!(app.viewport.reveal_selection);

            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(18, 3)).unwrap();
            terminal
                .draw(|frame| {
                    crate::ui::draw(frame, &mut app);
                })
                .unwrap();

            assert_eq!(
                app.viewport.row_tasks[app.viewport.scroll],
                Some(expected_index)
            );

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

    assert_ne!(app.tasks[1].status, TaskStatus::Completed);
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
    app.tasks[1].status = TaskStatus::Completed;
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
    app.viewport.row_tasks = vec![Some(0), Some(1), Some(2)];
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
    assert_eq!(app.tasks[2].status, TaskStatus::Completed);
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
        app.viewport.row_tasks = vec![Some(0), Some(1), Some(2)];
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
    app.viewport.scroll = 5;
    app.viewport.row_tasks = (0..10).map(Some).collect();
    let rows = Rect::new(1, 1, 30, 3);

    let click = |column, row| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };

    app.mouse(click(1, 2), rows, &fixture.storage).unwrap();
    assert_eq!(app.list.selected(), Some(6));
    assert_eq!(app.tasks[6].status, TaskStatus::Completed);
    assert_eq!(fixture.storage.load().unwrap(), app.tasks);

    for (column, row) in [(0, 1), (31, 1), (1, 0), (1, 4)] {
        app.mouse(click(column, row), rows, &fixture.storage)
            .unwrap();
        assert_eq!(app.list.selected(), Some(6));
    }

    app.mode = Mode::Adding(TextInput::default(), AddPlacement::End);
    app.mouse(click(1, 2), rows, &fixture.storage).unwrap();
    assert_eq!(app.tasks[6].status, TaskStatus::Completed);

    app.mode = Mode::Normal;
    app.list.select(Some(9));
    app.viewport.scroll = 7;
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

    assert_eq!(app.viewport.scroll, 6);
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

#[test]
fn emacs_aliases_match_their_normal_mode_keys() {
    let fixture = Fixture::new();

    for (alias, equivalent) in [
        (
            (KeyCode::Char('o'), KeyModifiers::CONTROL),
            KeyCode::Char('o'),
        ),
        ((KeyCode::Char('e'), KeyModifiers::ALT), KeyCode::Char('e')),
        ((KeyCode::Char('g'), KeyModifiers::ALT), KeyCode::Char('`')),
        (
            (KeyCode::Char('v'), KeyModifiers::CONTROL),
            KeyCode::PageDown,
        ),
        ((KeyCode::Char('v'), KeyModifiers::ALT), KeyCode::PageUp),
    ] {
        let mut apps = [
            app_with_texts(&["Work: one", "Work: two", "three", "four", "five"]),
            app_with_texts(&["Work: one", "Work: two", "three", "four", "five"]),
        ];

        for app in &mut apps {
            app.show_section(SectionView::Named("work".into()));
            app.list.select(Some(1));
            app.viewport.row_tasks = (0..5).map(Some).collect();
            app.viewport.height = 2;
        }

        let [aliased, plain] = &mut apps;
        press(aliased, &fixture, KeyEvent::new(alias.0, alias.1));
        press(plain, &fixture, equivalent);

        assert_ne!(aliased.list.selected(), None);
        assert_eq!(aliased.mode, plain.mode);
        assert_eq!(aliased.view, plain.view);
        assert_eq!(aliased.list.selected(), plain.list.selected());
        assert_eq!(
            aliased
                .completion
                .as_ref()
                .map(|c| (c.labels.clone(), c.current)),
            plain
                .completion
                .as_ref()
                .map(|c| (c.labels.clone(), c.current)),
        );
        assert!(!aliased.can_undo());
    }

    assert!(!fixture.task_path().exists());
}

#[test]
fn emacs_aliases_leave_text_inputs_unchanged() {
    let fixture = Fixture::new();

    for start in [KeyCode::Char('a'), KeyCode::Char('e'), KeyCode::Char('/')] {
        let mut app = app_with_tasks(3);
        app.viewport.row_tasks = (0..3).map(Some).collect();
        app.viewport.height = 1;
        press(&mut app, &fixture, start);
        let selected = app.list.selected();
        let text = app.search_query().map(str::to_owned);

        for (code, modifiers) in [
            (KeyCode::Char('o'), KeyModifiers::CONTROL),
            (KeyCode::Char('e'), KeyModifiers::ALT),
            (KeyCode::Char('g'), KeyModifiers::ALT),
            (KeyCode::Char('v'), KeyModifiers::CONTROL),
            (KeyCode::Char('v'), KeyModifiers::ALT),
        ] {
            press(&mut app, &fixture, KeyEvent::new(code, modifiers));
        }

        assert_ne!(app.mode, Mode::Normal);
        assert!(!matches!(app.mode, Mode::FuzzySection { .. }));
        assert_eq!(app.list.selected(), selected);
        assert_eq!(app.search_query().map(str::to_owned), text);
    }

    assert!(!fixture.task_path().exists());
}

#[test]
fn page_aliases_repeat_while_mode_changing_aliases_fire_once() {
    let fixture = Fixture::new();
    let mut app = app_with_tasks(6);
    app.viewport.row_tasks = (0..6).map(Some).collect();
    app.viewport.height = 2;
    app.list.select(Some(0));

    repeat(
        &mut app,
        &fixture,
        KeyCode::Char('v'),
        KeyModifiers::CONTROL,
    );
    let paged_down = app.list.selected();

    assert_ne!(paged_down, Some(0));

    repeat(&mut app, &fixture, KeyCode::Char('v'), KeyModifiers::ALT);

    assert_ne!(app.list.selected(), paged_down);

    for (code, modifiers) in [
        (KeyCode::Char('o'), KeyModifiers::CONTROL),
        (KeyCode::Char('e'), KeyModifiers::ALT),
        (KeyCode::Char('g'), KeyModifiers::ALT),
        (KeyCode::Char('k'), KeyModifiers::CONTROL),
    ] {
        repeat(&mut app, &fixture, code, modifiers);
        assert_eq!(app.mode, Mode::Normal);
    }

    assert_eq!(app.tasks.len(), 6);
    assert!(!app.can_undo());
}

#[test]
fn ctrl_k_does_nothing_in_normal_mode() {
    let fixture = Fixture::new();
    let mut app = app_with_tasks(2);
    press(&mut app, &fixture, KeyCode::Char('y'));
    let before = task_state(&app);

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
    );

    assert_eq!(task_state(&app), before);
    assert_eq!(app.task_register.as_deref(), Some("task 0"));
    assert!(!fixture.task_path().exists());
}

#[test]
fn ctrl_s_starts_a_search_like_slash_and_ctrl_f() {
    let fixture = Fixture::new();
    let mut expected = app_with_texts(&["one", "two"]);
    expected.list.select(Some(1));
    press(&mut expected, &fixture, KeyCode::Char('/'));

    for character in ['f', 's'] {
        let mut app = app_with_texts(&["one", "two"]);
        app.list.select(Some(1));
        let search = KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL);

        press(&mut app, &fixture, search);
        assert_eq!(app.mode, expected.mode);

        // Inside the search input, the chord is ignored rather than restarting the search.
        app.paste("tw");
        press(&mut app, &fixture, search);
        assert_eq!(app.search_query(), Some("tw"));
    }
}

#[test]
fn ctrl_d_and_ctrl_u_move_half_a_page_and_stop_at_the_ends() {
    let fixture = Fixture::new();
    let mut app = app_with_tasks(10);
    app.viewport.row_tasks = (0..10).map(Some).collect();
    app.viewport.height = 4;
    app.list.select(Some(0));
    let original = app.tasks.clone();

    for (character, selected) in [
        ('d', 2),
        ('d', 4),
        ('d', 6),
        ('d', 8),
        ('d', 9),
        ('d', 9),
        ('u', 7),
        ('u', 5),
    ] {
        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL),
        );

        assert_eq!(app.list.selected(), Some(selected), "Ctrl+{character}");
        assert!(app.viewport.reveal_selection);
    }

    assert_eq!(app.tasks, original);
    assert!(!app.can_undo());
    assert!(!fixture.task_path().exists());
}

#[test]
fn half_pages_move_at_least_one_row_in_tiny_or_undrawn_viewports() {
    let fixture = Fixture::new();

    for height in [0, 1] {
        let mut app = app_with_tasks(3);
        app.viewport.row_tasks = (0..3).map(Some).collect();
        app.viewport.height = height;
        app.list.select(Some(0));

        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.list.selected(), Some(1));

        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.list.selected(), Some(0));
    }

    let mut empty = app_with_tasks(0);
    press(
        &mut empty,
        &fixture,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
    );
    assert_eq!(empty.list.selected(), None);
}

#[test]
fn held_ctrl_d_and_ctrl_u_keep_moving() {
    let fixture = Fixture::new();
    let mut app = app_with_tasks(6);
    app.viewport.row_tasks = (0..6).map(Some).collect();
    app.viewport.height = 4;
    app.list.select(Some(0));

    repeat(
        &mut app,
        &fixture,
        KeyCode::Char('d'),
        KeyModifiers::CONTROL,
    );
    assert_eq!(app.list.selected(), Some(2));

    repeat(
        &mut app,
        &fixture,
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    );
    assert_eq!(app.list.selected(), Some(0));
}

#[test]
fn only_shifted_x_clears_completed_tasks() {
    let fixture = Fixture::new();
    let mut app = app_with_tasks(2);
    app.tasks[0].status = TaskStatus::Completed;
    let before = task_state(&app);

    press(&mut app, &fixture, KeyCode::Char('x'));

    assert_eq!(task_state(&app), before);
    assert!(!fixture.task_path().exists());

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::SHIFT),
    );

    assert_eq!(texts(&app), ["task 1"]);
    assert_eq!(notice(&app), Some("Cleared 1 completed task"));
}

#[test]
fn shift_arrows_navigate_without_reordering_tasks() {
    let fixture = Fixture::new();
    let mut app = app_with_tasks(3);
    app.list.select(Some(0));
    let original = app.tasks.clone();

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
    );

    assert_eq!(app.list.selected(), Some(1));
    assert_eq!(app.tasks, original);
    assert!(!app.can_undo());
    assert!(!fixture.task_path().exists());
}

#[test]
fn ctrl_k_deletes_to_the_end_in_every_text_input() {
    let fixture = Fixture::new();
    let ctrl_k = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL);

    for start in [
        KeyCode::Char('a'),
        KeyCode::Char('e'),
        KeyCode::Char('/'),
        KeyCode::Char('E'),
        KeyCode::Char('`'),
    ] {
        let mut app = app_with_texts(&["Work: alpha", "Work: beta"]);
        app.show_section(SectionView::Named("work".into()));
        press(&mut app, &fixture, start);
        app.paste("xyz");
        press(&mut app, &fixture, KeyCode::Home);

        press(&mut app, &fixture, ctrl_k);
        repeat(
            &mut app,
            &fixture,
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        );

        let (Mode::Adding(input, _)
        | Mode::Editing { input, .. }
        | Mode::Searching { input, .. }
        | Mode::RenamingSection { input }
        | Mode::FuzzySection { input, .. }) = &app.mode
        else {
            panic!("{start:?} left text input");
        };

        assert_eq!(input.text(), "");
        assert_eq!(app.tasks.len(), 2);
        assert!(!app.can_undo());
    }
}

// Opens the keyboard reference with the bounds a draw would record, so scrolling can be
// tested without a terminal.
fn open_help(app: &mut App, fixture: &Fixture, max_scroll: usize, page: usize) {
    press(app, fixture, KeyCode::Char('?'));

    let Mode::Help {
        max_scroll: max,
        page: rows,
        ..
    } = &mut app.mode
    else {
        panic!("? did not open the keyboard reference");
    };

    *max = max_scroll;
    *rows = page;
}

fn help_scroll(app: &App) -> usize {
    let Mode::Help { scroll, .. } = &app.mode else {
        panic!("expected the keyboard reference");
    };

    *scroll
}

#[test]
fn question_mark_opens_the_reference_from_normal_mode_with_or_without_shift() {
    let fixture = Fixture::new();

    for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
        let mut app = app_with_tasks(3);
        press(&mut app, &fixture, KeyCode::Char('2'));

        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('?'), modifiers),
        );

        assert_eq!(help_scroll(&app), 0, "{modifiers:?}");
        assert_eq!(app.pending_count, None);
    }
}

#[test]
fn question_mark_esc_and_ctrl_g_each_close_the_reference_without_changing_anything() {
    let fixture = Fixture::new();

    for close in [
        KeyEvent::from(KeyCode::Char('?')),
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT),
        KeyEvent::from(KeyCode::Esc),
        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL),
    ] {
        let mut app = app_with_texts(&["Work: a", "Home: b", "Work: c"]);
        app.show_section(SectionView::Named("work".into()));
        press(&mut app, &fixture, KeyCode::Char('j'));
        app.viewport.scroll = 1;
        app.viewport.reveal_selection = false;
        app.task_register = Some("kept".into());
        let before = task_state(&app);

        open_help(&mut app, &fixture, 10, 5);
        press(&mut app, &fixture, KeyCode::Char('j'));
        press(&mut app, &fixture, close);

        assert_eq!(app.mode, Mode::Normal, "{close:?}");
        assert_eq!(task_state(&app), before);
        assert_eq!(app.view, SectionView::Named("work".into()));
        assert_eq!(
            (app.viewport.scroll, app.viewport.reveal_selection),
            (1, false)
        );
        assert_eq!(app.task_register.as_deref(), Some("kept"));
    }

    assert!(!fixture.task_path().exists());
}

#[test]
fn keys_in_the_reference_never_reach_task_commands() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: a", "Home: b"]);
    app.tasks[0].status = TaskStatus::Completed;
    app.yank_task();
    let before = task_state(&app);

    open_help(&mut app, &fixture, 0, 5);

    for key in [
        KeyEvent::from(KeyCode::Char('d')),
        KeyEvent::from(KeyCode::Delete),
        KeyEvent::from(KeyCode::Char('X')),
        KeyEvent::from(KeyCode::Char('D')),
        KeyEvent::from(KeyCode::Char(' ')),
        KeyEvent::from(KeyCode::Enter),
        KeyEvent::from(KeyCode::Char('m')),
        KeyEvent::from(KeyCode::Char('J')),
        KeyEvent::from(KeyCode::Char('L')),
        KeyEvent::from(KeyCode::Char('l')),
        KeyEvent::from(KeyCode::Tab),
        KeyEvent::from(KeyCode::Char('p')),
        KeyEvent::from(KeyCode::Char('u')),
        KeyEvent::from(KeyCode::Char('a')),
        KeyEvent::from(KeyCode::Char('/')),
        KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT),
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
        KeyEvent::from(KeyCode::Char('q')),
    ] {
        assert!(!press(&mut app, &fixture, key), "{key:?} quit");
        assert!(matches!(app.mode, Mode::Help { .. }), "{key:?} closed help");
    }

    assert_eq!(task_state(&app), before);
    assert_eq!(app.view, SectionView::All);
    assert!(!fixture.task_path().exists());
}

#[test]
fn reference_scrolling_stops_at_both_ends() {
    let fixture = Fixture::new();
    let mut app = app_with_tasks(3);
    open_help(&mut app, &fixture, 20, 8);

    let steps = [
        (KeyEvent::from(KeyCode::Char('k')), 0),
        (KeyEvent::from(KeyCode::Up), 0),
        (KeyEvent::from(KeyCode::Char('j')), 1),
        (KeyEvent::from(KeyCode::Down), 2),
        (KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL), 3),
        (KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL), 2),
        (KeyEvent::from(KeyCode::PageDown), 9),
        (KeyEvent::from(KeyCode::PageDown), 16),
        (KeyEvent::from(KeyCode::PageDown), 20),
        (KeyEvent::from(KeyCode::Char('j')), 20),
        (KeyEvent::from(KeyCode::PageUp), 13),
        (KeyEvent::from(KeyCode::Home), 0),
        (KeyEvent::from(KeyCode::PageUp), 0),
        (KeyEvent::from(KeyCode::End), 20),
    ];

    for (key, expected) in steps {
        press(&mut app, &fixture, key);

        assert_eq!(help_scroll(&app), expected, "{key:?}");
    }

    assert_eq!(app.list.selected(), Some(0));
}

#[test]
fn held_scroll_keys_repeat_but_a_held_question_mark_does_not_toggle() {
    let fixture = Fixture::new();
    let mut app = app_with_tasks(3);
    open_help(&mut app, &fixture, 20, 8);

    repeat(&mut app, &fixture, KeyCode::Char('j'), KeyModifiers::NONE);
    repeat(&mut app, &fixture, KeyCode::Down, KeyModifiers::NONE);
    repeat(&mut app, &fixture, KeyCode::PageDown, KeyModifiers::NONE);
    repeat(
        &mut app,
        &fixture,
        KeyCode::Char('n'),
        KeyModifiers::CONTROL,
    );
    repeat(&mut app, &fixture, KeyCode::Char('k'), KeyModifiers::NONE);

    assert_eq!(help_scroll(&app), 9);

    repeat(&mut app, &fixture, KeyCode::Char('?'), KeyModifiers::NONE);
    repeat(&mut app, &fixture, KeyCode::Esc, KeyModifiers::NONE);

    assert_eq!(help_scroll(&app), 9);

    press(&mut app, &fixture, KeyCode::Char('?'));
    repeat(&mut app, &fixture, KeyCode::Char('?'), KeyModifiers::SHIFT);

    assert_eq!(app.mode, Mode::Normal);

    let release = KeyEvent::new_with_kind(
        KeyCode::Char('?'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    );
    press(&mut app, &fixture, release);

    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn question_mark_is_text_in_every_input_mode() {
    let fixture = Fixture::new();

    for start in [
        KeyEvent::from(KeyCode::Char('a')),
        KeyEvent::from(KeyCode::Char('e')),
        KeyEvent::from(KeyCode::Char('/')),
        KeyEvent::from(KeyCode::Char('E')),
        KeyEvent::from(KeyCode::Char('`')),
    ] {
        let mut app = app_with_texts(&["Work: a"]);
        app.show_section(SectionView::Named("work".into()));
        press(&mut app, &fixture, start);
        press(&mut app, &fixture, KeyCode::Char('?'));
        press(
            &mut app,
            &fixture,
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT),
        );

        let (Mode::Adding(input, _)
        | Mode::Editing { input, .. }
        | Mode::Searching { input, .. }
        | Mode::RenamingSection { input }
        | Mode::FuzzySection { input, .. }) = &app.mode
        else {
            panic!("{start:?} left text input");
        };

        assert!(input.text().ends_with("??"), "{start:?}");
    }
}

#[test]
fn mouse_clicks_are_ignored_and_the_wheel_scrolls_while_the_reference_is_open() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: a", "b"]);
    app.viewport.row_tasks = vec![Some(0), Some(1)];
    app.viewport.tab_targets = vec![(Rect::new(1, 0, 5, 1), SectionView::Named("work".into()))];
    let before = task_state(&app);
    let rows = Rect::new(1, 1, 20, 2);

    let mouse = |kind, column, row| MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };

    open_help(&mut app, &fixture, 3, 8);

    for event in [
        mouse(MouseEventKind::Down(MouseButton::Left), 1, 1),
        mouse(MouseEventKind::Down(MouseButton::Left), 2, 0),
        mouse(MouseEventKind::ScrollDown, 1, 1),
        mouse(MouseEventKind::ScrollDown, 30, 9),
    ] {
        app.mouse(event, rows, &fixture.storage).unwrap();
    }

    assert_eq!(help_scroll(&app), 2);
    assert_eq!(task_state(&app), before);
    assert_eq!(app.view, SectionView::All);

    for _ in 0..3 {
        let event = mouse(MouseEventKind::ScrollUp, 1, 1);
        app.mouse(event, rows, &fixture.storage).unwrap();
    }

    assert_eq!(help_scroll(&app), 0);
    assert_eq!(app.viewport.scroll, 0);
    assert!(!fixture.task_path().exists());
}
