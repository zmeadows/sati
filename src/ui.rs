use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Mode};
use crate::input::TextInput;

// Gruvbox Material dark/medium; the terminal supplies the background.
const TEXT: Color = Color::Rgb(212, 190, 152);
const MUTED: Color = Color::Rgb(146, 131, 116);
pub const DEFAULT_ACCENT: Color = Color::Rgb(231, 138, 78);
const BORDER: Color = Color::Rgb(80, 73, 69);

pub fn draw(frame: &mut Frame, app: &mut App) -> Rect {
    let area = frame.area();
    let entering_text = matches!(app.mode, Mode::Adding(_) | Mode::Editing { .. });
    // Give the checklist priority when height is limited; input always gets a row.
    let input_height = u16::from(entering_text && area.height > 0);
    let mut shortcuts = shortcut_lines(app, area.width);
    let available = usize::from(area.height.saturating_sub(3 + input_height));
    if shortcuts.len() > available {
        shortcuts.clear();
    } else if !shortcuts.is_empty() && shortcuts.len() < available {
        shortcuts.insert(0, Line::raw(""));
    }
    let legend_height = shortcuts.len() as u16;
    let sections = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(input_height),
        Constraint::Length(legend_height),
    ])
    .split(area);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(if app.rounded {
            BorderType::Rounded
        } else {
            BorderType::Plain
        })
        .border_style(Style::default().fg(BORDER));
    // Hide the title when it cannot fit intact between the corners.
    if app.logo && sections[0].width >= 8 {
        block = block.title(Line::styled(" sati ", Style::default().fg(MUTED)).left_aligned());
    }
    let rows = block.inner(sections[0]);
    frame.render_widget(block, sections[0]);

    draw_tasks(frame, app, rows);

    match &app.mode {
        Mode::Adding(input) => draw_input(frame, sections[1], input, "add", app.accent),
        Mode::Editing { input, .. } => draw_input(frame, sections[1], input, "edit", app.accent),
        _ => {}
    }
    frame.render_widget(
        Paragraph::new(shortcuts).style(Style::default().fg(MUTED)),
        sections[2],
    );
    if app.mode == Mode::ConfirmClear {
        draw_confirmation(frame, area);
    }
    rows
}

