use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Mode};

// Gruvbox Material dark/medium; the terminal supplies the background.
const TEXT: Color = Color::Rgb(212, 190, 152);
const MUTED: Color = Color::Rgb(146, 131, 116);
const ACCENT: Color = Color::Rgb(231, 138, 78);
const BORDER: Color = Color::Rgb(80, 73, 69);

pub fn draw(frame: &mut Frame, app: &mut App) -> Rect {
    let area = frame.area();
    let adding = matches!(app.mode, Mode::Adding(_));
    // Give the checklist priority when height is limited; input always gets a row.
    let input_height = u16::from(adding && area.height > 0);
    let legend_height = if area.height >= 6 + input_height {
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

    if app.tasks.is_empty() {
        frame.render_widget(
            Paragraph::new(" Press a to add a task").style(Style::default().fg(MUTED)),
            rows,
        );
    } else {
        let items = app.tasks.iter().enumerate().map(|(index, task)| {
            let selected = app.list.selected() == Some(index);
            let checkbox = if task.completed { "[x] " } else { "[ ] " };
            let text_style = if task.completed {
                Style::default()
                    .fg(MUTED)
                    .add_modifier(Modifier::DIM | Modifier::CROSSED_OUT)
            } else {
                Style::default().fg(TEXT)
            };
            // A JSON file edited by hand still renders each task on exactly one row.
            let text: String = task
                .text
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            ListItem::new(Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    checkbox,
                    Style::default().fg(if selected {
                        ACCENT
                    } else if task.completed {
                        MUTED
                    } else {
                        TEXT
                    }),
                ),
                Span::styled(text, text_style),
            ]))
        });
        frame.render_stateful_widget(List::new(items), rows, &mut app.list);
    }

    if let Mode::Adding(input) = &app.mode {
        draw_input(frame, sections[1], input);
    }
    frame.render_widget(
        Paragraph::new("\n j/k Navigate   Space/Enter Toggle   a Add\n d Delete       D Clear All          q Quit")
            .style(Style::default().fg(MUTED)),
        sections[2],
    );
    if app.mode == Mode::ConfirmClear {
        draw_confirmation(frame, area);
    }
    rows
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
        assert_eq!(checkbox.fg, ACCENT);
        assert_eq!(text.fg, MUTED);
        assert!(
            text.modifier
                .contains(Modifier::DIM | Modifier::CROSSED_OUT)
        );
        assert_eq!(text.bg, Color::Reset);
    }

    #[test]
    fn long_unicode_tasks_stay_on_one_row_and_input_shows_its_tail() {
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
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        let rendered = screen(&terminal);
        assert!(rendered.contains("[ ] next"));
        assert!(rendered.contains("界z"));
    }

    fn screen(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
