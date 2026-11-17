use super::*;
use crate::storage::tests::Fixture;
use ratatui::crossterm::event::{KeyEventKind, KeyModifiers};

fn app_with_texts(texts: &[&str]) -> App {
    App::new(texts.iter().map(|text| Task::new(*text)).collect())
}

fn press(app: &mut App, fixture: &Fixture, key: impl Into<KeyEvent>) {
    assert!(!app.handle_key(key.into(), &fixture.storage).unwrap());
}

fn type_text(app: &mut App, fixture: &Fixture, text: &str) {
    for character in text.chars() {
        press(app, fixture, KeyCode::Char(character));
    }
}

fn finder(app: &App) -> &TaskFinder {
    match &app.mode {
        Mode::FindingTask(finder) => finder,
        mode => panic!("expected the task finder, found {mode:?}"),
    }
}

// The focused match, as a task index.
fn focused(app: &App) -> Option<usize> {
    let finder = finder(app);
    finder.matches.get(finder.focused).copied()
}

fn named(key: &str) -> SectionView {
    SectionView::Named(key.into())
}

#[test]
fn f_opens_an_empty_finder_without_moving() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: report", "Home: dishes", "plain"]);
    app.select_tab(named("home"));

    press(&mut app, &fixture, KeyCode::Char('f'));

    assert!(finder(&app).input.text().is_empty());
    assert!(finder(&app).matches.is_empty());
    assert_eq!(app.view, named("home"));
    assert_eq!(app.list.selected(), Some(1));

    // A blank query still matches nothing.
    press(&mut app, &fixture, KeyCode::Char(' '));
    assert!(finder(&app).matches.is_empty());
}

#[test]
fn queries_match_every_tab_fuzzily_and_rank_completed_ties_last() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: fix bug", "Home: fix bug", "Home: water plants"]);
    app.tasks[0].status = TaskStatus::Completed;
    app.select_tab(named("home"));

    press(&mut app, &fixture, KeyCode::Char('f'));
    type_text(&mut app, &fixture, "fb");

    // Both bugs match alike, so the completed one comes second despite coming first in the
    // list, and the Work task is found from the Home tab.
    assert_eq!(finder(&app).matches, [1, 0]);

    // The section prefix counts, and Space separates words that must all match.
    press(&mut app, &fixture, KeyCode::Backspace);
    press(&mut app, &fixture, KeyCode::Backspace);
    type_text(&mut app, &fixture, "home pl");
    assert_eq!(finder(&app).input.text(), "home pl");
    assert_eq!(finder(&app).matches, [2]);

    type_text(&mut app, &fixture, "zzz");
    assert!(finder(&app).matches.is_empty());
}

#[test]
fn tab_and_arrows_step_through_matches_and_wrap_while_cursor_moves_keep_focus() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["alpha", "alps", "alto"]);

    press(&mut app, &fixture, KeyCode::Char('f'));
    type_text(&mut app, &fixture, "al");
    let matches = finder(&app).matches.clone();
    assert_eq!(matches.len(), 3);
    assert_eq!(focused(&app), Some(matches[0]));

    press(&mut app, &fixture, KeyCode::Tab);
    assert_eq!(focused(&app), Some(matches[1]));

    press(&mut app, &fixture, KeyCode::Left);
    assert_eq!(focused(&app), Some(matches[1]));

    press(&mut app, &fixture, KeyCode::Down);
    press(&mut app, &fixture, KeyCode::Down);
    assert_eq!(focused(&app), Some(matches[0]));

    press(&mut app, &fixture, KeyCode::BackTab);
    assert_eq!(focused(&app), Some(matches[2]));

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
    );
    assert_eq!(focused(&app), Some(matches[0]));

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
    );
    assert_eq!(focused(&app), Some(matches[2]));

    // Holding Tab keeps stepping, and editing the query focuses the best match again.
    let held = KeyEvent::new_with_kind(KeyCode::Tab, KeyModifiers::NONE, KeyEventKind::Repeat);
    press(&mut app, &fixture, held);
    assert_eq!(focused(&app), Some(matches[0]));

    press(&mut app, &fixture, KeyCode::Tab);
    press(&mut app, &fixture, KeyCode::End);
    type_text(&mut app, &fixture, "p");
    assert_eq!(finder(&app).focused, 0);
}

#[test]
fn enter_stays_on_a_tab_that_shows_the_task_without_saving_or_history() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: report", "Home: dishes", "plain"]);
    app.list.select(Some(0));
    let tasks = app.tasks.clone();

    press(&mut app, &fixture, KeyCode::Char('f'));
    type_text(&mut app, &fixture, "dishes");

    // Holding Enter does nothing; only a press goes to the task.
    let held = KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat);
    press(&mut app, &fixture, held);
    assert!(matches!(app.mode, Mode::FindingTask(_)));

    press(&mut app, &fixture, KeyCode::Enter);

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view, SectionView::All);
    assert_eq!(app.list.selected(), Some(1));
    assert_eq!(app.tasks, tasks);
    assert!(!app.can_undo() && !app.can_redo());
    assert!(!fixture.task_path().exists());
}

#[test]
fn enter_switches_to_the_tasks_own_tab_and_quote_returns() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: report", "Home: dishes", "plain"]);
    app.select_tab(named("work"));

    press(&mut app, &fixture, KeyCode::Char('f'));
    type_text(&mut app, &fixture, "dishes");
    press(&mut app, &fixture, KeyCode::Enter);

    assert_eq!(app.view, named("home"));
    assert_eq!(app.list.selected(), Some(1));

    press(&mut app, &fixture, KeyCode::Char('f'));
    type_text(&mut app, &fixture, "plain");
    press(&mut app, &fixture, KeyCode::Enter);

    assert_eq!(app.view, SectionView::Unsectioned);
    assert_eq!(app.list.selected(), Some(2));

    press(&mut app, &fixture, KeyCode::Char('\''));
    assert_eq!(app.view, named("home"));
    assert_eq!(app.list.selected(), Some(1));
}

#[test]
fn esc_and_empty_enter_close_while_enter_without_matches_waits() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Work: report", "Home: dishes"]);
    app.select_tab(named("work"));

    press(&mut app, &fixture, KeyCode::Char('f'));
    type_text(&mut app, &fixture, "dishes");
    press(&mut app, &fixture, KeyCode::Esc);

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view, named("work"));
    assert_eq!(app.list.selected(), Some(0));

    press(&mut app, &fixture, KeyCode::Char('f'));
    press(&mut app, &fixture, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Normal);

    press(&mut app, &fixture, KeyCode::Char('f'));
    type_text(&mut app, &fixture, "zzz");
    press(&mut app, &fixture, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::FindingTask(_)));

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL),
    );
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view, named("work"));
}

#[test]
fn shortcuts_type_into_the_query_and_pastes_become_one_line() {
    let fixture = Fixture::new();
    let mut app = app_with_texts(&["Buy milk", "Call mom"]);
    let tasks = app.tasks.clone();

    press(&mut app, &fixture, KeyCode::Char('f'));
    type_text(&mut app, &fixture, "dxq");
    assert_eq!(finder(&app).input.text(), "dxq");
    assert_eq!(app.tasks, tasks);

    press(
        &mut app,
        &fixture,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    app.handle_paste("buy\nmilk");

    assert_eq!(finder(&app).input.text(), "buy milk");
    assert_eq!(finder(&app).matches, [0]);
}