fn draw_tasks(frame: &mut Frame, app: &mut App, area: Rect) {
    app.viewport_height = area.height;
    app.row_tasks.clear();
    if app.tasks.is_empty() {
        app.scroll = 0;
        frame.render_widget(
            Paragraph::new(" Press a to add a task").style(Style::default().fg(MUTED)),
            area,
        );
        return;
    }
    let mut lines = Vec::new();
    let text_width = usize::from(area.width.saturating_sub(6));
    for (index, task) in app.tasks.iter().enumerate() {
        let text: String = task
            .text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let wrapped = if app.wrap && text_width > 0 {
            wrap_text(&text, text_width)
        } else {
            vec![text]
        };
        let text_style = if task.completed {
            Style::default()
                .fg(MUTED)
                .add_modifier(Modifier::CROSSED_OUT)
        } else {
            Style::default().fg(TEXT)
        };
        for (row, text) in wrapped.into_iter().enumerate() {
            app.row_tasks.push(index);
            let prefix = if row > 0 {
                "     "
            } else if task.completed {
                " [x] "
            } else {
                " [ ] "
            };
            let color = if app.list.selected() == Some(index) {
                app.accent
            } else if task.completed {
                MUTED
            } else {
                TEXT
            };
            lines.push(Line::from(vec![
                Span::styled(
                    if row == 0 && app.list.selected() == Some(index) {
                        "›"
                    } else {
                        " "
                    },
                    Style::default().fg(app.accent),
                ),
                Span::styled(prefix, Style::default().fg(color)),
                Span::styled(text, text_style),
            ]));
        }
    }
    let height = usize::from(area.height);
    app.scroll = app.scroll.min(lines.len().saturating_sub(height));
    if height == 0 {
        return;
    }
    if let Some(selected) = app.list.selected() {
        let start = app
            .row_tasks
            .iter()
            .position(|&task| task == selected)
            .unwrap_or(0);
        let end = app
            .row_tasks
            .iter()
            .rposition(|&task| task == selected)
            .unwrap_or(start)
            + 1;
        if app.reveal_selection || end <= app.scroll || start >= app.scroll + height {
            if end - start > height || start < app.scroll {
                app.scroll = start;
            } else if end > app.scroll + height {
                app.scroll = end - height;
            }
        }
    }
    app.reveal_selection = false;
    frame.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .skip(app.scroll)
                .take(height)
                .collect::<Vec<_>>(),
        ),
        area,
    );
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        let mut used = 0;
        let mut word_break = None;
        let mut split = None;
        for (index, grapheme) in remaining.grapheme_indices(true) {
            let size = grapheme.width();
            if used + size > width {
                // A glyph wider than the entire text column cannot be drawn intact.
                if index == 0 {
                    lines.push("�".to_owned());
                    split = Some(grapheme.len());
                } else {
                    let boundary = if grapheme.chars().all(char::is_whitespace) {
                        index
                    } else {
                        word_break.filter(|&offset| offset > 0).unwrap_or(index)
                    };
                    split = Some(boundary);
                    lines.push(remaining[..boundary].trim_end().to_owned());
                }
                break;
            }
            if grapheme.chars().all(char::is_whitespace) {
                word_break = Some(index);
            }
            used += size;
        }
        let Some(split) = split else {
            lines.push(remaining.to_owned());
            break;
        };
        remaining = remaining[split..].trim_start();
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn shortcut_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    if !app.keybinds || width == 0 || app.mode == Mode::ConfirmClear {
        return Vec::new();
    }
    let keys = Style::default().fg(TEXT).add_modifier(Modifier::BOLD);
    let available = |key: &str| match key {
        "u" => app.can_undo(),
        "x" => app.tasks.iter().any(|task| task.completed),
        _ => true,
    };
    // The full aligned legend occupies 54 columns; pack smaller contexts by action.
    if app.mode != Mode::Normal || app.tasks.is_empty() || width < 54 {
        let mut rows: Vec<Vec<(&str, &str)>> = match app.mode {
            Mode::Adding(_) | Mode::Editing { .. } => {
                vec![vec![("Enter", "Confirm"), ("Esc", "Cancel")]]
            }
            _ if app.tasks.is_empty() => vec![vec![("u", "Undo"), ("w", "Wrap"), ("q", "Quit")]],
            _ => vec![
                vec![("j/k", "Navigate"), ("Space", "Toggle"), ("w", "Wrap")],
                vec![("a/e", "Add/edit"), ("d", "Delete"), ("u", "Undo")],
                vec![("x", "Clear done"), ("D", "Clear all"), ("q", "Quit")],
            ],
        };
        for row in &mut rows {
            row.retain(|(key, _)| available(key));
        }
        let mut lines = wrap_shortcuts(&rows, width);
        // Keep at most three hint rows, dropping secondary actions first.
        for omit in ["x", "D", "w", "u", "d", "j/k"] {
            if lines.len() <= 3 {
                break;
            }
            for row in &mut rows {
                row.retain(|(key, _)| *key != omit);
            }
            lines = wrap_shortcuts(&rows, width);
        }
        return lines;
    }
    let columns = [
        [("j/k", "Navigate"), ("a", "Add"), ("d", "Delete")],
        [("g/G", "First/last"), ("e", "Edit"), ("x", "Clear done")],
        [("J/K", "Move"), ("Space", "Toggle"), ("D", "Clear all")],
        [("w", "Wrap"), ("u", "Undo"), ("q", "Quit")],
    ];
    let mut rows = [
        vec![Span::raw(" ")],
        vec![Span::raw(" ")],
        vec![Span::raw(" ")],
    ];
    let column_count = columns.len();
    for (index, column) in columns.into_iter().enumerate() {
        let gap = if index + 1 == column_count { "" } else { "  " };
        let key_width = column.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
        let label_width = column
            .iter()
            .map(|(_, label)| label.len())
            .max()
            .unwrap_or(0);
        for (row, (key, label)) in rows.iter_mut().zip(column) {
            if !available(key) {
                row.push(Span::raw(
                    " ".repeat(key_width + 1 + label_width + gap.len()),
                ));
                continue;
            }
            row.push(Span::styled(format!("{key:<key_width$}"), keys));
            row.push(Span::raw(format!(" {label:<label_width$}{gap}")));
        }
    }
    let [first, second, third] = rows;
    vec![Line::from(first), Line::from(second), Line::from(third)]
}

