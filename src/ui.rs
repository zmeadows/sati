use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Mode};

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
    let shortcuts = shortcut_lines();
    let legend_width = shortcuts.iter().map(Line::width).max().unwrap_or(0);
    let legend_height =
        if area.height >= 6 + input_height && usize::from(area.width) >= legend_width {
            3
        } else {
            0
        };
    let sections = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(input_height),
        Constraint::Length(legend_height),
    ])
    .split(area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER));
    let rows = block.inner(sections[0]);
    frame.render_widget(block, sections[0]);

    draw_tasks(frame, app, rows);

    if let Mode::Adding(input) | Mode::Editing { input, .. } = &app.mode {
        draw_input(frame, sections[1], input);
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
    let text_width = usize::from(area.width.saturating_sub(5));
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
                .add_modifier(Modifier::DIM | Modifier::CROSSED_OUT)
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
    *app.list.offset_mut() = app.row_tasks[app.scroll];
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

fn shortcut_lines() -> Vec<Line<'static>> {
    let keys = Style::default().fg(TEXT).add_modifier(Modifier::BOLD);
    let columns = [
        [("j/k", "Navigate"), ("J/K", "Move")],
        [("Space", "Toggle"), ("d", "Delete")],
        [("a", "Add"), ("D", "Clear")],
        [("q", "Quit"), ("e", "Edit")],
        [("u", "Undo"), ("x", "Clear done")],
    ];
    let mut rows = [vec![Span::raw(" ")], vec![Span::raw(" ")]];
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
            row.push(Span::styled(format!("{key:<key_width$}"), keys));
            row.push(Span::raw(format!(" {label:<label_width$}{gap}")));
        }
    }
    let [first, second] = rows;
    vec![Line::raw(""), Line::from(first), Line::from(second)]
}

fn draw_input(frame: &mut Frame, area: Rect, input: &str) {
    if area.is_empty() {
        return;
    }
    let prefix = if area.width >= 3 { "> " } else { "" };
    let capacity = usize::from(area.width).saturating_sub(prefix.len() + 1);
    // Show the tail without splitting UTF-8 or double-width characters.
    let mut start = input.len();
    let mut width = 0;
    for (index, character) in input.char_indices().rev() {
        let character_width = unicode_width::UnicodeWidthChar::width(character).unwrap_or(0);
        if width + character_width > capacity {
            break;
        }
        width += character_width;
        start = index;
    }
    let visible = &input[start..];
    frame.render_widget(
        Paragraph::new(format!("{prefix}{visible}")).style(Style::default().fg(TEXT)),
        area,
    );
    let cursor = (prefix.len() + visible.width()).min(usize::from(area.width - 1));
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
    fn tiny_terminals_and_resizing_are_safe_in_every_mode() {
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        let mut app = App::new(vec![Task {
            text: "日本語 🪷".into(),
            completed: false,
        }]);
        for width in [0, 1, 2, 3, 8, 40] {
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
        app.navigate(false, true);
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert!(app.list.offset() > 0);
        assert!(screen(&terminal).contains("task 999"));
        app.navigate(true, true);
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert_eq!(app.list.offset(), 0);
        assert!(screen(&terminal).contains("task 0"));
    }

    #[test]
    fn completed_text_is_dimmed_and_crossed_out_with_only_checkbox_accented() {
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let mut app = App::new(vec![Task {
            text: "Completed".into(),
            completed: true,
        }]);
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let checkbox = &buffer[(2, 1)];
        let text = &buffer[(6, 1)];
        assert_eq!(checkbox.fg, DEFAULT_ACCENT);
        assert_eq!(text.fg, MUTED);
        assert!(
            text.modifier
                .contains(Modifier::DIM | Modifier::CROSSED_OUT)
        );
        assert_eq!(text.bg, Color::Reset);
    }

    #[test]
    fn disabled_wrapping_keeps_one_row_and_input_shows_its_tail() {
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
        app.mode = Mode::Adding("a long input ending in 界z".into());
        app.wrap = false;
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        let rendered = screen(&terminal);
        assert!(rendered.contains("[ ] next"));
        assert!(rendered.contains("界z"));
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
        let mut terminal = Terminal::new(TestBackend::new(18, 9)).unwrap();
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
        assert_eq!(app.row_tasks, [0, 0, 1]);
        assert!(screen(&terminal).contains("     gamma delta"));
        let click = |row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 6,
            row,
            modifiers: KeyModifiers::NONE,
        };
        app.mouse(click(2), rows, &fixture.storage).unwrap();
        assert!(app.tasks[0].completed);
        assert!(!app.tasks[1].completed);
        app.mouse(click(3), rows, &fixture.storage).unwrap();
        assert!(app.tasks[1].completed);
        app.mouse(click(4), rows, &fixture.storage).unwrap();
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
        app.navigate(true, true);
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        assert_eq!(app.list.selected(), Some(1));
        app.navigate(false, true);
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
