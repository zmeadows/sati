use std::ops::Range;

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
    let searching = matches!(
        app.mode,
        Mode::Searching { .. } | Mode::SearchResults { .. }
    );
    // Only search uses a separate input row; add/edit live inside the checklist.
    let input_height = u16::from(searching && area.height > 0);
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
        block = block.title(Line::styled(" sati ", Style::default().fg(MUTED)).right_aligned());
    }
    let rows = block.inner(sections[0]);
    frame.render_widget(block, sections[0]);

    draw_tasks(frame, app, rows);

    match &app.mode {
        Mode::Searching { input, .. } | Mode::SearchResults { input, .. } => {
            let no_matches = !input.text().is_empty() && app.list.selected().is_none();
            let label = if no_matches && sections[1].width >= 28 {
                "search (No matches)"
            } else {
                "search"
            };
            draw_input_line(
                frame,
                sections[1],
                input,
                label,
                app.accent,
                matches!(app.mode, Mode::Searching { .. }),
            );
        }
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
    let inline = match &app.mode {
        Mode::Adding(input, placement) => Some((app.insertion_index(*placement), input, "add")),
        Mode::Editing { index, input } => Some((*index, input, "edit")),
        _ => None,
    };
    let editing = matches!(app.mode, Mode::Editing { .. });
    let selected = if inline.is_some() {
        None
    } else {
        app.list.selected()
    };
    if app.tasks.is_empty() && inline.is_none() {
        app.scroll = 0;
        frame.render_widget(
            Paragraph::new(" Press a to add a task").style(Style::default().fg(MUTED)),
            area,
        );
        return;
    }
    let mut lines = Vec::new();
    let mut input_row = None;
    let text_width = usize::from(area.width.saturating_sub(6));
    for index in 0..=app.tasks.len() {
        if let Some((position, _, _)) = inline
            && position == index
        {
            input_row = Some(lines.len());
            lines.push(Line::default());
            app.row_tasks.push(editing.then_some(index));
            if editing {
                continue;
            }
        }
        let Some(task) = app.tasks.get(index) else {
            break;
        };
        let text: String = task
            .text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let tag_end = text
            .find(':')
            .filter(|&end| end > 0 && !text[..end].chars().any(char::is_whitespace))
            .map_or(0, |end| {
                text.grapheme_indices(true)
                    .take_while(|(index, _)| *index <= end)
                    .last()
                    .map_or(0, |(index, grapheme)| index + grapheme.len())
            });
        let searching = app.search_query().is_some();
        let matches = app
            .search_query()
            .map_or_else(Vec::new, |query| crate::search::match_ranges(&text, query));
        let wrapped = if app.wrap && text_width > 0 {
            wrap_text(&text, text_width)
        } else {
            vec![(0..text.len(), text.as_str())]
        };
        let text_style = if task.completed {
            Style::default()
                .fg(MUTED)
                .add_modifier(Modifier::CROSSED_OUT)
        } else {
            Style::default().fg(TEXT)
        };
        for (row, (source, text)) in wrapped.into_iter().enumerate() {
            app.row_tasks.push(Some(index));
            let prefix = if row > 0 {
                "     "
            } else if searching {
                if matches.is_empty() {
                    "  □  "
                } else {
                    "  ■  "
                }
            } else if task.completed {
                " [x] "
            } else if task.carried {
                " [>] "
            } else {
                " [ ] "
            };
            let color = if searching {
                if matches.is_empty() {
                    MUTED
                } else {
                    app.accent
                }
            } else if selected == Some(index) {
                app.accent
            } else if task.completed {
                MUTED
            } else {
                TEXT
            };
            let mut line = Line::from(vec![
                Span::styled(
                    if row == 0 && selected == Some(index) {
                        "›"
                    } else {
                        " "
                    },
                    Style::default().fg(app.accent),
                ),
                Span::styled(prefix, Style::default().fg(color)),
            ]);
            line.spans.extend(task_spans(
                text,
                source.start,
                tag_end,
                &matches,
                text_style,
                app.accent,
            ));
            lines.push(line);
        }
    }
    let height = usize::from(area.height);
    app.scroll = app.scroll.min(lines.len().saturating_sub(height));
    if height == 0 {
        return;
    }
    let focus = input_row.map(|row| (row, row + 1)).or_else(|| {
        selected.map(|selected| {
            let start = app
                .row_tasks
                .iter()
                .position(|&task| task == Some(selected))
                .unwrap_or(0);
            let end = app
                .row_tasks
                .iter()
                .rposition(|&task| task == Some(selected))
                .unwrap_or(start)
                + 1;
            (start, end)
        })
    });
    if let Some((start, end)) = focus
        && (app.reveal_selection || end <= app.scroll || start >= app.scroll + height)
    {
        if end - start > height || start < app.scroll {
            app.scroll = start;
        } else if end > app.scroll + height {
            app.scroll = end - height;
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
    if let Some((_, input, label)) = inline
        && let Some(row) = input_row
        && row >= app.scroll
        && row - app.scroll < height
    {
        let indent = area.width.saturating_sub(1).min(2);
        draw_input(
            frame,
            Rect::new(
                area.x + indent,
                area.y + (row - app.scroll) as u16,
                area.width - indent,
                1,
            ),
            input,
            label,
            app.accent,
        );
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<(Range<usize>, &str)> {
    let mut lines = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        let offset = text.len() - remaining.len();
        let mut used = 0;
        let mut word_break = None;
        let mut split = None;
        for (index, grapheme) in remaining.grapheme_indices(true) {
            let size = grapheme.width();
            if used + size > width {
                // A glyph wider than the entire text column cannot be drawn intact.
                if index == 0 {
                    lines.push((offset..offset + grapheme.len(), "�"));
                    split = Some(grapheme.len());
                } else {
                    let boundary = if grapheme.chars().all(char::is_whitespace) {
                        index
                    } else {
                        word_break.filter(|&offset| offset > 0).unwrap_or(index)
                    };
                    split = Some(boundary);
                    // Trim only complete whitespace clusters, preserving marks attached to spaces.
                    let end = remaining[..boundary]
                        .grapheme_indices(true)
                        .rev()
                        .find(|(_, grapheme)| !grapheme.chars().all(char::is_whitespace))
                        .map_or(0, |(index, grapheme)| index + grapheme.len());
                    lines.push((offset..offset + end, &remaining[..end]));
                }
                break;
            }
            if grapheme.chars().all(char::is_whitespace) {
                word_break = Some(index);
            }
            used += size;
        }
        let Some(split) = split else {
            lines.push((offset..text.len(), remaining));
            break;
        };
        remaining = &remaining[split..];
        let start = remaining
            .grapheme_indices(true)
            .find(|(_, grapheme)| !grapheme.chars().all(char::is_whitespace))
            .map_or(remaining.len(), |(index, _)| index);
        remaining = &remaining[start..];
    }
    if lines.is_empty() {
        lines.push((0..0, ""));
    }
    lines
}

fn highlight_foreground(accent: Color) -> Color {
    let Color::Rgb(r, g, b) = accent else {
        return Color::Rgb(0, 0, 0);
    };
    let linear = |value: u8| {
        let srgb = f64::from(value) / 255.0;
        if srgb <= 0.04045 {
            srgb / 12.92
        } else {
            ((srgb + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
    let black = (luminance + 0.05) / 0.05;
    let white = 1.05 / (luminance + 0.05);
    if black >= white {
        Color::Rgb(0, 0, 0)
    } else {
        Color::Rgb(255, 255, 255)
    }
}

fn task_spans(
    text: &str,
    offset: usize,
    tag_end: usize,
    matches: &[Range<usize>],
    base: Style,
    accent: Color,
) -> Vec<Span<'static>> {
    let highlight = base.bg(accent).fg(highlight_foreground(accent));
    let mut spans = Vec::new();
    let mut start = 0;
    let mut style = base;
    let mut matching = 0;
    for (index, _) in text.grapheme_indices(true) {
        let source = offset + index;
        while matching < matches.len() && matches[matching].end <= source {
            matching += 1;
        }
        let mut next = if matches
            .get(matching)
            .is_some_and(|range| range.contains(&source))
        {
            highlight
        } else {
            base
        };
        if source < tag_end {
            next = next.add_modifier(Modifier::BOLD);
        }
        if index > start && next != style {
            spans.push(Span::styled(text[start..index].to_owned(), style));
            start = index;
        }
        style = next;
    }
    spans.push(Span::styled(text[start..].to_owned(), style));
    spans
}

fn shortcut_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    if !app.keybinds || width == 0 || app.mode == Mode::ConfirmClear {
        return Vec::new();
    }
    let keys = Style::default().fg(TEXT).add_modifier(Modifier::BOLD);
    let extra_hints = |lines: &mut [Line<'static>]| {
        if app.mode == Mode::Normal
            && app.can_redo()
            && let Some(line) = lines
                .iter_mut()
                .find(|line| line.width() + "  Ctrl+R Redo".width() <= usize::from(width))
        {
            line.push_span(Span::raw("  "));
            line.push_span(Span::styled("Ctrl+R", keys));
            line.push_span(Span::raw(" Redo"));
        }
        if app.mode == Mode::Normal
            && app
                .list
                .selected()
                .is_some_and(|index| !app.tasks[index].completed)
            && let Some(line) = lines
                .iter_mut()
                .find(|line| line.width() + "  m Carry".width() <= usize::from(width))
        {
            line.push_span(Span::raw("  "));
            line.push_span(Span::styled("m", keys));
            line.push_span(Span::raw(" Carry"));
        }
    };
    let available = |key: &str| match key {
        "D" => app.tasks.iter().any(|task| !task.carried),
        "u" => app.can_undo(),
        "x" => app.tasks.iter().any(|task| task.completed),
        _ => true,
    };
    // The full aligned legend occupies 54 columns; pack smaller contexts by action.
    if app.mode != Mode::Normal || app.tasks.is_empty() || width < 54 {
        let mut rows: Vec<Vec<(&str, &str)>> = match app.mode {
            Mode::Searching { .. } => vec![vec![("Enter", "Results"), ("Esc", "Cancel")]],
            Mode::SearchResults { .. } => vec![vec![
                ("n/N", "Next/prev"),
                ("↑/↓", "Navigate"),
                ("Enter", "Select"),
                ("Esc", "Cancel"),
            ]],
            Mode::Adding(_, _) | Mode::Editing { .. } => {
                vec![vec![("Enter", "Confirm"), ("Esc", "Cancel")]]
            }
            _ if app.tasks.is_empty() => vec![vec![("u", "Undo"), ("w", "Wrap"), ("q", "Quit")]],
            _ => vec![
                vec![("j/k", "Navigate"), ("Space", "Toggle"), ("w", "Wrap")],
                vec![("a/e", "Add/edit"), ("d", "Delete"), ("u", "Undo")],
                vec![("x", "Clear done"), ("D", "Clear"), ("q", "Quit")],
            ],
        };
        for row in &mut rows {
            row.retain(|(key, _)| available(key));
        }
        let mut lines = wrap_shortcuts(&rows, width);
        // Keep at most three hint rows, dropping secondary actions first.
        for omit in ["x", "D", "w", "u", "d", "j/k", "↑/↓"] {
            if lines.len() <= 3 {
                break;
            }
            for row in &mut rows {
                row.retain(|(key, _)| *key != omit);
            }
            lines = wrap_shortcuts(&rows, width);
        }
        extra_hints(&mut lines);
        return lines;
    }
    let columns = [
        [("j/k", "Navigate"), ("a", "Add"), ("d", "Delete")],
        [("g/G", "First/last"), ("e", "Edit"), ("x", "Clear done")],
        [("J/K", "Move"), ("Space", "Toggle"), ("D", "Clear")],
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
    let mut lines = vec![Line::from(first), Line::from(second), Line::from(third)];
    if lines[0].width() + "  / Search".width() <= usize::from(width) {
        lines[0].push_span(Span::raw("  "));
        lines[0].push_span(Span::styled("/", keys));
        lines[0].push_span(Span::raw(" Search"));
    }
    extra_hints(&mut lines);
    lines
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
    draw_input_line(frame, area, input, label, accent, true);
}

fn draw_input_line(
    frame: &mut Frame,
    area: Rect,
    input: &TextInput,
    label: &str,
    accent: Color,
    show_cursor: bool,
) {
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
    if show_cursor {
        frame.set_cursor_position((area.x + cursor as u16, area.y));
    }
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
            Line::raw("Clear unmarked tasks?"),
            Line::raw(""),
            Line::raw("Enter: Confirm    Esc: Cancel"),
        ]
    } else if inner.height >= 2 {
        vec![
            Line::raw("Clear unmarked tasks?"),
            Line::raw("Enter: Confirm  Esc: Cancel"),
        ]
    } else {
        vec![Line::raw("Clear unmarked tasks?")]
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

    fn wrapped_strings(text: &str, width: usize) -> Vec<&str> {
        wrap_text(text, width)
            .into_iter()
            .map(|(_, line)| line)
            .collect()
    }
    use crate::app::{AddPlacement, Task};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn carryover_checkboxes_preserve_layout_and_search_markers() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = App::new(vec![Task {
            text: "carry words across wrapped lines".into(),
            completed: false,
            carried: true,
        }]);
        app.keybinds = false;
        app.accent = Color::Rgb(169, 182, 101);
        let mut terminal = Terminal::new(TestBackend::new(24, 12)).unwrap();
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        let mapping = app.row_tasks.clone();
        assert!(mapping.len() > 1);
        assert!(screen(&terminal).contains("› [>] carry"));
        assert_eq!(terminal.backend().buffer()[(3, 1)].fg, app.accent);
        assert_eq!(terminal.backend().buffer()[(1, 2)].symbol(), " ");
        app.key(
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
            &fixture.storage,
        )
        .unwrap();
        app.paste("carry");
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert_eq!(app.row_tasks, mapping);
        assert!(screen(&terminal).contains('■'));
        assert!(!screen(&terminal).contains("[>]"));
        app.key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &fixture.storage,
        )
        .unwrap();
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert_eq!(app.row_tasks, mapping);
        assert!(screen(&terminal).contains("[>]"));
        assert!(app.tasks[0].carried);
        app.keybinds = true;
        for width in [1, 18, 40, 60, 80] {
            let lines = shortcut_lines(&app, width);
            assert!(lines.len() <= 3);
            assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
        }
        assert!(
            shortcut_lines(&app, 80)
                .iter()
                .any(|line| line.to_string().contains("m Carry"))
        );
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn inline_add_preview_matches_saved_position_without_changing_tasks() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for count in [0usize, 4] {
            for grouped in [false, true] {
                for selected in [None, (count > 0).then_some(0), count.checked_sub(1)] {
                    for (key, placement) in [
                        ('a', AddPlacement::End),
                        ('o', AddPlacement::Below),
                        ('O', AddPlacement::Above),
                    ] {
                        let fixture = crate::storage::tests::Fixture::new();
                        let mut app = App::new(
                            (0..count)
                                .map(|index| Task {
                                    carried: false,
                                    text: format!("task {index}"),
                                    completed: if grouped { index >= 2 } else { index % 2 == 0 },
                                })
                                .collect(),
                        );
                        app.completed_at_bottom = grouped;
                        app.list.select(selected);
                        app.accent = Color::Rgb(169, 182, 101);
                        app.keybinds = false;
                        fixture.storage.save(&app.tasks).unwrap();
                        let original = app.tasks.clone();
                        let bytes = std::fs::read(fixture.task_path()).unwrap();
                        app.key(
                            KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE),
                            &fixture.storage,
                        )
                        .unwrap();
                        app.paste("New task");
                        let index = app.insertion_index(placement);
                        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
                        let mut rows = Rect::default();
                        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
                        let expected: Vec<_> = (0..index)
                            .map(Some)
                            .chain([None])
                            .chain((index..count).map(Some))
                            .collect();
                        assert_eq!(app.row_tasks, expected);
                        assert_eq!(app.tasks, original);
                        assert_eq!(app.list.selected(), selected);
                        assert!(!app.can_undo() && !app.can_redo());
                        assert_eq!(std::fs::read(fixture.task_path()).unwrap(), bytes);
                        let y = rows.y + index as u16;
                        let rendered = screen(&terminal);
                        assert!(
                            rendered
                                .lines()
                                .nth(usize::from(y))
                                .unwrap()
                                .contains("  add › New task")
                        );
                        assert_eq!(rendered.matches('›').count(), 1);
                        let buffer = terminal.backend().buffer();
                        assert_eq!(buffer[(rows.x + 2, y)].fg, MUTED);
                        assert_eq!(buffer[(rows.x + 6, y)].fg, app.accent);
                        assert_eq!(buffer[(rows.x + 8, y)].fg, TEXT);
                        assert!(terminal.backend().cursor_visible());
                        assert_eq!(
                            terminal.backend().cursor_position(),
                            (rows.x + 16, y).into()
                        );
                        app.key(
                            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                            &fixture.storage,
                        )
                        .unwrap();
                        assert_eq!(app.tasks[index].text, "New task");
                        assert!(!app.tasks[index].completed);
                        assert_eq!(app.list.selected(), Some(index));
                        assert_eq!(fixture.storage.load().unwrap(), app.tasks);
                        let added = app.tasks.clone();
                        app.key(
                            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE),
                            &fixture.storage,
                        )
                        .unwrap();
                        assert_eq!(app.tasks, original);
                        assert_eq!(app.list.selected(), selected);
                        app.key(
                            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
                            &fixture.storage,
                        )
                        .unwrap();
                        assert_eq!(app.tasks, added);
                        terminal
                            .draw(|frame| {
                                draw(frame, &mut app);
                            })
                            .unwrap();
                        assert!(app.row_tasks.iter().all(Option::is_some));
                        assert!(!screen(&terminal).contains("add ›"));
                        assert!(!terminal.backend().cursor_visible());
                    }
                }
            }
        }
    }

    #[test]
    fn inline_edit_replaces_wrapped_rows_and_cancel_or_confirm_restores_layout() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for ending in [KeyCode::Esc, KeyCode::Enter] {
            let fixture = crate::storage::tests::Fixture::new();
            let mut app = App::new(vec![
                Task {
                    carried: false,
                    text: "before".into(),
                    completed: false,
                },
                Task {
                    carried: false,
                    text: format!("{} end", "original e\u{301}界👩‍💻 ".repeat(30)),
                    completed: true,
                },
                Task {
                    carried: false,
                    text: "after".into(),
                    completed: true,
                },
            ]);
            app.keybinds = false;
            app.completed_at_bottom = true;
            app.list.select(Some(1));
            fixture.storage.save(&app.tasks).unwrap();
            let original = app.tasks.clone();
            let bytes = std::fs::read(fixture.task_path()).unwrap();
            let mut terminal = Terminal::new(TestBackend::new(26, 8)).unwrap();
            let mut rows = Rect::default();
            terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
            assert!(
                app.row_tasks
                    .iter()
                    .filter(|&&task| task == Some(1))
                    .count()
                    > usize::from(rows.height)
            );
            app.key(
                KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
                &fixture.storage,
            )
            .unwrap();
            if let Mode::Editing { input, .. } = &app.mode {
                assert_eq!(input.text(), original[1].text);
                assert_eq!(input.cursor(), input.text().len());
            } else {
                panic!("expected editing");
            }
            terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
            assert_eq!(app.row_tasks, [Some(0), Some(1), Some(2)]);
            assert!(screen(&terminal).contains("before"));
            assert!(screen(&terminal).contains("after"));
            assert!(screen(&terminal).contains("edit ›"));
            assert!(screen(&terminal).contains("end"));
            assert_eq!(screen(&terminal).matches('›').count(), 1);
            assert_eq!(terminal.backend().cursor_position().y, rows.y + 1);
            app.key(
                KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
                &fixture.storage,
            )
            .unwrap();
            app.paste("New\n");
            terminal
                .draw(|frame| {
                    draw(frame, &mut app);
                })
                .unwrap();
            assert!(screen(&terminal).contains("edit › New original"));
            assert_eq!(app.tasks, original);
            assert_eq!(std::fs::read(fixture.task_path()).unwrap(), bytes);
            assert!(!app.can_undo());
            app.key(KeyEvent::new(ending, KeyModifiers::NONE), &fixture.storage)
                .unwrap();
            terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
            assert!(!screen(&terminal).contains("edit ›"));
            assert!(!terminal.backend().cursor_visible());
            let end = (app.scroll + usize::from(rows.height)).min(app.row_tasks.len());
            assert!(app.row_tasks[app.scroll..end].contains(&Some(1)));
            if ending == KeyCode::Esc {
                assert_eq!(app.tasks, original);
                assert_eq!(std::fs::read(fixture.task_path()).unwrap(), bytes);
                assert!(!app.can_undo());
            } else {
                assert_eq!(app.tasks[1].text, format!("New {}", original[1].text));
                assert!(app.tasks[1].completed);
                let edited = app.tasks.clone();
                app.key(
                    KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE),
                    &fixture.storage,
                )
                .unwrap();
                assert_eq!(app.tasks, original);
                app.key(
                    KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
                    &fixture.storage,
                )
                .unwrap();
                assert_eq!(app.tasks, edited);
            }
            assert_eq!(app.list.selected(), Some(1));
            assert_eq!(fixture.storage.load().unwrap(), app.tasks);
        }
    }

    #[test]
    fn inline_inputs_follow_scrolling_and_resize_without_out_of_bounds_cursors() {
        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };
        let fixture = crate::storage::tests::Fixture::new();
        for key in ['a', 'o', 'O', 'e'] {
            let mut app = App::new(
                (0..20)
                    .map(|index| Task {
                        carried: false,
                        text: format!("task {index} {}end", "e\u{301}界👩‍💻 ".repeat(12)),
                        completed: index >= 10,
                    })
                    .collect(),
            );
            app.completed_at_bottom = true;
            app.list.select(Some(18));
            let original = app.tasks.clone();
            app.key(
                KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE),
                &fixture.storage,
            )
            .unwrap();
            if key != 'e' {
                app.paste("new e\u{301}界👩‍💻 text");
            }
            let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
            for wrap in [false, true] {
                app.wrap = wrap;
                for width in [0, 1, 2, 3, 8, 16, 40] {
                    for height in [0, 1, 2, 3, 6, 12] {
                        terminal.backend_mut().resize(width, height);
                        for movement in [KeyCode::Home, KeyCode::Right, KeyCode::End, KeyCode::Left]
                        {
                            app.key(
                                KeyEvent::new(movement, KeyModifiers::NONE),
                                &fixture.storage,
                            )
                            .unwrap();
                            app.scroll = usize::MAX;
                            app.reveal_selection = false;
                            let mut rows = Rect::default();
                            terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
                            assert!(
                                app.row_tasks
                                    .iter()
                                    .flatten()
                                    .all(|&index| index < app.tasks.len())
                            );
                            if rows.is_empty() {
                                assert!(!terminal.backend().cursor_visible());
                            } else {
                                assert!(terminal.backend().cursor_visible());
                                let cursor = terminal.backend().cursor_position();
                                assert!(rows.contains(cursor));
                                let visual_row = app.scroll + usize::from(cursor.y - rows.y);
                                assert_eq!(
                                    app.row_tasks[visual_row],
                                    if key == 'e' { Some(18) } else { None }
                                );
                                for kind in [
                                    MouseEventKind::Down(MouseButton::Left),
                                    MouseEventKind::ScrollDown,
                                ] {
                                    app.mouse(
                                        MouseEvent {
                                            kind,
                                            column: cursor.x,
                                            row: cursor.y,
                                            modifiers: KeyModifiers::NONE,
                                        },
                                        rows,
                                        &fixture.storage,
                                    )
                                    .unwrap();
                                }
                            }
                        }
                    }
                }
            }
            app.key(
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                &fixture.storage,
            )
            .unwrap();
            terminal.backend_mut().resize(40, 12);
            let mut rows = Rect::default();
            terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
            let end = (app.scroll + usize::from(rows.height)).min(app.row_tasks.len());
            assert!(app.row_tasks[app.scroll..end].contains(&Some(18)));
            assert_eq!(app.tasks, original);
            assert_eq!(app.list.selected(), Some(18));
            assert!(!app.can_undo() && !app.can_redo());
            assert!(!fixture.task_path().exists());
        }
    }

    #[test]
    fn search_keeps_its_bottom_prompt_and_read_only_results_cursor() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = App::new(vec![Task {
            carried: false,
            text: "needle".into(),
            completed: true,
        }]);
        app.keybinds = false;
        app.key(
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
            &fixture.storage,
        )
        .unwrap();
        app.paste("needle");
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        for results in [false, true] {
            let mut rows = Rect::default();
            terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
            assert_eq!(rows.height, 9);
            assert!(
                screen(&terminal)
                    .lines()
                    .last()
                    .unwrap()
                    .starts_with("search › needle")
            );
            assert_eq!(app.row_tasks, [Some(0)]);
            assert!(screen(&terminal).contains("■"));
            assert_eq!(terminal.backend().cursor_visible(), !results);
            if !results {
                assert_eq!(terminal.backend().cursor_position().y, 11);
            }
            app.key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &fixture.storage,
            )
            .unwrap();
        }
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert!(screen(&terminal).contains("[x] needle"));
        assert!(!screen(&terminal).contains("search ›"));
        assert!(app.tasks[0].completed);
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn search_markers_highlights_and_exit_preserve_text_and_alignment() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = App::new(
            [
                ("work: İfoo İFOO e\u{301} 👩‍💻", false),
                ("İfoo completed", true),
                ("unrelated", false),
            ]
            .map(|(text, completed)| Task {
                carried: false,
                text: text.into(),
                completed,
            })
            .into(),
        );
        let original = app.tasks.clone();
        app.keybinds = false;
        app.list.select(Some(2));
        app.accent = Color::Rgb(20, 40, 60);
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let key = |app: &mut App, code| {
            app.key(KeyEvent::new(code, KeyModifiers::NONE), &fixture.storage)
                .unwrap()
        };
        key(&mut app, KeyCode::Char('/'));
        app.paste("i\u{307}foo");
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(1, 1)].symbol(), "›");
        assert_eq!(buffer[(1, 3)].symbol(), " ");
        for y in [1, 2] {
            assert_eq!(buffer[(4, y)].symbol(), "■");
            assert_eq!(buffer[(4, y)].fg, app.accent);
        }
        assert_eq!(buffer[(4, 3)].symbol(), "□");
        assert_eq!(buffer[(4, 3)].fg, MUTED);
        assert_eq!(buffer[(7, 1)].symbol(), "w");
        assert!(buffer[(7, 1)].modifier.contains(Modifier::BOLD));
        for (y, expected) in [(1, "İfooİFOO"), (2, "İfoo")] {
            let mut highlighted = String::new();
            for x in 7..39 {
                let cell = &buffer[(x, y)];
                if cell.bg == app.accent {
                    highlighted.push_str(cell.symbol());
                    assert_eq!(cell.fg, Color::Rgb(255, 255, 255));
                    assert_eq!(cell.modifier.contains(Modifier::CROSSED_OUT), y == 2);
                }
            }
            assert_eq!(highlighted, expected);
        }
        key(&mut app, KeyCode::Enter);
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert!(screen(&terminal).contains("search › i\u{307}foo"));
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert_eq!(app.list.selected(), Some(1));
        assert_eq!(app.tasks, original);
        assert_eq!(terminal.backend().buffer()[(4, 1)].symbol(), " ");
        assert_eq!(terminal.backend().buffer()[(4, 2)].symbol(), "x");
        for y in 1..4 {
            for x in 7..39 {
                assert_eq!(terminal.backend().buffer()[(x, y)].bg, Color::Reset);
            }
        }
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn search_spanning_wrapped_rows_preserves_mapping_and_reveals_results_after_resize() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = App::new(vec![
            Task {
                carried: false,
                text: "tag: abCDef abcdef".into(),
                completed: false,
            },
            Task {
                carried: false,
                text: "nonmatching".into(),
                completed: false,
            },
            Task {
                carried: false,
                text: format!("{}tag: abcdef", "long ".repeat(50)),
                completed: true,
            },
        ]);
        app.keybinds = false;
        let original = app.tasks.clone();
        let mut terminal = Terminal::new(TestBackend::new(14, 9)).unwrap();
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        let mapping = app.row_tasks.clone();
        app.key(
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
            &fixture.storage,
        )
        .unwrap();
        app.paste("tag: abcd");
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert_eq!(app.row_tasks, mapping);
        for (y, length) in [(1, 4), (2, 4)] {
            for x in 7..7 + length {
                assert_eq!(terminal.backend().buffer()[(x, y)].bg, app.accent);
            }
            assert_eq!(
                terminal.backend().buffer()[(7 + length, y)].bg,
                Color::Reset
            );
        }
        app.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &fixture.storage,
        )
        .unwrap();
        for (width, height, wrap) in [
            (18, 8, true),
            (40, 12, false),
            (9, 3, true),
            (1, 1, true),
            (0, 0, false),
            (14, 9, true),
        ] {
            app.wrap = wrap;
            terminal.backend_mut().resize(width, height);
            terminal
                .draw(|frame| {
                    draw(frame, &mut app);
                })
                .unwrap();
            app.key(
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                &fixture.storage,
            )
            .unwrap();
            terminal
                .draw(|frame| {
                    draw(frame, &mut app);
                })
                .unwrap();
            let selected = app.list.selected().unwrap();
            if app.viewport_height > 0 {
                let end = (app.scroll + usize::from(app.viewport_height)).min(app.row_tasks.len());
                assert!(app.row_tasks[app.scroll..end].contains(&Some(selected)));
            }
        }
        app.key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &fixture.storage,
        )
        .unwrap();
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert_eq!(app.list.selected(), Some(0));
        assert_eq!(app.tasks, original);
        assert!(!app.can_undo());
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn search_feedback_legends_and_contrast_remain_compact() {
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = App::new(vec![Task {
            carried: false,
            text: "ordinary".into(),
            completed: true,
        }]);
        app.mode = Mode::Searching {
            input: "missing".into(),
            original: Some(0),
        };
        app.list.select(None);
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert!(screen(&terminal).contains("No matches"));
        assert_eq!(terminal.backend().buffer()[(1, 1)].symbol(), " ");
        assert_eq!(terminal.backend().buffer()[(4, 1)].symbol(), "□");
        for results in [false, true] {
            app.mode = if results {
                Mode::SearchResults {
                    input: "ordinary".into(),
                    original: Some(0),
                }
            } else {
                Mode::Searching {
                    input: "ordinary".into(),
                    original: Some(0),
                }
            };
            let text = shortcut_lines(&app, 80)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ");
            assert!(text.contains("Esc") && text.contains("Cancel"));
            assert!(text.contains(if results { "Select" } else { "Results" }));
            assert!(!text.contains("Page"));
            for width in [0, 1, 12, 18, 30, 40, 60, 80] {
                let lines = shortcut_lines(&app, width);
                assert!(lines.len() <= 3);
                assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
            }
            app.keybinds = false;
            assert!(shortcut_lines(&app, 80).is_empty());
            app.keybinds = true;
        }
        assert_eq!(
            highlight_foreground(Color::Rgb(0, 0, 0)),
            Color::Rgb(255, 255, 255)
        );
        assert_eq!(
            highlight_foreground(Color::Rgb(255, 255, 255)),
            Color::Rgb(0, 0, 0)
        );
        assert_eq!(highlight_foreground(DEFAULT_ACCENT), Color::Rgb(0, 0, 0));
        // Independent contrast check across a small RGB cube, including midtones.
        for r in [0, 64, 128, 192, 255] {
            for g in [0, 64, 128, 192, 255] {
                for b in [0, 64, 128, 192, 255] {
                    let linear = |v| {
                        let s = f64::from(v) / 255.0;
                        if s <= 0.04045 {
                            s / 12.92
                        } else {
                            ((s + 0.055) / 1.055).powf(2.4)
                        }
                    };
                    let l = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
                    let ratio = if highlight_foreground(Color::Rgb(r, g, b)) == Color::Rgb(0, 0, 0)
                    {
                        (l + 0.05) / 0.05
                    } else {
                        1.05 / (l + 0.05)
                    };
                    assert!(ratio >= 4.5);
                }
            }
        }
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn redo_hint_is_contextual_and_restored_wrapped_selection_is_visible() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = App::new(vec![
            Task {
                carried: false,
                text: "wrapped task ".repeat(40).trim_end().into(),
                completed: false,
            },
            Task {
                carried: false,
                text: "other active task".into(),
                completed: false,
            },
            Task {
                carried: false,
                text: "done".into(),
                completed: true,
            },
        ]);
        app.completed_at_bottom = true;
        let key = |app: &mut App, code, modifiers| {
            app.key(KeyEvent::new(code, modifiers), &fixture.storage)
                .unwrap()
        };
        assert!(
            !shortcut_lines(&app, 80)
                .iter()
                .any(|line| line.to_string().contains("Redo"))
        );
        key(&mut app, KeyCode::Char(' '), KeyModifiers::NONE);
        let modified = app.tasks.clone();
        key(&mut app, KeyCode::Char('u'), KeyModifiers::NONE);
        assert!(
            shortcut_lines(&app, 80)
                .iter()
                .any(|line| line.to_string().contains("Ctrl+R Redo"))
        );
        for width in [0, 1, 18, 40, 60, 80] {
            let lines = shortcut_lines(&app, width);
            assert!(lines.len() <= 3);
            assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
        }
        let mut terminal = Terminal::new(TestBackend::new(18, 7)).unwrap();
        app.scroll = usize::MAX;
        key(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);
        assert_eq!(app.tasks, modified);
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert_eq!(app.list.selected(), Some(1));
        assert_eq!(app.row_tasks[app.scroll], Some(1));
        assert!(!app.can_redo());
        key(&mut app, KeyCode::Char('D'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('u'), KeyModifiers::NONE);
        assert!(app.can_redo());
        app.keybinds = false;
        assert!(shortcut_lines(&app, 80).is_empty());
    }

    #[test]
    fn legends_follow_mode_availability_and_terminal_width() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = App::new(vec![Task {
            carried: false,
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
                "Clear",
                "Quit",
                "Wrap",
            ] {
                assert!(normal.contains(key), "{normal}");
            }
        }
        assert_eq!(
            text(&app, 40),
            " j/k Navigate    Space Toggle  w Wrap\n a/e Add/edit    d     Delete  u Undo\n x   Clear done  D     Clear   q Quit"
        );
        for mode in [
            Mode::Adding(TextInput::default(), AddPlacement::End),
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
        app.mode = Mode::Adding(TextInput::default(), AddPlacement::End);
        assert!(shortcut_lines(&app, 80).is_empty());
    }

    #[test]
    fn changing_footer_height_preserves_selection_and_mouse_mapping() {
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = App::new(
            (0..12)
                .map(|index| Task {
                    carried: false,
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
            Mode::Adding(TextInput::default(), AddPlacement::End),
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
            assert!(app.row_tasks[app.scroll..end].contains(&Some(11)));
            if app.mode == Mode::Normal {
                let index = app.row_tasks[app.scroll].unwrap();
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
        app.mode = Mode::Adding(TextInput::default(), AddPlacement::End);
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        assert_eq!(rows.height, 10);
    }

    #[test]
    fn tiny_terminals_and_resizing_are_safe_in_every_mode() {
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        let mut app = App::new(vec![Task {
            carried: false,
            text: "日本語 🪷".into(),
            completed: false,
        }]);
        for width in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 40, 60, 80] {
            for height in [0, 1, 2, 3, 6, 12] {
                terminal.backend_mut().resize(width, height);
                for mode in [
                    Mode::Normal,
                    Mode::Adding("Long input 日本語 🪷".into(), AddPlacement::End),
                    Mode::Editing {
                        index: 0,
                        input: "Existing text 日本語 🪷".into(),
                    },
                    Mode::Searching {
                        input: "Long search 日本語 🪷".into(),
                        original: Some(0),
                    },
                    Mode::SearchResults {
                        input: "日本語".into(),
                        original: Some(0),
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
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
                carried: false,
                text: text.into(),
                completed: false,
            })
            .into(),
        );
        app.keybinds = false;
        let mut rows = Rect::default();
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        assert_eq!(rows.height, 3);
        assert_eq!(app.row_tasks, [0, 1, 1, 2, 3, 3, 3, 4].map(Some));
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
            assert!(app.row_tasks[app.scroll..end].contains(&Some(selected)));
        }
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn page_navigation_uses_current_viewport_wrapping_and_resize() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        let mut app = App::new(
            (0..40)
                .map(|index| Task {
                    carried: false,
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
                assert!(
                    app.row_tasks[app.scroll..end].contains(&Some(app.list.selected().unwrap()))
                );
            }
        }
    }

    #[test]
    fn long_lists_scroll_to_selection_and_wrap_back_to_start() {
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let mut app = App::new(
            (0..1000)
                .map(|index| Task {
                    carried: false,
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
    fn moving_a_completed_task_keeps_selection_visible_and_mouse_undo_correct() {
        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = App::new(vec![
            Task {
                carried: false,
                text: "long wrapped task with many words".into(),
                completed: false,
            },
            Task {
                carried: false,
                text: "next".into(),
                completed: false,
            },
            Task {
                carried: false,
                text: "already done".into(),
                completed: true,
            },
        ]);
        app.completed_at_bottom = true;
        app.keybinds = false;
        let original = app.tasks.clone();
        let mut terminal = Terminal::new(TestBackend::new(18, 5)).unwrap();
        let mut rows = Rect::default();
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: rows.x + 6,
                row: rows.y,
                modifiers: KeyModifiers::NONE,
            },
            rows,
            &fixture.storage,
        )
        .unwrap();
        assert_eq!(app.list.selected(), Some(1));
        assert_eq!(app.tasks[1].text, original[0].text);
        assert!(app.tasks[1].completed);
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        assert_eq!(app.row_tasks[app.scroll], Some(1));
        assert!(screen(&terminal).contains("› [x] long"));
        app.key(
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE),
            &fixture.storage,
        )
        .unwrap();
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert_eq!(app.list.selected(), Some(0));
        assert_eq!(app.tasks, original);
        assert_eq!(app.row_tasks[app.scroll], Some(0));
        assert_eq!(fixture.storage.load().unwrap(), original);
        assert!(!app.can_undo());
    }

    #[test]
    fn only_leading_tags_are_bold_across_wrapping_and_completion_styles() {
        for (text, tag) in [
            ("foo: first bar:", "foo:"),
            ("foo:bar", "foo:"),
            ("no tag: task", ""),
            ("task foo:", ""),
            (":empty", ""),
            ("e\u{301}👩‍💻: body", "e\u{301}👩‍💻:"),
            ("abcdefghijkl: body", "abcdefghijkl:"),
            ("a:\u{301} body", "a:\u{301}"),
        ] {
            for completed in [false, true] {
                for (width, wrap) in [(12, true), (40, true), (40, false)] {
                    let mut app = App::new(vec![Task {
                        carried: false,
                        text: text.into(),
                        completed,
                    }]);
                    let original = app.tasks.clone();
                    app.keybinds = false;
                    app.wrap = wrap;
                    let mut terminal = Terminal::new(TestBackend::new(width, 20)).unwrap();
                    terminal
                        .draw(|frame| {
                            draw(frame, &mut app);
                        })
                        .unwrap();
                    let buffer = terminal.backend().buffer();
                    let mut bold = String::new();
                    for y in 1..19 {
                        for x in 7..width - 1 {
                            let cell = &buffer[(x, y)];
                            if cell.symbol() == " " {
                                continue;
                            }
                            if cell.modifier.contains(Modifier::BOLD) {
                                bold.push_str(cell.symbol());
                            }
                            assert_eq!(cell.fg, if completed { MUTED } else { TEXT });
                            assert_eq!(cell.modifier.contains(Modifier::CROSSED_OUT), completed);
                            assert!(!cell.modifier.contains(Modifier::DIM));
                        }
                    }
                    assert_eq!(bold, tag, "{text:?}, width={width}, wrap={wrap}");
                    assert!(!buffer[(3, 1)].modifier.contains(Modifier::BOLD));
                    assert_eq!(buffer[(1, 1)].fg, app.accent);
                    assert_eq!(app.tasks, original);
                    assert!(!app.can_undo());
                }
            }
        }
    }

    #[test]
    fn completed_text_stays_muted_with_accented_marker_and_checkbox() {
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let mut app = App::new(vec![Task {
            carried: false,
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
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = Terminal::new(TestBackend::new(12, 8)).unwrap();
        let mut app = App::new(vec![
            Task {
                carried: false,
                text: "日本語".repeat(100),
                completed: false,
            },
            Task {
                carried: false,
                text: "next".into(),
                completed: false,
            },
        ]);
        let toggle = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE);
        app.key(toggle, &fixture.storage).unwrap();
        assert!(!app.wrap);
        app.mode = Mode::Adding("a long input ending in 界z".into(), AddPlacement::End);
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        let rendered = screen(&terminal);
        assert!(rendered.contains("[ ] next"));
        assert!(rendered.contains("add › z"));
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
    fn editing_navigation_keeps_cursor_visible_through_long_unicode_input() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let text = format!("start {}end", "e\u{301}界👩‍💻 ".repeat(8));
        for edit in [false, true] {
            for width in [1, 2, 8, 16, 40] {
                let mut app = App::new(vec![Task {
                    carried: false,
                    text: text.clone(),
                    completed: false,
                }]);
                app.mode = if edit {
                    Mode::Editing {
                        index: 0,
                        input: text.clone().into(),
                    }
                } else {
                    Mode::Adding(text.clone().into(), AddPlacement::End)
                };
                let mut terminal = Terminal::new(TestBackend::new(width, 5)).unwrap();
                for code in std::iter::repeat_n(KeyCode::Left, text.graphemes(true).count() + 2)
                    .chain(std::iter::repeat_n(
                        KeyCode::Right,
                        text.graphemes(true).count() + 2,
                    ))
                    .chain([KeyCode::Home, KeyCode::End])
                {
                    app.key(
                        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Repeat),
                        &fixture.storage,
                    )
                    .unwrap();
                    terminal
                        .draw(|frame| {
                            draw(frame, &mut app);
                        })
                        .unwrap();
                    let cursor = terminal.get_cursor_position().unwrap();
                    assert!(cursor.x < width);
                    assert!(cursor.y < 5);
                    if width >= 16 && code == KeyCode::Home {
                        assert!(screen(&terminal).contains("start"));
                    }
                    if width >= 16 && code == KeyCode::End {
                        assert!(screen(&terminal).contains("end"));
                    }
                }
                assert_eq!(app.tasks[0].text, text);
                assert!(!app.can_undo());
            }
        }
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
        assert_eq!(
            wrapped_strings("alpha beta gamma", 10),
            ["alpha beta", "gamma"]
        );
        assert_eq!(wrapped_strings("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(wrapped_strings("日本語", 4), ["日本", "語"]);
        assert_eq!(
            wrapped_strings("e\u{301}e\u{301}", 1),
            ["e\u{301}", "e\u{301}"]
        );
        assert_eq!(wrapped_strings("👩‍💻👩‍💻", 2), ["👩‍💻", "👩‍💻"]);
        assert_eq!(wrapped_strings("界a", 1), ["�", "a"]);
        assert_eq!(wrapped_strings("", 4), [""]);
    }

    #[test]
    fn wrapping_trims_only_whole_whitespace_graphemes() {
        for (text, width, expected) in [
            ("abcd \u{301}X", 4, vec!["abcd", " \u{301}X"]),
            ("abcd  \u{301}X", 4, vec!["abcd", " \u{301}X"]),
            ("ab\u{600} cde", 4, vec!["ab\u{600} ", "cde"]),
            ("ab\u{600}  cde", 4, vec!["ab\u{600} ", "cde"]),
            ("abcd \u{a0}\u{2003}X", 4, vec!["abcd", "X"]),
        ] {
            assert_eq!(wrapped_strings(text, width), expected, "{text:?}");
        }
    }

    #[test]
    fn wrapped_whitespace_cluster_stays_visible_and_maps_to_its_task() {
        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = App::new(
            ["abcd \u{301}X", "last"]
                .map(|text| Task {
                    carried: false,
                    text: text.into(),
                    completed: false,
                })
                .into(),
        );
        app.keybinds = false;
        let mut terminal = Terminal::new(TestBackend::new(12, 5)).unwrap();
        let mut rows = Rect::default();
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        assert_eq!(app.row_tasks, [0, 0, 1].map(Some));
        assert_eq!(terminal.backend().buffer()[(7, 2)].symbol(), " \u{301}");
        assert_eq!(terminal.backend().buffer()[(8, 2)].symbol(), "X");
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 7,
                row: 2,
                modifiers: KeyModifiers::NONE,
            },
            rows,
            &fixture.storage,
        )
        .unwrap();
        assert!(app.tasks[0].completed);
        assert!(!app.tasks[1].completed);
        assert_eq!(fixture.storage.load().unwrap(), app.tasks);
        assert_eq!(app.tasks[0].text, "abcd \u{301}X");
        for (code, selected) in [(KeyCode::PageDown, 1), (KeyCode::PageUp, 0)] {
            app.key(KeyEvent::new(code, KeyModifiers::NONE), &fixture.storage)
                .unwrap();
            terminal
                .draw(|frame| {
                    draw(frame, &mut app);
                })
                .unwrap();
            assert_eq!(app.list.selected(), Some(selected));
        }
        terminal.backend_mut().resize(13, 5);
        terminal
            .draw(|frame| {
                draw(frame, &mut app);
            })
            .unwrap();
        assert_eq!(app.row_tasks, [0, 0, 1].map(Some));
        assert_eq!(terminal.backend().buffer()[(11, 1)].symbol(), " \u{301}");
    }

    #[test]
    fn continuation_clicks_toggle_the_correct_task_and_leave_other_rows_alone() {
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = Terminal::new(TestBackend::new(18, 12)).unwrap();
        let mut app = App::new(vec![
            Task {
                carried: false,
                text: "alpha beta gamma delta".into(),
                completed: false,
            },
            Task {
                carried: false,
                text: "next".into(),
                completed: false,
            },
        ]);
        let mut rows = Rect::default();
        terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();
        assert_eq!(app.row_tasks, [0, 0, 0, 1].map(Some));
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
        use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = Terminal::new(TestBackend::new(18, 8)).unwrap();
        let mut app = App::new(vec![
            Task {
                carried: false,
                text: format!("{}THE END", "word ".repeat(30)),
                completed: false,
            },
            Task {
                carried: false,
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