fn wrap_shortcuts(rows: &[Vec<(&str, &str)>], width: u16) -> Vec<Line<'static>> {
    let keys = Style::default().fg(TEXT).add_modifier(Modifier::BOLD);
    let column_count = rows.iter().map(Vec::len).max().unwrap_or(0);
    if column_count == 0 {
        return Vec::new();
    }
    let mut widths = vec![(0, 0); column_count];
    for row in rows {
        for (column, (key, label)) in row.iter().enumerate() {
            widths[column].0 = widths[column].0.max(key.width());
            widths[column].1 = widths[column].1.max(label.width());
        }
    }
    let aligned_width = 1
        + widths
            .iter()
            .map(|(key, label)| key + 1 + label)
            .sum::<usize>()
        + 2 * (column_count - 1);
    if aligned_width <= usize::from(width) {
        return rows
            .iter()
            .filter(|row| !row.is_empty())
            .map(|row| {
                let mut line = Line::raw(" ");
                for (column, (key, label)) in row.iter().enumerate() {
                    let (key_width, label_width) = widths[column];
                    line.push_span(Span::styled(format!("{key:<key_width$}"), keys));
                    line.push_span(Span::raw(format!(" {label}")));
                    if column + 1 < row.len() {
                        line.push_span(Span::raw(" ".repeat(label_width - label.width() + 2)));
                    }
                }
                line
            })
            .collect();
    }
    let mut lines = Vec::new();
    for row in rows {
        let mut line = Line::raw(" ");
        for &(key, label) in row {
            let pair_width = key.width() + 1 + label.width();
            if pair_width + 1 > usize::from(width) {
                continue;
            }
            let gap = if line.width() > 1 { "  " } else { "" };
            if line.width() + gap.len() + pair_width > usize::from(width) {
                lines.push(line);
                line = Line::raw(" ");
            } else {
                line.push_span(Span::raw(gap));
            }
            line.push_span(Span::styled(key.to_owned(), keys));
            line.push_span(Span::raw(format!(" {label}")));
        }
        if line.width() > 1 {
            lines.push(line);
        }
    }
    lines
}

fn draw_input(frame: &mut Frame, area: Rect, input: &TextInput, label: &str, accent: Color) {
    if area.is_empty() {
        return;
    }
    let mut line = Line::from(vec![
        Span::styled(label, Style::default().fg(MUTED)),
        Span::raw(" "),
        Span::styled("› ", Style::default().fg(accent)),
    ]);
    // Use a compact prompt when the full label would leave no cursor cell.
    if usize::from(area.width) <= line.width() {
        line = if area.width >= 3 {
            Line::from(Span::styled("› ", Style::default().fg(accent)))
        } else {
            Line::default()
        };
    }
    let prefix_width = line.width();
    let capacity = usize::from(area.width).saturating_sub(prefix_width + 1);
    // Keep whole graphemes visible and reserve one cell for the cursor.
    let text = input.text();
    let cursor = input.cursor();
    let before_capacity = capacity - text[cursor..].width().min(capacity / 2);
    let mut start = cursor;
    let mut width = 0;
    for (index, grapheme) in text[..cursor].grapheme_indices(true).rev() {
        let grapheme_width = grapheme.width();
        if width + grapheme_width > before_capacity {
            break;
        }
        width += grapheme_width;
        start = index;
    }
    let mut end = cursor;
    let mut visible_width = width;
    for grapheme in text[cursor..].graphemes(true) {
        let size = grapheme.width();
        if prefix_width + visible_width + size > usize::from(area.width) {
            break;
        }
        visible_width += size;
        end += grapheme.len();
    }
    let visible = &text[start..end];
    line.push_span(Span::styled(visible, Style::default().fg(TEXT)));
    frame.render_widget(Paragraph::new(line), area);
    let cursor = (prefix_width + width).min(usize::from(area.width - 1));
    frame.set_cursor_position((area.x + cursor as u16, area.y));
}

fn draw_confirmation(frame: &mut Frame, area: Rect) {
    let width = area.width.min(40);
    let height = area.height.min(7);
    let dialog = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, dialog);
    let block = Block::bordered().border_style(Style::default().fg(BORDER));
    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);
    let lines = if inner.height >= 5 {
        vec![
            Line::raw(""),
            Line::raw("Clear all tasks?"),
            Line::raw(""),
            Line::raw("Enter: Confirm    Esc: Cancel"),
        ]
    } else if inner.height >= 2 {
        vec![
            Line::raw("Clear all tasks?"),
            Line::raw("Enter: Confirm  Esc: Cancel"),
        ]
    } else {
        vec![Line::raw("Clear all tasks?")]
    };
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(Style::default().fg(TEXT)),
        inner,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Task;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn legends_follow_mode_availability_and_terminal_width() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = App::new(vec![Task {
            text: "task".into(),
            completed: false,
        }]);
        let text = |app: &App, width| {
            shortcut_lines(app, width)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(!text(&app, 80).contains("Undo"));
        assert!(!text(&app, 80).contains("Clear done"));
        app.key(
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
            &fixture.storage,
        )
        .unwrap();
        for width in [80, 60, 40, 30, 18, 1, 0] {
            let lines = shortcut_lines(&app, width);
            assert!(lines.len() <= 3);
            assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
        }
        for width in [80, 60] {
            let normal = text(&app, width);
            for key in [
                "j/k",
                "g/G",
                "J/K",
                "Space",
                "Undo",
                "Clear done",
                "Clear all",
                "Quit",
                "Wrap",
            ] {
                assert!(normal.contains(key), "{normal}");
            }
        }
        assert_eq!(
            text(&app, 40),
            " j/k Navigate    Space Toggle     w Wrap\n a/e Add/edit    d     Delete     u Undo\n x   Clear done  D     Clear all  q Quit"
        );
        for mode in [
            Mode::Adding(TextInput::default()),
            Mode::Editing {
                index: 0,
                input: "task".into(),
            },
        ] {
            app.mode = mode;
            assert_eq!(text(&app, 40), " Enter Confirm  Esc Cancel");
        }
        app.mode = Mode::ConfirmClear;
        assert!(shortcut_lines(&app, 80).is_empty());
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let mut rows = Rect::default();
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        assert_eq!(rows.height, 10);
        assert_eq!(screen(&terminal).matches("Confirm").count(), 1);
        assert_eq!(screen(&terminal).matches("Cancel").count(), 1);
        app.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &fixture.storage,
        )
        .unwrap();
        assert_eq!(text(&app, 40), " u Undo  w Wrap  q Quit");
        assert_eq!(text(&App::new(Vec::new()), 40), " w Wrap  q Quit");
        app.keybinds = false;
        assert!(shortcut_lines(&app, 80).is_empty());
        app.mode = Mode::Adding(TextInput::default());
        assert!(shortcut_lines(&app, 80).is_empty());
    }

    #[test]
    fn changing_footer_height_preserves_selection_and_mouse_mapping() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = App::new(
            (0..12)
                .map(|index| Task {
                    text: format!("task {index} {}", "wrapped words ".repeat(4)),
                    completed: false,
                })
                .collect(),
        );
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        app.list.select(Some(11));
        let mut heights = Vec::new();
        for mode in [
            Mode::Normal,
            Mode::Adding(TextInput::default()),
            Mode::Editing {
                index: 11,
                input: "task".into(),
            },
            Mode::Normal,
        ] {
            app.mode = mode;
            let mut rows = Rect::default();
            terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
            heights.push(rows.height);
            let end = (app.scroll + usize::from(rows.height)).min(app.row_tasks.len());
            assert!(app.row_tasks[app.scroll..end].contains(&11));
            if app.mode == Mode::Normal {
                let index = app.row_tasks[app.scroll];
                let was_completed = app.tasks[index].completed;
                app.mouse(
                    MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column: rows.x,
                        row: rows.y,
                        modifiers: KeyModifiers::NONE,
                    },
                    rows,
                    &fixture.storage,
                )
                .unwrap();
                assert_eq!(app.tasks[index].completed, !was_completed);
                app.list.select(Some(11));
                app.reveal_selection = true;
            }
        }
        assert!(heights[1] > heights[0]);
        assert_eq!(heights[1], heights[2]);
        assert_eq!(heights[0], heights[3]);
        app.keybinds = false;
        let mut rows = Rect::default();
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        assert_eq!(rows.height, 10);
        app.mode = Mode::Adding(TextInput::default());
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        assert_eq!(rows.height, 9);
    }

    #[test]
    fn tiny_terminals_and_resizing_are_safe_in_every_mode() {
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        let mut app = App::new(vec![Task {
            text: "日本語 🪷".into(),
            completed: false,
        }]);
        for width in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 40, 60, 80] {
            for height in [0, 1, 2, 3, 6, 12] {
                terminal.backend_mut().resize(width, height);
                for mode in [
                    Mode::Normal,
                    Mode::Adding("Long input 日本語 🪷".into()),
                    Mode::Editing {
                        index: 0,
                        input: "Existing text 日本語 🪷".into(),
                    },
                    Mode::ConfirmClear,
                ] {
                    app.mode = mode;
                    terminal
                        .draw(|frame| {
                            draw(frame, &mut app);
                        })
                        .unwrap();
                }
            }
        }
    }

    #[test]
    fn page_navigation_targets_visual_rows_in_mixed_task_heights() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = Terminal::new(TestBackend::new(18, 5)).unwrap();
        let mut app = App::new(
            [
                "short",
                "abcd abcd abcd",
                "short",
                "abcd abcd abcd abcd abcd",
                "short",
            ]
            .map(|text| Task {
                text: text.into(),
                completed: false,
            })
            .into(),
        );
        app.keybinds = false;
        let mut rows = Rect::default();
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        assert_eq!(rows.height, 3);
        assert_eq!(app.row_tasks, [0, 1, 1, 2, 3, 3, 3, 4]);
        for (code, selected) in [
            (KeyCode::PageDown, 2),
            (KeyCode::PageDown, 3),
            (KeyCode::PageDown, 4),
            (KeyCode::PageUp, 3),
            (KeyCode::PageUp, 1),
            (KeyCode::PageUp, 0),
        ] {
            app.key(KeyEvent::new(code, KeyModifiers::NONE), &fixture.storage)
                .unwrap();
            assert_eq!(app.list.selected(), Some(selected));
            terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
            let end = (app.scroll + usize::from(rows.height)).min(app.row_tasks.len());
            assert!(app.row_tasks[app.scroll..end].contains(&selected));
        }
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn page_navigation_uses_current_viewport_wrapping_and_resize() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        let mut app = App::new(
            (0..40)
                .map(|index| Task {
                    text: format!("task {index}"),
                    completed: false,
                })
                .collect(),
        );
        // Include the real footer so page size excludes its rows as well as borders.
        let mut rows = Rect::default();
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        assert_eq!(app.viewport_height, rows.height);
        assert!(rows.height < 10);
        let original = app.tasks.clone();
        let mut press = |app: &mut App, code, kind| {
            app.key(
                KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind),
                &fixture.storage,
            )
            .unwrap();
            terminal.draw(|frame| rows = draw(frame, app)).unwrap();
        };
        let height = usize::from(app.viewport_height);
        press(&mut app, KeyCode::PageDown, KeyEventKind::Press);
        assert_eq!(app.list.selected(), Some(height));
        assert_eq!(app.scroll, height);
        press(&mut app, KeyCode::PageDown, KeyEventKind::Repeat);
        assert_eq!(app.list.selected(), Some(2 * height));
        press(&mut app, KeyCode::PageUp, KeyEventKind::Repeat);
        assert_eq!(app.list.selected(), Some(height));
        press(&mut app, KeyCode::PageDown, KeyEventKind::Release);
        assert_eq!(app.list.selected(), Some(height));
        for _ in 0..50 {
            press(&mut app, KeyCode::PageDown, KeyEventKind::Press);
        }
        assert_eq!(app.list.selected(), Some(39));
        for _ in 0..50 {
            press(&mut app, KeyCode::PageUp, KeyEventKind::Press);
        }
        assert_eq!(app.list.selected(), Some(0));
        assert_eq!(app.tasks, original);
        assert!(!app.can_undo());
        assert!(!fixture.task_path().exists());
        drop(press);

        app.keybinds = false;
        app.tasks[1].text = "word ".repeat(60);
        terminal.backend_mut().resize(18, 8);
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        let page = KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE);
        let up = KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE);
        app.key(page, &fixture.storage).unwrap();
        assert_eq!(app.list.selected(), Some(1));
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        // Simulate reading partway through the oversized selected task with the mouse wheel.
        app.scroll += 5;
        app.reveal_selection = false;
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        app.key(page, &fixture.storage).unwrap();
        assert_eq!(app.list.selected(), Some(2));
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        app.key(up, &fixture.storage).unwrap();
        assert_eq!(app.list.selected(), Some(1));
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        app.key(up, &fixture.storage).unwrap();
        assert_eq!(app.list.selected(), Some(0));

        for (width, height, wrap) in [(18, 8, false), (40, 10, true), (30, 6, false), (1, 1, true)]
        {
            if app.wrap != wrap {
                app.key(
                    KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
                    &fixture.storage,
                )
                .unwrap();
            }
            app.list.select(Some(0));
            app.scroll = 0;
            terminal.backend_mut().resize(width, height);
            terminal
                .draw(|frame| {
                    draw(frame, &mut app);
                })
                .unwrap();
            let expected = usize::from(app.viewport_height.max(1));
            app.key(page, &fixture.storage).unwrap();
            // Only task 1 wraps: with wrapping off, or a one-cell width, each task has one row.
            assert_eq!(
                app.list.selected(),
                Some(if wrap && width > 6 { 1 } else { expected })
            );
            terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
            if rows.height > 0 {
                let end = (app.scroll + usize::from(rows.height)).min(app.row_tasks.len());
                assert!(app.row_tasks[app.scroll..end].contains(&app.list.selected().unwrap()));
            }
        }
    }

    #[test]
    fn long_lists_scroll_to_selection_and_wrap_back_to_start() {
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let mut app = App::new(
            (0..1000)
                .map(|index| Task {
                    text: format!("task {index}"),
                    completed: false,
                })
                .collect(),
        );
        app.navigate(false);
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert!(app.scroll > 0);
        assert!(screen(&terminal).contains("task 999"));
        app.navigate(true);
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert_eq!(app.scroll, 0);
        assert!(screen(&terminal).contains("task 0"));
    }

    #[test]
    fn completed_text_stays_muted_with_accented_marker_and_checkbox() {
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let mut app = App::new(vec![Task {
            text: "Completed".into(),
            completed: true,
        }]);
        app.accent = Color::Rgb(169, 182, 101);
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let marker = &buffer[(1, 1)];
        let checkbox = &buffer[(3, 1)];
        let text = &buffer[(7, 1)];
        assert_eq!(marker.symbol(), "›");
        assert_eq!(marker.fg, app.accent);
        assert_eq!(marker.bg, Color::Reset);
        assert_eq!(checkbox.fg, app.accent);
        assert_eq!(text.fg, MUTED);
        assert!(text.modifier.contains(Modifier::CROSSED_OUT));
        assert!(!text.modifier.contains(Modifier::DIM));
        assert_eq!(text.bg, Color::Reset);
    }

    #[test]
    fn disabled_wrapping_keeps_one_row_and_input_shows_its_tail() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = Terminal::new(TestBackend::new(12, 8)).unwrap();
        let mut app = App::new(vec![
            Task {
                text: "日本語".repeat(100),
                completed: false,
            },
            Task {
                text: "next".into(),
                completed: false,
            },
        ]);
        let toggle = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE);
        app.key(toggle, &fixture.storage).unwrap();
        assert!(!app.wrap);
        app.mode = Mode::Adding("a long input ending in 界z".into());
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        let rendered = screen(&terminal);
        assert!(rendered.contains("[ ] next"));
        assert!(rendered.contains("界z"));
        app.mode = Mode::Normal;
        app.key(toggle, &fixture.storage).unwrap();
        assert!(app.wrap);
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert!(app.row_tasks.len() > 2);
        assert_eq!(app.list.selected(), Some(0));
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn input_cursor_in_the_middle_keeps_both_sides_visible() {
        for label in ["add", "edit"] {
            let mut input = TextInput::from("hidden text e\u{301}界👩‍💻after");
            input.set_cursor("hidden text e\u{301}界".len());
            for width in 0..=30 {
                let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
                terminal
                    .draw(|frame| draw_input(frame, frame.area(), &input, label, DEFAULT_ACCENT))
                    .unwrap();
                if width == 0 {
                    continue;
                }
                let position = terminal.get_cursor_position().unwrap();
                let cursor = position.x;
                assert_eq!(position.y, 0);
                assert!(cursor < width);
                if width >= 16 {
                    let rendered = screen(&terminal);
                    assert!(rendered.contains("界👩‍💻"), "{rendered}");
                    let before_cursor = rendered.split("👩‍💻").next().unwrap();
                    assert_eq!(usize::from(cursor), before_cursor.width());
                }
            }
            input.set_cursor(0);
            let mut terminal = Terminal::new(TestBackend::new(40, 1)).unwrap();
            terminal
                .draw(|frame| draw_input(frame, frame.area(), &input, label, DEFAULT_ACCENT))
                .unwrap();
            assert_eq!(
                usize::from(terminal.get_cursor_position().unwrap().x),
                label.width() + 3
            );
            assert!(screen(&terminal).contains("hidden text"));
        }
    }

    #[test]
    fn input_tail_and_cursor_follow_graphemes_and_cell_widths() {
        for label in ["add", "edit"] {
            for tail in ["e\u{301}", "👩‍💻", "👨‍👩‍👧‍👦", "👍🏽", "🇺🇳", "👩‍💻a"]
            {
                let prefix_width = label.width() + 3;
                let cursor = (prefix_width + tail.width()) as u16;
                let mut terminal = Terminal::new(TestBackend::new(cursor + 1, 1)).unwrap();
                let input = TextInput::from(format!("hidden text {tail}"));
                let accent = Color::Rgb(169, 182, 101);
                terminal
                    .draw(|frame| draw_input(frame, frame.area(), &input, label, accent))
                    .unwrap();
                assert_eq!(screen(&terminal).trim_end(), format!("{label} › {tail}"));
                assert_eq!(terminal.get_cursor_position().unwrap(), (cursor, 0).into());
                let buffer = terminal.backend().buffer();
                assert_eq!(buffer[(cursor, 0)].symbol(), " ");
                assert_eq!(buffer[(0, 0)].fg, MUTED);
                assert_eq!(buffer[((label.width() + 1) as u16, 0)].fg, accent);
                assert_eq!(buffer[(prefix_width as u16, 0)].fg, TEXT);
            }
        }
    }

    #[test]
    fn narrow_input_viewports_hide_glyphs_that_cannot_fit_without_splitting_them() {
        for (label, expected_rows) in [
            (
                "add",
                [
                    "",
                    "",
                    "",
                    "›",
                    "›",
                    "› 👩‍💻",
                    "›  👩‍💻",
                    "add ›",
                    "add ›",
                    "add › 👩‍💻",
                    "add ›  👩‍💻",
                ],
            ),
            (
                "edit",
                [
                    "",
                    "",
                    "",
                    "›",
                    "›",
                    "› 👩‍💻",
                    "›  👩‍💻",
                    "› n 👩‍💻",
                    "edit ›",
                    "edit ›",
                    "edit › 👩‍💻",
                ],
            ),
        ] {
            for (width, expected) in expected_rows.into_iter().enumerate() {
                let width = width as u16;
                for height in 0..=1 {
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal
                        .draw(|frame| {
                            draw_input(
                                frame,
                                frame.area(),
                                &TextInput::from("hidden 👩‍💻"),
                                label,
                                DEFAULT_ACCENT,
                            )
                        })
                        .unwrap();
                    if width == 0 || height == 0 {
                        continue;
                    }
                    assert_eq!(screen(&terminal).trim_end(), expected);
                    let cursor = terminal.get_cursor_position().unwrap();
                    assert!(cursor.x < width);
                    assert_eq!(cursor.y, 0);
                    assert_eq!(
                        terminal.backend().buffer()[(cursor.x, cursor.y)].symbol(),
                        " "
                    );
                }
            }
        }
    }

    #[test]
    fn wrapping_preserves_words_and_unicode_clusters() {
        assert_eq!(wrap_text("alpha beta gamma", 10), ["alpha beta", "gamma"]);
        assert_eq!(wrap_text("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(wrap_text("日本語", 4), ["日本", "語"]);
        assert_eq!(wrap_text("e\u{301}e\u{301}", 1), ["e\u{301}", "e\u{301}"]);
        assert_eq!(wrap_text("👩‍💻👩‍💻", 2), ["👩‍💻", "👩‍💻"]);
        assert_eq!(wrap_text("界a", 1), ["�", "a"]);
        assert_eq!(wrap_text("", 4), [""]);
    }

    #[test]
    fn continuation_clicks_toggle_the_correct_task_and_leave_other_rows_alone() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = Terminal::new(TestBackend::new(18, 12)).unwrap();
        let mut app = App::new(vec![
            Task {
                text: "alpha beta gamma delta".into(),
                completed: false,
            },
            Task {
                text: "next".into(),
                completed: false,
            },
        ]);
        let mut rows = Rect::default();
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        assert_eq!(app.row_tasks, [0, 0, 0, 1]);
        let rendered = screen(&terminal);
        assert!(rendered.contains("› [ ] alpha beta"));
        assert!(rendered.contains("      gamma"));
        assert!(rendered.contains("      delta"));
        assert!(rendered.contains("  [ ] next"));
        assert_eq!(rendered.matches('›').count(), 1);
        assert_eq!(terminal.backend().buffer()[(1, 1)].fg, DEFAULT_ACCENT);
        let click = |row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 6,
            row,
            modifiers: KeyModifiers::NONE,
        };
        app.mouse(click(2), rows, &fixture.storage).unwrap();
        assert!(app.tasks[0].completed);
        assert!(!app.tasks[1].completed);
        app.mouse(click(4), rows, &fixture.storage).unwrap();
        assert!(app.tasks[1].completed);
        app.mouse(click(5), rows, &fixture.storage).unwrap();
        assert!(app.tasks.iter().all(|task| task.completed));
        assert_eq!(fixture.storage.load().unwrap(), app.tasks);
    }

    #[test]
    fn oversized_task_can_be_scrolled_read_and_navigated_after_resizing() {
        use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = Terminal::new(TestBackend::new(18, 8)).unwrap();
        let mut app = App::new(vec![
            Task {
                text: format!("{}THE END", "word ".repeat(30)),
                completed: false,
            },
            Task {
                text: "next".into(),
                completed: false,
            },
        ]);
        let mut rows = Rect::default();
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        let total = app.row_tasks.len();
        assert!(total > usize::from(rows.height));
        let wheel = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: rows.x,
            row: rows.y,
            modifiers: KeyModifiers::NONE,
        };
        for _ in 0..total {
            app.mouse(wheel, rows, &fixture.storage).unwrap();
            terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        }
        assert!(screen(&terminal).contains("THE END"));
        assert!(screen(&terminal).contains("next"));
        app.navigate(true);
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        assert_eq!(app.list.selected(), Some(1));
        app.navigate(false);
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        assert_eq!(app.scroll, 0);
        assert!(screen(&terminal).contains("[ ] word"));
        terminal.backend_mut().resize(30, 12);
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert!(app.row_tasks.len() < total);
        assert!(screen(&terminal).contains("[ ] word"));
        assert!(!fixture.task_path().exists());
    }

    fn screen(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                let mut line = String::new();
                let mut x = 0;
                while x < buffer.area.width {
                    let symbol = buffer[(x, y)].symbol();
                    line.push_str(symbol);
                    // Wide characters occupy subsequent cells that contain placeholders.
                    x += symbol.width().max(1) as u16;
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
