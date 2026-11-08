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

use crate::app::{App, Mode, SectionView, Task, TaskStatus, task_prefix};
use crate::config::{BorderStyle, MarkerStyle};
use crate::input::TextInput;
use crate::styles::Styles;

pub fn draw(frame: &mut Frame, app: &mut App) -> Rect {
    let area = frame.area();
    frame.render_widget(Block::default().style(app.styles.normal), area);
    let searching = matches!(app.mode, Mode::Searching { .. });
    // Only search uses a separate input row; add/edit live inside the checklist.
    let input_height = u16::from(searching && area.height > 0);
    let sections =
        Layout::vertical([Constraint::Min(0), Constraint::Length(input_height)]).split(area);

    let tabs = app.section_tabs();
    app.tab_targets.clear();
    let list_area = sections[0];

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type(app.border_style))
        .border_style(app.styles.border);

    let mut showing_status = false;

    if let Some(notice) = &app.status_message
        && let Some(status_style) = notice.style(
            std::time::Instant::now(),
            app.styles.accent,
            app.styles.normal,
        )
    {
        // Status messages may use the whole border between the corners.
        let available = usize::from(list_area.width).saturating_sub(2);
        let short = notice.compact_message();

        let message = if notice.message().width() <= available {
            notice.message()
        } else {
            short
        };

        if list_area.height >= 2 && message.width() <= available {
            block = block.title_bottom(Line::styled(message, status_style).left_aligned());
            showing_status = true;
        }
    }

    // The hint points to the keyboard reference from the bottom-right corner. It only shows
    // on the plain checklist, and gives the border to brief status messages.
    if app.show_help_hint
        && app.mode == Mode::Normal
        && !showing_status
        && list_area.height >= 2
        && HELP_HINT.width() + 2 <= usize::from(list_area.width)
    {
        block = block.title_bottom(Line::styled(HELP_HINT, app.styles.muted).right_aligned());
    }

    let mut rows = block.inner(list_area);
    frame.render_widget(block, list_area);

    if !tabs.is_empty() && list_area.height >= 2 {
        // Tabs sit on the top border between the corners.
        let strip_width = list_area.width.saturating_sub(2);
        let strip = Rect::new(list_area.x + 1, list_area.y, strip_width, 1);
        app.tab_targets = draw_section_tabs(frame, app, &tabs, strip);

        // Leave a blank row under the tabs so they don't merge with the first task,
        // unless that would leave no room for tasks.
        if rows.height >= 2 {
            let blank_row = rows.y;
            rows.y += 1;
            rows.height -= 1;

            // While `s` waits for a digit, number the tabs in the blank row, each under the
            // first character of its name (the `*` or `-` of [*] and [-]). Only the first
            // ten tabs have a digit, and tabs scrolled off the strip get none.
            if showing_section_jump(app) {
                for (target, view) in &app.tab_targets {
                    let Some(position) = tabs.iter().position(|(tab, _)| tab == view) else {
                        continue;
                    };

                    if position >= JUMPABLE_TABS {
                        continue;
                    }

                    let bracketed = matches!(view, SectionView::All | SectionView::Unsectioned);
                    let x = target.x + 1 + u16::from(bracketed);
                    let digit = (position + 1) % JUMPABLE_TABS;

                    frame.buffer_mut().set_string(
                        x,
                        blank_row,
                        digit.to_string(),
                        app.styles.accent,
                    );
                }
            }
        }
    }

    draw_tasks(frame, app, rows);

    if let Mode::Searching { input, .. } = &app.mode {
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
            &app.styles,
            true,
            false,
            None,
        );

        // The hint sits at the right edge only while the prompt, query, and cursor cell
        // leave room for it, so it never covers typing.
        let prompt_width = label.width() + " › ".width();
        let typed_width = prompt_width + input.text().width() + 1;
        let needed_width = typed_width + SEARCH_HINT_GAP + SEARCH_HINT.width();

        if needed_width <= usize::from(sections[1].width) {
            frame.render_widget(
                Paragraph::new(Line::styled(SEARCH_HINT, app.styles.muted).right_aligned()),
                sections[1],
            );
        }
    }

    if app.mode == Mode::ConfirmClear {
        let section = app.scope_label().map(|label| format!("{label} "));
        let question = format!("Clear unmarked {}tasks?", section.unwrap_or_default());
        draw_confirmation(frame, area, &app.styles, app.border_style, &question);
    }

    if matches!(app.mode, Mode::Help { .. }) {
        draw_help(frame, area, app);
    }

    rows
}

const HELP_HINT: &str = "? for help";

const TAB_MARKER: usize = 1;

const SEARCH_HINT: &str = "Tab to cycle";

// Blank cells kept between the search query's cursor and the hint.
const SEARCH_HINT_GAP: usize = 2;

// `s1` … `s9` and `s0` reach the first ten tabs.
const JUMPABLE_TABS: usize = 10;

// After `s` / Alt+S, Normal mode waits for a digit.
fn showing_section_jump(app: &App) -> bool {
    app.mode == Mode::Normal && app.section_jump_pending()
}

// The prompt drawn before a section name being renamed, like the inline edit row's.
const RENAME_LABEL: &str = "rename";

// The section picker's input grows with its query up to a third of the tab strip (but at
// least this many cells), leaving the rest for matching tabs.
const PICKER_INPUT_SHARE: usize = 3;
const PICKER_INPUT_MIN_WIDTH: usize = 10;

// The narrowest shortened tab label worth showing: one cell of the name and `…`.
const MIN_TAB_LABEL_WIDTH: usize = 2;

// Draw section tabs on the box's top border (`strip`). Every tab keeps the same width
// whichever one is active, so labels never move; the active tab only changes color.
// While renaming, the active tab becomes the name input and grows with it. The section
// picker replaces [*] with its query and shows only the matching tabs, best first.
// Returns each drawn tab's clickable area.
fn draw_section_tabs(
    frame: &mut Frame,
    app: &mut App,
    tabs: &[(SectionView, String)],
    mut strip: Rect,
) -> Vec<(Rect, SectionView)> {
    let mut tabs = tabs.to_vec();
    let picking = matches!(app.mode, Mode::FuzzySection { .. });

    if let Mode::FuzzySection { input, matches, .. } = &app.mode {
        let width = usize::from(strip.width);
        let prompt_width = 2 + Line::raw("› ").width() + 1;
        let wanted = prompt_width + input.text().width();

        // Leave room after the input for at least a shortened match (a leading cell, the
        // padded label, and a border cell), but never shrink the input below its empty
        // prompt and cursor.
        let shortest_match = TAB_MARKER + MIN_TAB_LABEL_WIDTH + 2 + 1;
        let limit = (width / PICKER_INPUT_SHARE)
            .max(PICKER_INPUT_MIN_WIDTH)
            .min(width.saturating_sub(TAB_MARKER + shortest_match))
            .max(prompt_width);
        let input_width = wanted.min(limit).min(width.saturating_sub(TAB_MARKER + 1));
        let x = strip.x + TAB_MARKER as u16;

        if input_width > 0 {
            // Blank the input first so the border doesn't show under the cursor cell.
            frame
                .buffer_mut()
                .set_string(x, strip.y, " ".repeat(input_width), app.styles.normal);

            draw_input_line(
                frame,
                Rect::new(x + 1, strip.y, (input_width as u16).saturating_sub(2), 1),
                input,
                "",
                &app.styles,
                true,
                false,
                None,
            );
        }

        // The border cell after the input leads the matches and holds `‹` when they scroll.
        let used = (TAB_MARKER + input_width).min(width) as u16;
        strip = Rect::new(strip.x + used, strip.y, strip.width - used, 1);

        tabs = matches
            .iter()
            .filter_map(|view| tabs.iter().find(|(tab, _)| tab == view).cloned())
            .collect();
    }

    // Outside the picker the view is always one of the tabs. In the picker it may not be a
    // match (it opened on [*], or nothing matches), and then no tab is highlighted.
    let found = tabs.iter().position(|(view, _)| *view == app.view);
    let active = if picking { found } else { found.or(Some(0)) };

    let mut widths: Vec<usize> = tabs.iter().map(|(_, label)| label.width() + 2).collect();
    let width = usize::from(strip.width);

    let rename_input = match &app.mode {
        Mode::RenamingSection { input } => Some(input),
        _ => None,
    };

    // Room for the prompt, the name, and one cursor cell, but never more than the strip
    // holds beside its leading cell and trailing border cell.
    if let Some(input) = rename_input
        && let Some(active) = active
    {
        let prompt_width = Line::raw(format!("{RENAME_LABEL} › ")).width();
        let input_width = 2 + prompt_width + input.text().width() + 1;

        widths[active] = input_width.min(width.saturating_sub(TAB_MARKER + 1));
    }

    // Tabs are drawn whole, so an active tab wider than the strip would vanish. Shorten its
    // label at a grapheme boundary to fit beside the leading cell and the trailing border
    // cell; when not even one glyph and `…` fit, it stays hidden behind `›`.
    let label_room = width.saturating_sub(TAB_MARKER + 1 + 2);

    if rename_input.is_none()
        && let Some(active) = active
        && tabs[active].1.width() > label_room
    {
        let mut shortened = String::new();
        let mut used = 0;

        for grapheme in tabs[active].1.graphemes(true) {
            if used + grapheme.width() + 1 > label_room {
                break;
            }

            used += grapheme.width();
            shortened.push_str(grapheme);
        }

        if label_room >= MIN_TAB_LABEL_WIDTH && !shortened.is_empty() {
            shortened.push('…');
            widths[active] = shortened.width() + 2;
            tabs[active].1 = shortened;
        }
    }

    // Scroll only when the active tab would otherwise leave the strip. The picker's matches
    // reorder on every edit, so they scroll from the first one and leave the saved scroll
    // for the ordinary tabs.
    let target = active.unwrap_or(0);
    let saved_scroll = if picking { 0 } else { app.tab_scroll };
    let mut start = saved_scroll.min(target);

    while start < target && fitting_tabs(&widths, start, width) <= target {
        start += 1;
    }

    if !picking {
        app.tab_scroll = start;
    }

    let end = fitting_tabs(&widths, start, width);

    let y = strip.y;
    let mut targets = Vec::new();

    if start > 0 {
        frame
            .buffer_mut()
            .set_string(strip.x, y, "‹", app.styles.muted);
    }

    let mut x = strip.x + TAB_MARKER as u16;

    for (index, (view, label)) in tabs.iter().enumerate().take(end).skip(start) {
        let tab_width = widths[index] as u16;

        let style = if Some(index) == active {
            app.styles.accent
        } else {
            app.styles.muted
        };

        if Some(index) == active
            && let Some(input) = rename_input
        {
            // A name that can't be used is drawn struck through; the notice says why.
            let mut styles = app.styles;

            if app.rename_problem().is_some() {
                styles.normal = styles.muted.add_modifier(Modifier::CROSSED_OUT);
            }

            // Blank the whole tab first so the border doesn't show under the cursor cell.
            frame.buffer_mut().set_string(
                x,
                y,
                " ".repeat(usize::from(tab_width)),
                app.styles.normal,
            );

            draw_input_line(
                frame,
                Rect::new(x + 1, y, tab_width.saturating_sub(2), 1),
                input,
                RENAME_LABEL,
                &styles,
                true,
                false,
                None,
            );
        } else {
            frame
                .buffer_mut()
                .set_string(x, y, format!(" {label} "), style);
        }

        targets.push((Rect::new(x, y, tab_width, 1), view.clone()));

        // One border cell separates tabs.
        x += tab_width + 1;
    }

    if end < tabs.len() {
        frame
            .buffer_mut()
            .set_string(x - 1, y, "›", app.styles.muted);
    }

    targets
}

// The end (exclusive) of the tabs that fit from `start`: one leading cell, then each tab
// followed by one border cell, which can hold the `›` marker.
fn fitting_tabs(widths: &[usize], start: usize, width: usize) -> usize {
    let mut used = TAB_MARKER;
    let mut end = start;

    while end < widths.len() {
        let tab_and_border = widths[end] + 1;

        if used + tab_and_border > width {
            break;
        }

        used += tab_and_border;
        end += 1;
    }

    end
}

// Prepared once per draw; the row mapping is moved back to App for navigation.
struct TaskLayout {
    lines: Vec<Line<'static>>,
    row_tasks: Vec<Option<usize>>,
    input_row: Option<usize>,
    selected_task: Option<usize>,
}

fn draw_tasks(frame: &mut Frame, app: &mut App, area: Rect) {
    let mapping = std::mem::take(&mut app.row_tasks);
    let layout = prepare_task_layout(app, area.width, mapping);
    let visible = update_task_viewport(app, &layout, area.height);

    let TaskLayout {
        lines,
        row_tasks,
        input_row,
        ..
    } = layout;

    app.row_tasks = row_tasks;
    let total_rows = lines.len();

    if lines.is_empty() {
        frame.render_widget(
            Paragraph::new(" Press a to add a task").style(app.styles.muted),
            area,
        );
        return;
    }

    frame.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .skip(visible.start)
                .take(visible.len())
                .collect::<Vec<_>>(),
        ),
        area,
    );
    draw_scrollbar(frame, area, total_rows, visible.start, app.styles.muted);
    draw_inline_input(frame, app, area, input_row, visible);
}

// When the rows don't fit, draw a thumb on the right border showing how much is visible and
// where. Drawing on the border keeps the list's full width.
fn draw_scrollbar(frame: &mut Frame, area: Rect, total: usize, scroll: usize, style: Style) {
    let height = usize::from(area.height);
    let border = area.right();

    if height == 0 || total <= height || border >= frame.area().right() {
        return;
    }

    let thumb = ((height * height + total / 2) / total).clamp(1, height);
    let hidden = total - height;
    let top = ((scroll * (height - thumb) + hidden / 2) / hidden).min(height - thumb);

    for row in top..top + thumb {
        frame
            .buffer_mut()
            .set_string(border, area.y + row as u16, "┃", style);
    }
}

fn prepare_task_layout(app: &App, width: u16, mut row_tasks: Vec<Option<usize>>) -> TaskLayout {
    row_tasks.clear();

    let inline = match &app.mode {
        Mode::Adding(_, placement) => Some(app.insertion_index(*placement)),
        Mode::Editing { index, .. } => Some(*index),
        _ => None,
    };

    let editing = matches!(app.mode, Mode::Editing { .. });

    let selected = if inline.is_some() {
        None
    } else {
        app.list.selected()
    };

    let mut lines = Vec::new();
    let mut input_row = None;
    let text_width = usize::from(width.saturating_sub(6));

    for index in 0..=app.tasks.len() {
        if let Some(position) = inline
            && position == index
        {
            input_row = Some(lines.len());
            lines.push(Line::default());
            row_tasks.push(editing.then_some(index));

            if editing {
                continue;
            }
        }

        let Some(task) = app.tasks.get(index) else {
            break;
        };

        if !app.is_visible(index) {
            continue;
        }

        // Inside a section tab every task shares the prefix, so show only the body.
        let in_section = matches!(app.view, SectionView::Named(_));

        let shown = if in_section {
            task.text.clone()
        } else {
            task.display_text()
        };

        let text: String = shown
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();

        // Bold the "Work:" part of the composed text; the section comes from its field.
        let section_prefix = if in_section {
            None
        } else {
            task.section.as_deref()
        };

        let tag_end = section_prefix.map_or(0, |prefix| {
            let end = prefix.len();
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

        let completed = task.status == TaskStatus::Completed;

        let text_style = if completed {
            app.styles.completed.add_modifier(Modifier::CROSSED_OUT)
        } else {
            app.styles.normal
        };

        // Matches are reversed, so the focused result's accent becomes their background and
        // the theme's background becomes their text, legible in light and dark themes alike.
        let match_color = if selected == Some(index) {
            app.styles.accent.fg.unwrap_or(Color::Reset)
        } else {
            Color::Reset
        };

        for (row, (source, text)) in wrapped.into_iter().enumerate() {
            row_tasks.push(Some(index));

            let prefix = if row > 0 {
                "     "
            } else if searching {
                if matches.is_empty() {
                    "  □  "
                } else {
                    "  ■  "
                }
            } else {
                task_checkbox(task, app.marker_style)
            };

            let marker_style = if searching {
                if matches.is_empty() {
                    app.styles.muted
                } else {
                    app.styles.accent
                }
            } else if selected == Some(index) {
                app.styles.accent
            } else if completed {
                app.styles.muted
            } else {
                app.styles.normal
            };

            let mut line = Line::from(vec![
                Span::styled(
                    if row == 0 && selected == Some(index) {
                        focus_marker(app.marker_style)
                    } else {
                        " "
                    },
                    app.styles.accent.add_modifier(Modifier::BOLD),
                ),
                Span::styled(prefix, marker_style),
            ]);

            line.spans.extend(task_spans(
                text,
                source.start,
                tag_end,
                &matches,
                text_style,
                match_color,
            ));
            lines.push(line);
        }
    }

    TaskLayout {
        lines,
        row_tasks,
        input_row,
        selected_task: selected,
    }
}

fn update_task_viewport(app: &mut App, layout: &TaskLayout, height: u16) -> Range<usize> {
    app.viewport_height = height;

    if layout.lines.is_empty() {
        app.scroll = 0;
        return 0..0;
    }

    let height = usize::from(height);
    app.scroll = app.scroll.min(layout.lines.len().saturating_sub(height));

    if height == 0 {
        return app.scroll..app.scroll;
    }

    let focus = layout.input_row.map(|row| (row, row + 1)).or_else(|| {
        layout.selected_task.map(|selected| {
            let start = layout
                .row_tasks
                .iter()
                .position(|&task| task == Some(selected))
                .unwrap_or(0);

            let end = layout
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

    app.scroll..(app.scroll + height).min(layout.lines.len())
}

fn draw_inline_input(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    input_row: Option<usize>,
    visible: Range<usize>,
) {
    let inline = match &app.mode {
        Mode::Adding(input, _) => Some((input, "add")),
        Mode::Editing { input, .. } => Some((input, "edit")),
        _ => None,
    };

    if let Some((input, label)) = inline
        && let Some(row) = input_row
        && row >= visible.start
        && row < visible.end
    {
        let indent = area.width.saturating_sub(1).min(2);
        draw_input_line(
            frame,
            Rect::new(
                area.x + indent,
                area.y + (row - visible.start) as u16,
                area.width - indent,
                1,
            ),
            input,
            label,
            &app.styles,
            true,
            true,
            app.completion_ghost().as_deref(),
        );
    }
}

fn border_type(style: BorderStyle) -> BorderType {
    match style {
        BorderStyle::Square => BorderType::Plain,
        BorderStyle::Rounded => BorderType::Rounded,
    }
}

fn focus_marker(style: MarkerStyle) -> &'static str {
    match style {
        MarkerStyle::Unicode => "›",
        MarkerStyle::Ascii => ">",
    }
}

fn task_checkbox(task: &Task, style: MarkerStyle) -> &'static str {
    match (task.status, style) {
        (TaskStatus::Pending, _) => " [ ] ",
        (TaskStatus::Completed, MarkerStyle::Unicode) => " [✓] ",
        (TaskStatus::Completed, MarkerStyle::Ascii) => " [x] ",
        (TaskStatus::Carried, MarkerStyle::Unicode) => " [↪] ",
        (TaskStatus::Carried, MarkerStyle::Ascii) => " [>] ",
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

fn task_spans(
    text: &str,
    offset: usize,
    tag_end: usize,
    matches: &[Range<usize>],
    base: Style,
    match_color: Color,
) -> Vec<Span<'static>> {
    let highlight = base
        .fg(match_color)
        .bg(base.bg.unwrap_or(Color::Reset))
        .remove_modifier(Modifier::DIM)
        .add_modifier(Modifier::REVERSED);

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

#[allow(clippy::too_many_arguments)]
fn draw_input_line(
    frame: &mut Frame,
    area: Rect,
    input: &TextInput,
    label: &str,
    styles: &Styles,
    show_cursor: bool,
    bold_section_prefix: bool,
    ghost: Option<&str>,
) {
    if area.is_empty() {
        return;
    }

    // Without a label the prompt is just the arrow, for inputs whose place says what they are.
    let mut line = if label.is_empty() {
        Line::from(Span::styled("› ", styles.accent))
    } else {
        Line::from(vec![
            Span::styled(label, styles.muted),
            Span::raw(" "),
            Span::styled("› ", styles.accent),
        ])
    };

    // Use a compact prompt when the full label would leave no cursor cell.
    if usize::from(area.width) <= line.width() {
        line = if area.width >= 3 {
            Line::from(Span::styled("› ", styles.accent))
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

    // A task input bolds a complete "Work: " prefix, as task rows bold their section. The
    // prefix ends at a grapheme boundary so a mark joined to its space stays with it.
    let prefix_end = if bold_section_prefix {
        task_prefix(text).map_or(0, |prefix| {
            let space = prefix.len() + 1;
            text.grapheme_indices(true)
                .find(|(index, _)| *index == space)
                .map_or(0, |(index, grapheme)| index + grapheme.len())
        })
    } else {
        0
    };

    let bold_end = prefix_end.clamp(start, end);
    line.push_span(Span::styled(
        &text[start..bold_end],
        styles.normal.add_modifier(Modifier::BOLD),
    ));
    line.push_span(Span::styled(&text[bold_end..end], styles.normal));

    // A suggested section follows the cursor in dim text, cut to the remaining cells.
    if let Some(ghost) = ghost {
        let mut room = usize::from(area.width).saturating_sub(prefix_width + visible_width);
        let mut shown = String::new();

        for grapheme in ghost.graphemes(true) {
            if grapheme.width() > room {
                break;
            }

            room -= grapheme.width();
            shown.push_str(grapheme);
        }

        line.push_span(Span::styled(shown, styles.muted));
    }

    frame.render_widget(Paragraph::new(line), area);
    let cursor = (prefix_width + width).min(usize::from(area.width - 1));

    if show_cursor {
        frame.set_cursor_position((area.x + cursor as u16, area.y));
    }
}

fn draw_confirmation(
    frame: &mut Frame,
    area: Rect,
    styles: &Styles,
    border_style: BorderStyle,
    question: &str,
) {
    let width = area.width.min(40);
    let height = area.height.min(7);

    let dialog = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );

    frame.render_widget(Clear, dialog);
    frame.render_widget(Block::default().style(styles.normal), dialog);
    let block = Block::bordered()
        .border_type(border_type(border_style))
        .border_style(styles.border);
    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    let lines = if inner.height >= 5 {
        vec![
            Line::raw(""),
            Line::raw(question),
            Line::raw(""),
            Line::raw("Enter: Confirm    Esc: Cancel"),
        ]
    } else if inner.height >= 2 {
        vec![
            Line::raw(question),
            Line::raw("Enter: Confirm  Esc: Cancel"),
        ]
    } else {
        vec![Line::raw(question)]
    };

    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(styles.normal),
        inner,
    );
}

// An action, its keys, and its Ctrl/Alt alternative ("" where either is missing).
type HelpRow = (&'static str, &'static str, &'static str);

// The `?` keyboard reference: headed groups of (action, keys, Ctrl/Alt alternative). Keep it
// in sync with the key dispatch in app.rs. Page Up/Down and Ctrl+V/Alt+V are deliberately
// left out, like everywhere else on screen.
const HELP_GROUPS: &[(&str, &[HelpRow])] = &[
    (
        "Tasks",
        &[
            ("Next / previous task", "j / k, ↓ / ↑", "Ctrl+N / Ctrl+P"),
            ("First / last task", "g / G, Home / End", ""),
            ("Half screen down / up", "", "Ctrl+D / Ctrl+U"),
            ("Repeat a move", "count first, e.g. 5j", ""),
        ],
    ),
    (
        "Sections",
        &[
            (
                "Next / previous tab",
                "l / h, Tab / Shift+Tab",
                "Alt+F / Alt+B",
            ),
            ("Go to tab 1–10", "s1 … s9, s0", "Alt+S, digit"),
            ("Find tab by name", "`", "Alt+G"),
            ("Move tab right / left", "L / H", "Alt+N / Alt+P"),
            ("Rename section", "E", ""),
        ],
    ),
    (
        "Changing tasks",
        &[
            ("Check / uncheck", "Space, Enter", ""),
            ("Append task", "a", ""),
            ("Add task below", "o", "Ctrl+O"),
            ("Add task above", "O", ""),
            ("Edit task", "e", "Alt+E"),
            ("Move task down / up", "J / K", "Alt+↓ / Alt+↑"),
            ("Toggle carryover", "m", ""),
            ("Delete task", "d, Delete", ""),
            ("Clear completed", "X", ""),
            ("Clear unmarked", "D", ""),
        ],
    ),
    (
        "Copy and undo",
        &[
            ("Copy task", "y", "Alt+W"),
            ("Paste below / above", "p / P", "Ctrl+Y (below)"),
            ("Undo", "u", "Ctrl+Z"),
            ("Redo", "", "Ctrl+R"),
            ("Copy list as Markdown", "c", ""),
        ],
    ),
    (
        "Other",
        &[
            ("Search", "/", "Ctrl+F, Ctrl+S"),
            ("Toggle wrapping", "w", ""),
            ("Next / previous theme", "t / T", ""),
            ("Keyboard shortcuts", "?", ""),
            ("Quit", "q", ""),
        ],
    ),
    (
        "While typing",
        &[
            ("Move cursor", "← / →", "Ctrl+B / Ctrl+F"),
            ("Start / end of text", "Home / End", "Ctrl+A / Ctrl+E"),
            ("Previous / next word", "Ctrl+← / Ctrl+→", "Alt+B / Alt+F"),
            ("Delete previous word", "Ctrl/Alt+Backspace", "Ctrl+W"),
            ("Delete next word", "", "Alt+D"),
            ("Delete character", "Delete", "Ctrl+D"),
            ("Delete to end / start", "", "Ctrl+K / Ctrl+U"),
            (
                "Next / previous match",
                "Tab / Shift+Tab",
                "Ctrl+N / Ctrl+P",
            ),
            ("Confirm", "Enter", ""),
            ("Cancel", "Esc", "Ctrl+G"),
        ],
    ),
];

// The side-by-side reference needs this many columns: its widest row and borders. Below it,
// each action's keys go on the line under it instead.
const HELP_WIDE_MIN_WIDTH: u16 = 65;

// Below either minimum there is no usable modal, so the reference asks for more room.
const HELP_MIN_WIDTH: u16 = 35;
const HELP_MIN_HEIGHT: u16 = 5;

const HELP_TOO_SMALL: &str = "Enlarge terminal to view shortcuts.";
const HELP_TITLE: &str = "Keyboard shortcuts";
const HELP_CLOSE_HINT: &str = "? or Esc to close";

// Cells before each action, between the side-by-side columns, and before the keys that the
// compact layout puts under an action.
const HELP_PADDING: usize = 1;
const HELP_COLUMN_GAP: usize = 2;
const HELP_KEYS_INDENT: usize = 3;

// Borders on both sides of the modal.
const HELP_BORDERS: usize = 2;

// Draw the keyboard reference centered over the checklist, scrolled to the offset in
// `Mode::Help`. The layout is picked from the terminal size on every draw, so resizing
// simply switches it; the offset is then clamped to the lines of whichever layout is shown.
fn draw_help(frame: &mut Frame, area: Rect, app: &mut App) {
    let styles = app.styles;

    if area.width < HELP_MIN_WIDTH || area.height < HELP_MIN_HEIGHT {
        frame.render_widget(Clear, area);
        frame.render_widget(Block::default().style(styles.normal), area);

        if area.is_empty() {
            return;
        }

        // Leave the offset alone, so it is clamped once a usable size returns.
        let message: Vec<Line> = wrap_text(HELP_TOO_SMALL, usize::from(area.width))
            .into_iter()
            .map(|(_, line)| Line::styled(line, styles.muted))
            .collect();
        let height = (message.len() as u16).min(area.height);
        let top = area.y + (area.height - height) / 2;

        frame.render_widget(
            Paragraph::new(message).alignment(Alignment::Center),
            Rect::new(area.x, top, area.width, height),
        );
        return;
    }

    let bindings = || HELP_GROUPS.iter().flat_map(|(_, rows)| rows.iter());
    let joined_keys = |keys: &str, alternative: &str| {
        [keys, alternative]
            .into_iter()
            .filter(|keys| !keys.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    };

    let action_width = bindings().map(|(action, ..)| action.width()).max();
    let keys_width = bindings().map(|(_, keys, _)| keys.width()).max();
    let alternative_width = bindings()
        .map(|(.., alternative)| alternative.width())
        .max();
    let action_width = action_width.unwrap_or(0);
    let keys_width = keys_width.unwrap_or(0);
    let alternative_width = alternative_width.unwrap_or(0);

    // Size the modal to its widest row, with the same padding on the right as on the left
    // when there is room for it.
    let wide = area.width >= HELP_WIDE_MIN_WIDTH;

    let row_width = if wide {
        HELP_PADDING
            + action_width
            + HELP_COLUMN_GAP
            + keys_width
            + HELP_COLUMN_GAP
            + alternative_width
    } else {
        bindings()
            .map(|(action, keys, alternative)| {
                let action_line = HELP_PADDING + action.width();
                let keys_line = HELP_KEYS_INDENT + joined_keys(keys, alternative).width();
                action_line.max(keys_line)
            })
            .max()
            .unwrap_or(0)
    };

    let natural_width = row_width + HELP_PADDING + HELP_BORDERS;
    let width = area.width.min(natural_width as u16);
    let inner_width = usize::from(width).saturating_sub(HELP_BORDERS);

    let padding = " ".repeat(HELP_PADDING);
    let keys_indent = " ".repeat(HELP_KEYS_INDENT);
    let mut lines = Vec::new();

    for (index, (heading, rows)) in HELP_GROUPS.iter().enumerate() {
        if index > 0 {
            lines.push(Line::default());
        }

        lines.push(Line::styled(format!("{padding}{heading}"), styles.accent));

        for (action, keys, alternative) in rows.iter() {
            if wide {
                let action_gap = " ".repeat(action_width - action.width() + HELP_COLUMN_GAP);
                let keys_gap = " ".repeat(keys_width - keys.width() + HELP_COLUMN_GAP);

                lines.push(Line::from(vec![
                    Span::styled(format!("{padding}{action}{action_gap}"), styles.normal),
                    Span::styled(format!("{keys}{keys_gap}{alternative}"), styles.muted),
                ]));
                continue;
            }

            lines.push(Line::styled(format!("{padding}{action}"), styles.normal));

            // Keys that don't fit on one line under the action get a line each; anything
            // still too long is cut off at the border.
            let joined = joined_keys(keys, alternative);

            if HELP_KEYS_INDENT + joined.width() <= inner_width {
                lines.push(Line::styled(format!("{keys_indent}{joined}"), styles.muted));
            } else {
                for keys in [keys, alternative]
                    .into_iter()
                    .filter(|keys| !keys.is_empty())
                {
                    lines.push(Line::styled(format!("{keys_indent}{keys}"), styles.muted));
                }
            }
        }
    }

    // Keep a row free above and below when there is room, so the reference reads as a modal.
    let available_height = if area.height >= HELP_MIN_HEIGHT + 2 {
        area.height - 2
    } else {
        area.height
    };
    let height = available_height.min((lines.len() + HELP_BORDERS) as u16);

    let dialog = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );

    frame.render_widget(Clear, dialog);
    frame.render_widget(Block::default().style(styles.normal), dialog);

    let mut block = Block::bordered()
        .border_type(border_type(app.border_style))
        .border_style(styles.border)
        .title(Line::from(vec![
            Span::styled("─ ", styles.border),
            Span::styled(HELP_TITLE, styles.accent),
            Span::styled(" ", styles.border),
        ]));

    if HELP_CLOSE_HINT.width() + HELP_BORDERS <= usize::from(width) {
        block = block.title_bottom(Line::styled(HELP_CLOSE_HINT, styles.muted).right_aligned());
    }

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    // Clamp the offset to this layout and size, and record the page for Page Up/Down.
    let visible_rows = usize::from(inner.height);
    let total_lines = lines.len();

    let Mode::Help {
        scroll,
        max_scroll,
        page,
    } = &mut app.mode
    else {
        return;
    };

    *max_scroll = total_lines.saturating_sub(visible_rows);
    *scroll = (*scroll).min(*max_scroll);
    *page = visible_rows;
    let first_line = *scroll;

    let visible: Vec<Line> = lines
        .into_iter()
        .skip(first_line)
        .take(visible_rows)
        .collect();

    frame.render_widget(Paragraph::new(visible), inner);
    draw_scrollbar(frame, inner, total_lines, first_line, styles.muted);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AddPlacement, Task};
    use crate::styles::DEFAULT_ACCENT;
    use ratatui::{Terminal, backend::TestBackend};

    #[track_caller]
    fn test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(width, height)).unwrap()
    }

    #[track_caller]
    fn render(terminal: &mut Terminal<TestBackend>, app: &mut App) -> Rect {
        let mut rows = Rect::default();
        terminal.draw(|frame| rows = draw(frame, app)).unwrap();

        rows
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

    fn row_text(terminal: &Terminal<TestBackend>, y: u16) -> String {
        screen(terminal)
            .lines()
            .nth(usize::from(y))
            .unwrap()
            .to_owned()
    }

    fn app_with_texts(texts: &[&str]) -> App {
        App::new(texts.iter().map(|text| Task::new(*text)).collect())
    }

    // For building lists that mix completed and pending tasks.
    fn completed_if(completed: bool) -> TaskStatus {
        if completed {
            TaskStatus::Completed
        } else {
            TaskStatus::Pending
        }
    }

    #[test]
    fn section_tabs_sit_on_the_top_border_in_fixed_positions() {
        let mut plain = app_with_texts(&["plain", "https://example.com"]);
        let mut terminal = test_terminal(40, 8);
        let rows = render(&mut terminal, &mut plain);

        assert_eq!(rows.y, 1);

        assert!(plain.tab_targets.is_empty());
        assert!(row_text(&terminal, 0).starts_with("┌──────"));

        let mut app = app_with_texts(&["Work: a", "home: b", "plain"]);
        app.styles.accent = app.styles.accent.fg(Color::Rgb(20, 40, 60));
        let rows = render(&mut terminal, &mut app);

        assert_eq!(rows.y, 2);
        assert_eq!(
            row_text(&terminal, 0),
            "┌─ [*] ─ [-] ─ home ─ Work ────────────┐"
        );
        assert_eq!(row_text(&terminal, 1).trim_matches(['│', ' ']), "");
        assert!(row_text(&terminal, 2).contains("Work: a"));

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(3, 0)].fg, Color::Rgb(20, 40, 60));
        assert!(!buffer[(3, 0)].modifier.contains(Modifier::BOLD));
        assert!(buffer[(9, 0)].modifier.contains(Modifier::DIM));

        // Switching tabs only recolors labels; the border row's text never changes.
        let expected = row_text(&terminal, 0);

        for (view, _) in app.section_tabs() {
            app.show_section(view.clone());
            render(&mut terminal, &mut app);

            assert_eq!(row_text(&terminal, 0), expected, "{view:?}");
        }

        app.border_style = BorderStyle::Rounded;
        render(&mut terminal, &mut app);
        assert!(row_text(&terminal, 0).starts_with("╭─ [*] ─ [-] ─ home ─ Work ─"));
    }

    #[test]
    fn a_section_tab_shows_only_its_tasks_without_the_prefix() {
        let mut app = app_with_texts(&["Work: fix bug", "Home: laundry", "Work: email"]);
        app.show_section(crate::app::SectionView::Named("work".into()));
        let mut terminal = test_terminal(40, 8);
        render(&mut terminal, &mut app);
        let screen = screen(&terminal);

        assert!(screen.contains("fix bug") && screen.contains("email"));
        assert!(!screen.contains("Work: ") && !screen.contains("laundry"));
        assert_eq!(app.row_tasks, [Some(0), Some(2)]);
        // Tabs sit on the border and the row under them is left blank, so tasks start on row 2.
        assert!(
            !terminal.backend().buffer()[(7, 2)]
                .modifier
                .contains(Modifier::BOLD)
        );

        app.show_section(crate::app::SectionView::All);
        render(&mut terminal, &mut app);

        assert!(row_text(&terminal, 2).contains("Work: fix bug"));
        assert!(
            terminal.backend().buffer()[(7, 2)]
                .modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn moving_a_section_on_a_narrow_terminal_keeps_its_tab_visible() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent};

        let fixture = crate::storage::tests::Fixture::new();
        let texts: Vec<String> = (0..8).map(|index| format!("S{index}: task")).collect();
        let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
        let mut app = app_with_texts(&texts);
        let mut terminal = test_terminal(24, 8);

        app.show_section(crate::app::SectionView::Named("s0".into()));
        render(&mut terminal, &mut app);

        for _ in 0..7 {
            app.key(KeyEvent::from(KeyCode::Char('L')), &fixture.storage)
                .unwrap();
            render(&mut terminal, &mut app);

            assert!(row_text(&terminal, 0).contains(" S0 "));
        }

        assert_eq!(app.section_order.last().map(String::as_str), Some("s0"));

        for _ in 0..7 {
            app.key(KeyEvent::from(KeyCode::Char('H')), &fixture.storage)
                .unwrap();
            render(&mut terminal, &mut app);

            assert!(row_text(&terminal, 0).contains(" S0 "));
        }

        assert_eq!(app.section_order.first().map(String::as_str), Some("s0"));
    }

    #[test]
    fn overflowing_tabs_scroll_only_to_keep_the_active_tab_visible() {
        let texts: Vec<String> = (0..12)
            .map(|index| format!("Section{index:02}: task"))
            .collect();
        let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
        let mut app = app_with_texts(&texts);
        let mut terminal = test_terminal(30, 8);

        render(&mut terminal, &mut app);
        let first = row_text(&terminal, 0);
        assert!(first.starts_with("┌─ [*] ─ Section00 ›"), "{first}");
        assert!(first.ends_with("─┐"), "{first}");

        app.show_section(crate::app::SectionView::Named("section11".into()));
        render(&mut terminal, &mut app);
        let last = row_text(&terminal, 0);

        assert!(
            last.starts_with("┌‹ ") && last.contains(" Section11 "),
            "{last}"
        );
        assert!(!last.contains('›') && last.ends_with("─┐"), "{last}");

        for (area, _) in &app.tab_targets {
            assert!(area.right() < 29);
        }

        // Within the visible window, moving back and forth never slides the tabs.
        let fixture = crate::storage::tests::Fixture::new();
        let mut wide = test_terminal(60, 8);
        app.show_section(crate::app::SectionView::All);
        render(&mut wide, &mut app);
        let initial = row_text(&wide, 0);

        let tab = |app: &mut App, code| {
            app.key(
                ratatui::crossterm::event::KeyEvent::new(
                    code,
                    ratatui::crossterm::event::KeyModifiers::NONE,
                ),
                &fixture.storage,
            )
            .unwrap();
        };

        for _ in 0..4 {
            tab(&mut app, ratatui::crossterm::event::KeyCode::Tab);
            render(&mut wide, &mut app);
            assert_eq!(row_text(&wide, 0), initial);
        }

        tab(&mut app, ratatui::crossterm::event::KeyCode::Tab);
        render(&mut wide, &mut app);
        let scrolled = row_text(&wide, 0);
        assert_ne!(scrolled, initial);

        tab(&mut app, ratatui::crossterm::event::KeyCode::BackTab);
        render(&mut wide, &mut app);
        assert_eq!(row_text(&wide, 0), scrolled);

        // A two-row box still shows tabs on its border; a single row has no room.
        let mut two_rows = test_terminal(30, 2);
        render(&mut two_rows, &mut app);
        assert!(!app.tab_targets.is_empty());

        // With one inner row, the task keeps it instead of the blank row.
        let mut three_rows = test_terminal(30, 3);
        let rows = render(&mut three_rows, &mut app);
        assert_eq!((rows.y, rows.height), (1, 1));

        let mut one_row = test_terminal(30, 1);
        render(&mut one_row, &mut app);
        assert!(app.tab_targets.is_empty());
    }

    #[test]
    fn the_clear_confirmation_names_the_current_tab() {
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b"]);
        app.tasks[1].status = TaskStatus::Completed;
        let mut terminal = test_terminal(80, 14);

        app.show_section(crate::app::SectionView::Named("work".into()));
        app.key(
            ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Char('D'),
                ratatui::crossterm::event::KeyModifiers::NONE,
            ),
            &fixture.storage,
        )
        .unwrap();
        render(&mut terminal, &mut app);

        assert!(screen(&terminal).contains("Clear unmarked Work tasks?"));
    }

    #[test]
    fn the_clear_confirmation_uses_the_configured_border_style() {
        for (border_style, corners) in [
            (BorderStyle::Square, ['┌', '┐', '└', '┘']),
            (BorderStyle::Rounded, ['╭', '╮', '╰', '╯']),
        ] {
            let mut app = app_with_texts(&["a"]);
            app.border_style = border_style;
            app.mode = Mode::ConfirmClear;
            let mut terminal = test_terminal(80, 20);

            render(&mut terminal, &mut app);

            // Skip the list's own border columns so only the dialog's corners are counted.
            let dialog_corners = screen(&terminal)
                .lines()
                .flat_map(|line| line.chars().skip(1).take(78).collect::<Vec<_>>())
                .filter(|c| corners.contains(c))
                .count();

            assert!(screen(&terminal).contains("Clear unmarked tasks?"));
            assert_eq!(dialog_corners, 4, "{border_style:?}");
        }
    }

    #[test]
    fn a_suggested_section_is_drawn_dim_after_the_cursor() {
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b"]);
        app.show_section(crate::app::SectionView::Named("work".into()));
        app.key(
            ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Char('a'),
                ratatui::crossterm::event::KeyModifiers::NONE,
            ),
            &fixture.storage,
        )
        .unwrap();
        let mut terminal = test_terminal(30, 8);
        render(&mut terminal, &mut app);

        // The add row follows the one visible task, below the blank row under the tabs.
        let row = row_text(&terminal, 3);
        assert!(row.contains("add › Work:"), "{row}");

        let cursor = terminal.get_cursor_position().unwrap();
        let ghost = &terminal.backend().buffer()[(cursor.x, cursor.y)];

        assert_eq!(ghost.symbol(), "W");
        assert!(ghost.modifier.contains(Modifier::DIM));

        let mut narrow = test_terminal(12, 8);
        render(&mut narrow, &mut app);
        assert!(
            screen(&narrow)
                .lines()
                .all(|line| line.chars().count() <= 12)
        );
    }

    fn renaming_app(texts: &[&str], name: &str) -> App {
        let mut app = app_with_texts(texts);
        app.show_section(SectionView::Named("work".into()));
        app.mode = Mode::RenamingSection { input: name.into() };

        app
    }

    #[test]
    fn renaming_draws_the_input_in_the_active_tab_with_the_cursor() {
        let mut app = renaming_app(&["Work: a", "Home: b"], "Jo");
        let mut terminal = test_terminal(40, 8);
        render(&mut terminal, &mut app);

        // Tabs after the input shift right while it is wider than the label.
        assert_eq!(
            row_text(&terminal, 0),
            "┌─ [*] ─ Home ─ rename › Jo  ──────────┐"
        );

        let cursor = terminal.get_cursor_position().unwrap();
        assert_eq!((cursor.x, cursor.y), (27, 0));

        let text = &terminal.backend().buffer()[(25, 0)];
        assert!(!text.modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    fn a_taken_rename_is_struck_through_and_muted() {
        let mut app = renaming_app(&["Work: a", "Home: b"], "home");
        let mut terminal = test_terminal(40, 8);
        render(&mut terminal, &mut app);

        let text = &terminal.backend().buffer()[(25, 0)];

        assert_eq!(text.symbol(), "h");
        assert!(
            text.modifier
                .contains(Modifier::CROSSED_OUT | Modifier::DIM)
        );
    }

    #[test]
    fn a_long_rename_stays_within_a_narrow_tab_bar() {
        let mut app = renaming_app(&["Work: a", "Home: b"], "AVeryLongSectionName");
        let mut terminal = test_terminal(16, 8);
        render(&mut terminal, &mut app);

        let top = row_text(&terminal, 0);
        let cursor = terminal.get_cursor_position().unwrap();

        assert_eq!(top.chars().count(), 16, "{top}");
        assert!(top.ends_with('┐'), "{top}");
        assert!(top.contains("› "), "{top}");
        assert_eq!(cursor.y, 0);
        assert!(cursor.x < 15);
    }

    fn press_keys(app: &mut App, fixture: &crate::storage::tests::Fixture, keys: &str) {
        use ratatui::crossterm::event::{KeyCode, KeyEvent};

        for character in keys.chars() {
            app.key(KeyEvent::from(KeyCode::Char(character)), &fixture.storage)
                .unwrap();
        }
    }

    #[test]
    fn a_pending_section_jump_numbers_the_tabs_under_their_names() {
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b", "plain"]);
        let mut terminal = test_terminal(40, 8);

        press_keys(&mut app, &fixture, "s");
        render(&mut terminal, &mut app);

        assert_eq!(
            row_text(&terminal, 0),
            "┌─ [*] ─ [-] ─ Home ─ Work ────────────┐"
        );
        assert_eq!(
            row_text(&terminal, 1),
            "│   1     2    3      4                │"
        );

        let digit_style = terminal.backend().buffer()[(4, 1)].style();
        assert_eq!(digit_style.fg, app.styles.accent.fg);
    }

    #[test]
    fn section_jump_numbers_disappear_after_the_next_key() {
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b"]);
        let mut terminal = test_terminal(40, 8);

        press_keys(&mut app, &fixture, "sx");
        render(&mut terminal, &mut app);

        assert_eq!(
            row_text(&terminal, 1),
            "│                                      │"
        );
    }

    #[test]
    fn section_jump_numbers_stop_at_the_tenth_tab() {
        let fixture = crate::storage::tests::Fixture::new();
        let texts: Vec<String> = ('A'..='J')
            .map(|letter| format!("{letter}: task"))
            .collect();
        let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
        let mut app = app_with_texts(&texts);
        let mut terminal = test_terminal(80, 8);

        press_keys(&mut app, &fixture, "s");
        render(&mut terminal, &mut app);

        assert_eq!(
            row_text(&terminal, 0),
            "┌─ [*] ─ A ─ B ─ C ─ D ─ E ─ F ─ G ─ H ─ I ─ J ────────────────────────────────┐"
        );
        assert_eq!(
            row_text(&terminal, 1),
            "│   1    2   3   4   5   6   7   8   9   0                                     │"
        );
    }

    #[test]
    fn section_jump_numbers_are_hidden_when_there_is_no_blank_row_under_the_tabs() {
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b"]);
        let mut terminal = test_terminal(40, 3);

        press_keys(&mut app, &fixture, "s");
        render(&mut terminal, &mut app);

        assert!(!row_text(&terminal, 1).contains('1'));
    }

    #[test]
    fn the_section_picker_replaces_all_with_its_input_and_follows_it_with_matches() {
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b"]);
        let mut terminal = test_terminal(40, 8);

        press_keys(&mut app, &fixture, "`");
        render(&mut terminal, &mut app);

        assert_eq!(
            row_text(&terminal, 0),
            "┌─ ›   ─ Home ─ Work ──────────────────┐"
        );
        let cursor = terminal.get_cursor_position().unwrap();
        assert_eq!((cursor.x, cursor.y), (5, 0));

        press_keys(&mut app, &fixture, "wo");
        render(&mut terminal, &mut app);

        assert_eq!(
            row_text(&terminal, 0),
            "┌─ › wo  ─ Work ───────────────────────┐"
        );
        let cursor = terminal.get_cursor_position().unwrap();
        assert_eq!((cursor.x, cursor.y), (7, 0));

        // The previewed match is accented and its tasks are shown.
        assert_eq!(terminal.backend().buffer()[(11, 0)].fg, DEFAULT_ACCENT);
        assert!(screen(&terminal).contains(" a"));
        assert!(!screen(&terminal).contains(" b"));
    }

    #[test]
    fn overflowing_picker_matches_keep_the_focused_one_visible_without_moving_tab_scroll() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent};

        let fixture = crate::storage::tests::Fixture::new();
        let mut app = app_with_texts(&[
            "Alpha: a",
            "Bravo: b",
            "Charlie: c",
            "Delta: d",
            "Echo: e",
            "Foxtrot: f",
        ]);
        let mut terminal = test_terminal(30, 8);

        press_keys(&mut app, &fixture, "`");
        render(&mut terminal, &mut app);

        let top = row_text(&terminal, 0);
        assert!(top.contains(" Alpha ") && !top.contains("Foxtrot"), "{top}");

        app.key(KeyEvent::from(KeyCode::BackTab), &fixture.storage)
            .unwrap();
        render(&mut terminal, &mut app);

        let top = row_text(&terminal, 0);
        assert!(top.contains(" Foxtrot ") && top.contains('‹'), "{top}");
        assert_eq!(app.tab_scroll, 0);

        app.key(KeyEvent::from(KeyCode::Tab), &fixture.storage)
            .unwrap();
        render(&mut terminal, &mut app);

        let top = row_text(&terminal, 0);
        assert!(top.contains(" Alpha ") && !top.contains('‹'), "{top}");
    }

    #[test]
    fn a_long_picker_query_stays_within_its_share_of_a_narrow_tab_bar() {
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = app_with_texts(&["Alphabetical: a", "Alpaca: b"]);
        let mut terminal = test_terminal(30, 8);

        press_keys(&mut app, &fixture, "`alphabetical");
        render(&mut terminal, &mut app);

        let top = row_text(&terminal, 0);
        let cursor = terminal.get_cursor_position().unwrap();

        assert_eq!(top.chars().count(), 30, "{top}");
        assert!(top.ends_with('┐'), "{top}");
        assert!(
            top.contains("› ") && top.contains(" Alphabetical "),
            "{top}"
        );
        assert_eq!(cursor.y, 0);
        assert!(cursor.x < 12, "{cursor:?}");
    }

    #[test]
    fn a_long_focused_match_is_shortened_beside_the_picker_input() {
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = app_with_texts(&["Supercalifragilistic: a", "Home: b"]);
        let mut terminal = test_terminal(24, 8);

        press_keys(&mut app, &fixture, "`sup");
        render(&mut terminal, &mut app);

        let top = row_text(&terminal, 0);

        assert_eq!(top, "┌─ › sup  ─ Supercal… ─┐");
        assert_eq!(terminal.backend().buffer()[(12, 0)].fg, DEFAULT_ACCENT);
    }

    #[test]
    fn a_long_active_section_tab_is_shortened_on_a_narrow_terminal() {
        let mut app = app_with_texts(&["Supercalifragilistic: a", "Home: b"]);
        app.show_section(SectionView::Named("supercalifragilistic".into()));
        let mut terminal = test_terminal(16, 8);

        render(&mut terminal, &mut app);

        assert_eq!(row_text(&terminal, 0), "┌‹ Supercali… ─┐");
        assert_eq!(terminal.backend().buffer()[(3, 0)].fg, DEFAULT_ACCENT);
    }

    #[test]
    fn tiny_terminals_and_resizing_are_safe_while_picking_a_section() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let fixture = crate::storage::tests::Fixture::new();
        let mut app = app_with_texts(&["日本語: 🪷 task", "LongSectionName: b", "plain"]);
        let mut terminal = test_terminal(40, 12);

        press_keys(&mut app, &fixture, "`");
        let clear_query = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL);

        // An empty query focuses each tab in turn, including the shortened long one; a long
        // Unicode query matches nothing and has to clip inside the input.
        for query in ["", "日本語 🪷 long query 日本語"] {
            app.key(clear_query, &fixture.storage).unwrap();
            app.paste(query);

            for width in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 16, 40] {
                for height in [0, 1, 2, 3, 6, 12] {
                    terminal.backend_mut().resize(width, height);

                    for _ in 0..3 {
                        app.key(KeyEvent::from(KeyCode::Tab), &fixture.storage)
                            .unwrap();

                        let rows = render(&mut terminal, &mut app);

                        assert!(rows.right() <= width && rows.bottom() <= height);

                        if terminal.backend().cursor_visible() {
                            let buffer = terminal.backend().buffer();
                            assert!(buffer.area.contains(terminal.backend().cursor_position()));
                        }
                    }
                }
            }
        }

        assert!(matches!(app.mode, Mode::FuzzySection { .. }));
    }

    fn thumb_rows(terminal: &Terminal<TestBackend>, x: u16) -> Vec<u16> {
        let buffer = terminal.backend().buffer();

        (0..buffer.area.height)
            .filter(|&y| buffer[(x, y)].symbol() == "┃")
            .collect()
    }

    #[test]
    fn scrollbar_appears_on_the_right_border_only_when_tasks_overflow() {
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = test_terminal(30, 8);
        let mut few = App::new(
            (0..3)
                .map(|index| Task::new(format!("task {index}")))
                .collect(),
        );
        render(&mut terminal, &mut few);

        assert!(thumb_rows(&terminal, 29).is_empty());

        let mut app = App::new(
            (0..20)
                .map(|index| Task::new(format!("task {index}")))
                .collect(),
        );
        let rows = render(&mut terminal, &mut app);
        let first = rows.y;
        let last = rows.bottom() - 1;

        // Six visible rows of twenty: a two-row thumb starting at the top.
        let thumb = thumb_rows(&terminal, 29);
        assert_eq!(thumb, [first, first + 1]);
        assert!(
            terminal.backend().buffer()[(29, first)]
                .modifier
                .contains(Modifier::DIM)
        );
        assert_eq!(terminal.backend().buffer()[(0, first)].symbol(), "│");
        assert!(row_text(&terminal, first).contains("task 0"));

        let key = |app: &mut App, code| {
            app.key(
                ratatui::crossterm::event::KeyEvent::new(
                    code,
                    ratatui::crossterm::event::KeyModifiers::NONE,
                ),
                &fixture.storage,
            )
            .unwrap();
        };

        key(&mut app, ratatui::crossterm::event::KeyCode::Char('G'));
        render(&mut terminal, &mut app);
        assert_eq!(thumb_rows(&terminal, 29).last(), Some(&last));

        app.list.select(Some(10));
        app.reveal_selection = true;
        render(&mut terminal, &mut app);
        let middle = thumb_rows(&terminal, 29);
        assert!(
            middle[0] > first && *middle.last().unwrap() < last,
            "{middle:?}"
        );

        // Rows, not task counts: two long wrapped tasks overflow a six-row list.
        let mut wrapped = App::new(
            (0..2)
                .map(|_| Task::new("wrapped words ".repeat(12).trim_end()))
                .collect(),
        );
        render(&mut terminal, &mut wrapped);
        assert!(!thumb_rows(&terminal, 29).is_empty());

        // With section tabs, the thumb stays beside task rows, below the blank row.
        let mut sectioned = App::new(
            (0..20)
                .map(|index| Task::new(format!("Work: task {index}")))
                .collect(),
        );
        let rows = render(&mut terminal, &mut sectioned);

        assert_eq!(rows.y, 2);
        assert_eq!(thumb_rows(&terminal, 29)[0], 2);
    }

    #[test]
    fn clicking_a_tab_shows_that_section() {
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = app_with_texts(&["Work: a", "Home: b"]);
        let mut terminal = test_terminal(80, 12);
        let rows = render(&mut terminal, &mut app);

        let (area, view) = app.tab_targets[1].clone();
        assert_eq!(view, crate::app::SectionView::Named("home".into()));

        app.mouse(
            ratatui::crossterm::event::MouseEvent {
                kind: ratatui::crossterm::event::MouseEventKind::Down(
                    ratatui::crossterm::event::MouseButton::Left,
                ),
                column: area.x,
                row: area.y,
                modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
            },
            rows,
            &fixture.storage,
        )
        .unwrap();

        assert_eq!(app.view, view);
        assert_eq!(app.list.selected(), Some(1));
        assert!(
            app.tasks
                .iter()
                .all(|task| task.status != TaskStatus::Completed)
        );
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn semantic_styles_and_reverse_highlights_preserve_modifier_hierarchy() {
        assert_eq!(DEFAULT_ACCENT, Color::LightYellow);
        assert!(Styles::default().muted.add_modifier.contains(Modifier::DIM));
        assert!(
            !Styles::default()
                .muted
                .patch(Styles::default().normal)
                .add_modifier
                .contains(Modifier::DIM)
        );

        for accent in [Color::Yellow, Color::Indexed(3), Color::Rgb(20, 40, 60)] {
            assert!(
                !Styles::default()
                    .muted
                    .patch(Styles::default().accent.fg(accent))
                    .add_modifier
                    .contains(Modifier::DIM)
            );

            let mut app = App::new(vec![Task {
                status: TaskStatus::Completed,
                ..Task::new("Work: text")
            }]);
            app.styles.accent = app.styles.accent.fg(accent);
            app.mode = Mode::Searching {
                input: "Work".into(),
                original: Some(0),
            };
            let mut terminal = test_terminal(40, 10);
            render(&mut terminal, &mut app);
            let buffer = terminal.backend().buffer();
            // Tabs sit on the border and the row under them is left blank, so tasks start on row 2.
            let marker = &buffer[(4, 2)];

            assert_eq!(marker.fg, accent);
            assert!(!marker.modifier.contains(Modifier::DIM));

            // The only task is the focused result, so its match sits on the accent.
            for x in 7..11 {
                let cell = &buffer[(x, 2)];
                assert_eq!((cell.fg, cell.bg), (accent, Color::Reset));
                assert!(
                    cell.modifier
                        .contains(Modifier::REVERSED | Modifier::BOLD | Modifier::CROSSED_OUT)
                );
                assert!(!cell.modifier.contains(Modifier::DIM));
            }

            let ordinary = &buffer[(13, 2)];
            assert!(
                ordinary
                    .modifier
                    .contains(Modifier::DIM | Modifier::CROSSED_OUT)
            );
            assert!(!ordinary.modifier.contains(Modifier::REVERSED));
        }

        let spans = task_spans(
            "abc",
            0,
            0,
            std::slice::from_ref(&(1..2)),
            Styles::default().muted.add_modifier(Modifier::ITALIC),
            Color::Reset,
        );

        assert!(
            spans[1]
                .style
                .add_modifier
                .contains(Modifier::ITALIC | Modifier::REVERSED)
        );
        assert!(!spans[1].style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn focus_is_bold_accented_in_both_marker_modes_without_bolding_task() {
        for marker_style in [MarkerStyle::Ascii, MarkerStyle::Unicode] {
            for accent in [DEFAULT_ACCENT, Color::Indexed(3), Color::Rgb(12, 34, 56)] {
                let mut app = App::new(vec![Task::new("ordinary text")]);
                app.marker_style = marker_style;
                app.styles.accent = app.styles.accent.fg(accent);
                let mut terminal = test_terminal(40, 10);
                render(&mut terminal, &mut app);
                let buffer = terminal.backend().buffer();
                let focus = &buffer[(1, 1)];

                assert_eq!(focus.symbol(), focus_marker(marker_style));
                assert_eq!(focus.fg, accent);
                assert!(focus.modifier.contains(Modifier::BOLD));

                for x in [3, 7] {
                    assert!(!buffer[(x, 1)].modifier.contains(Modifier::BOLD));
                }
            }
        }
    }

    #[test]
    fn rendering_uses_resolved_roles_without_reconstructing_defaults() {
        let mut app = App::new(vec![
            Task::new("ordinary"),
            Task {
                status: TaskStatus::Completed,
                ..Task::new("completed")
            },
        ]);
        app.styles.normal = app.styles.normal.fg(Color::Red);
        app.styles.muted = app.styles.muted.fg(Color::Green);
        app.styles.completed = app.styles.completed.fg(Color::Blue);
        app.styles.border = app.styles.border.fg(Color::Cyan);
        app.styles.accent = app.styles.accent.fg(Color::Magenta);
        let mut terminal = test_terminal(60, 12);
        render(&mut terminal, &mut app);
        let buffer = terminal.backend().buffer();

        assert_eq!(buffer[(0, 0)].fg, Color::Cyan);
        assert_eq!(buffer[(1, 1)].fg, Color::Magenta);
        assert!(buffer[(1, 1)].modifier.contains(Modifier::BOLD));
        assert_eq!(buffer[(7, 1)].fg, Color::Red);
        assert_eq!(buffer[(7, 2)].fg, Color::Blue);
        assert!(buffer[(7, 2)].modifier.contains(Modifier::CROSSED_OUT));

        let mut input_terminal = test_terminal(40, 1);
        input_terminal
            .draw(|frame| {
                draw_input_line(
                    frame,
                    frame.area(),
                    &TextInput::from("draft"),
                    "add",
                    &app.styles,
                    true,
                    false,
                    None,
                )
            })
            .unwrap();
        let buffer = input_terminal.backend().buffer();

        assert_eq!(buffer[(0, 0)].fg, Color::Green);
        assert_eq!(buffer[(4, 0)].fg, Color::Magenta);
        assert_eq!(buffer[(6, 0)].fg, Color::Red);
    }

    #[test]
    fn configured_styles_preserve_mandatory_focus_completion_and_search_modifiers() {
        for modifiers in ["[]", "['dim']"] {
            let config: crate::config::Config = toml::from_str(&format!("[styles.normal]\nmodifiers=['bold']\n[styles.muted]\nmodifiers=['italic']\n[styles.completed]\nfg='red'\nmodifiers={modifiers}\n[styles.accent]\nfg='blue'\nmodifiers=['italic','underlined']")).unwrap();

            let mut app = App::new(vec![Task {
                status: TaskStatus::Completed,
                ..Task::new("Work: task")
            }]);
            app.styles = Styles::resolve(config.theme, &config.styles, None);
            let mut terminal = test_terminal(40, 12);
            render(&mut terminal, &mut app);
            let buffer = terminal.backend().buffer();

            // Tabs sit on the border and the row under them is left blank, so tasks start on row 2.
            assert_eq!(buffer[(1, 2)].fg, Color::Blue);
            assert!(
                buffer[(1, 2)]
                    .modifier
                    .contains(Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED)
            );
            assert!(!buffer[(3, 2)].modifier.contains(Modifier::BOLD));
            assert!(
                buffer[(7, 2)]
                    .modifier
                    .contains(Modifier::BOLD | Modifier::CROSSED_OUT)
            );
            assert_eq!(buffer[(13, 2)].fg, Color::Red);
            assert_eq!(
                buffer[(13, 2)].modifier.contains(Modifier::DIM),
                modifiers != "[]"
            );
            assert!(buffer[(13, 2)].modifier.contains(Modifier::CROSSED_OUT));

            app.mode = Mode::Searching {
                input: "Work".into(),
                original: Some(0),
            };
            render(&mut terminal, &mut app);
            let cell = &terminal.backend().buffer()[(7, 2)];

            assert_eq!((cell.fg, cell.bg), (Color::Blue, Color::Reset));
            assert!(
                cell.modifier
                    .contains(Modifier::REVERSED | Modifier::BOLD | Modifier::CROSSED_OUT)
            );
            assert!(!cell.modifier.contains(Modifier::DIM));
        }
    }

    #[test]
    fn themed_background_fills_every_cell_including_empty_regions_and_dialogs() {
        for mode in 0..8 {
            for width in [1, 12, 40, 60, 80] {
                let mut app = App::new(vec![Task {
                    status: TaskStatus::Completed,
                    ..Task::new("Work: long wrapped task words")
                }]);

                let config: crate::config::Config =
                    toml::from_str("theme='nord'\n[styles.normal]\nbg='#20242d'").unwrap();
                app.styles = Styles::resolve(config.theme, &config.styles, None);
                app.mode = match mode {
                    0 => Mode::Normal,
                    1 => Mode::Adding("draft".into(), AddPlacement::End),
                    2 => Mode::Editing {
                        index: 0,
                        input: "edited".into(),
                    },
                    3 => Mode::Searching {
                        input: "Work".into(),
                        original: Some(0),
                    },
                    4 => Mode::Searching {
                        input: "".into(),
                        original: Some(0),
                    },
                    5 => Mode::ConfirmClear,
                    6 => Mode::Help {
                        scroll: 3,
                        max_scroll: 0,
                        page: 0,
                    },
                    _ => {
                        app.show_section(SectionView::Named("work".into()));
                        Mode::RenamingSection {
                            input: "Job".into(),
                        }
                    }
                };
                let mut terminal = test_terminal(width, 12);
                render(&mut terminal, &mut app);

                for cell in &terminal.backend().buffer().content {
                    assert_eq!(
                        cell.bg,
                        Color::Rgb(32, 36, 45),
                        "mode {mode}, width {width}, cell {cell:?}"
                    );
                }
            }
        }

        let mut app = App::new(Vec::new());
        let config: crate::config::Config =
            toml::from_str("theme='nord'\n[styles.border]\nbg='#112233'").unwrap();
        app.styles = Styles::resolve(config.theme, &config.styles, None);
        let mut terminal = test_terminal(40, 12);
        render(&mut terminal, &mut app);

        assert_eq!(
            terminal.backend().buffer()[(0, 0)].bg,
            Color::Rgb(17, 34, 51)
        );
        assert_eq!(
            terminal.backend().buffer()[(1, 1)].bg,
            Color::Rgb(46, 52, 64)
        );
    }

    fn wrapped_strings(text: &str, width: usize) -> Vec<&str> {
        wrap_text(text, width)
            .into_iter()
            .map(|(_, line)| line)
            .collect()
    }
    #[test]
    fn copy_feedback_uses_the_border_without_changing_the_viewport() {
        for width in [0, 1, 2, 8, 15, 17, 24, 32, 80] {
            for border_style in [BorderStyle::Square, BorderStyle::Rounded] {
                let mut app = App::new(Vec::new());
                app.border_style = border_style;
                let mut terminal = test_terminal(width, 12);
                let before = render(&mut terminal, &mut app);
                app.copy_markdown(&mut Vec::new());

                for notice in [
                    "Copy sent to terminal",
                    "Copy failed: output unavailable",
                    "Copied task",
                    "Theme: Everforest Dark / Medium (everforest-medium)",
                    "Theme: Rosé Pine (rose-pine)",
                    "Theme: Default (default)",
                ] {
                    app.status_message = Some(crate::status_message::StatusMessage::new(
                        notice,
                        crate::status_message::DEFAULT_DURATION,
                    ));
                    let after = render(&mut terminal, &mut app);
                    assert_eq!(before, after);

                    let text = screen(&terminal);

                    if width == 80 {
                        assert!(text.contains(notice));
                    } else if width == 24 || width == 32 {
                        let compact = app.status_message.as_ref().unwrap().compact_message();

                        if notice.starts_with("Theme: ") {
                            assert_eq!(
                                text.contains(compact),
                                compact.width() <= usize::from(width) - 2
                            );
                        } else {
                            assert!(text.contains(if notice == "Copied task" {
                                "Copied task"
                            } else if notice.starts_with("Copy failed") {
                                "Copy failed"
                            } else {
                                "Copy sent"
                            }));
                        }
                    }

                    if width >= 2 {
                        let buffer = terminal.backend().buffer();
                        assert_eq!(
                            buffer[(0, after.bottom())].symbol(),
                            if border_style == BorderStyle::Rounded {
                                "╰"
                            } else {
                                "└"
                            }
                        );
                        assert_eq!(
                            buffer[(width - 1, after.bottom())].symbol(),
                            if border_style == BorderStyle::Rounded {
                                "╯"
                            } else {
                                "┘"
                            }
                        );
                    }
                }

                app.status_message = None;
                render(&mut terminal, &mut app);
                assert!(!screen(&terminal).contains("Copy"));
            }
        }
    }

    #[test]
    fn search_enter_selects_the_focused_match_but_keeps_search_open_without_one() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

        for (query, count) in [("missing", 0), ("task 18", 1), ("task 1", 11)] {
            let fixture = crate::storage::tests::Fixture::new();

            let mut app = App::new(
                (0..20)
                    .map(|index| Task {
                        status: completed_if(index >= 10),
                        ..Task::new(format!("{}end", format!("task {index} ").repeat(12)))
                    })
                    .collect(),
            );
            let original = app.tasks.clone();
            let mut terminal = test_terminal(24, 6);
            render(&mut terminal, &mut app);

            assert_eq!(app.scroll, 0);

            for accept in [false, true] {
                app.key(
                    KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
                    &fixture.storage,
                )
                .unwrap();
                app.paste(query);
                render(&mut terminal, &mut app);
                let selected = app.list.selected();
                app.key(
                    KeyEvent::new(
                        if accept { KeyCode::Enter } else { KeyCode::Esc },
                        KeyModifiers::NONE,
                    ),
                    &fixture.storage,
                )
                .unwrap();

                if !accept {
                    assert_eq!(app.mode, Mode::Normal);
                    assert_eq!(app.list.selected(), Some(0));
                } else if count == 0 {
                    assert!(matches!(app.mode, Mode::Searching { .. }));
                } else {
                    assert_eq!(app.mode, Mode::Normal);
                    assert_eq!(app.list.selected(), selected);

                    app.key(
                        KeyEvent::new_with_kind(
                            KeyCode::Enter,
                            KeyModifiers::NONE,
                            KeyEventKind::Repeat,
                        ),
                        &fixture.storage,
                    )
                    .unwrap();
                }

                let rows = render(&mut terminal, &mut app);

                if let Some(selected) = app.list.selected() {
                    let end = (app.scroll + usize::from(rows.height)).min(app.row_tasks.len());
                    assert!(app.row_tasks[app.scroll..end].contains(&Some(selected)));
                }

                if accept && count == 1 {
                    let rendered = screen(&terminal);
                    assert_eq!(selected, Some(18));
                    assert!(rendered.contains("[✓] task 18"));
                    assert!(!rendered.contains('■') && !rendered.contains('□'));
                    assert!(!rendered.contains("search ›"));
                }

                assert_eq!(app.tasks, original);
                assert!(!app.can_undo() && !app.can_redo());
                assert!(!fixture.task_path().exists());
            }
        }
    }

    #[test]
    fn unicode_checkboxes_are_single_cell_and_preserve_rendering_and_search() {
        for glyph in ["\u{2713}", "\u{21aa}", ">", "\u{203a}"] {
            assert_eq!(glyph.width(), 1);
        }

        assert_eq!(focus_marker(MarkerStyle::Ascii), ">");
        assert_eq!(focus_marker(MarkerStyle::Unicode), "\u{203a}");

        let tasks = vec![
            Task::new("ordinary task wraps here"),
            Task {
                status: TaskStatus::Completed,
                ..Task::new("done task wraps here")
            },
            Task {
                status: TaskStatus::Carried,
                ..Task::new("carried task wraps here")
            },
        ];

        for (task, ascii, unicode) in [
            (&tasks[0], " [ ] ", " [ ] "),
            (&tasks[1], " [x] ", " [\u{2713}] "),
            (&tasks[2], " [>] ", " [\u{21aa}] "),
        ] {
            assert_eq!(task_checkbox(task, MarkerStyle::Ascii), ascii);
            assert_eq!(task_checkbox(task, MarkerStyle::Unicode), unicode);
            assert_eq!(ascii.width(), 5);
            assert_eq!(unicode.width(), 5);
        }

        for width in [0, 1, 2, 3, 8, 18, 40] {
            for wrap in [false, true] {
                for mode in [
                    Mode::Normal,
                    Mode::Searching {
                        input: "done".into(),
                        original: Some(1),
                    },
                ] {
                    let searching = mode != Mode::Normal;
                    let mut app = App::new(tasks.clone());
                    assert_eq!(app.marker_style, MarkerStyle::Unicode);

                    app.marker_style = MarkerStyle::Ascii;
                    app.mode = mode;
                    app.list.select(Some(1));
                    app.wrap = wrap;
                    app.styles.accent = app.styles.accent.fg(Color::Rgb(169, 182, 101));
                    let mut terminal = test_terminal(width, 12);
                    let rows = render(&mut terminal, &mut app);
                    let ascii = terminal.backend().buffer().clone();
                    let mapping = app.row_tasks.clone();
                    let scroll = app.scroll;
                    app.marker_style = MarkerStyle::Unicode;
                    let unicode_rows = render(&mut terminal, &mut app);

                    assert_eq!(rows, unicode_rows);
                    assert_eq!(app.row_tasks, mapping);
                    assert_eq!(app.scroll, scroll);

                    let mut unicode = terminal.backend().buffer().clone();

                    for (offset, cell) in unicode.content.iter_mut().enumerate() {
                        match cell.symbol() {
                            "›" if offset % usize::from(width) == usize::from(rows.x) => {
                                cell.set_symbol(">");
                            }
                            "✓" if !searching => {
                                cell.set_symbol("x");
                            }
                            "↪" if !searching => {
                                cell.set_symbol(">");
                            }
                            _ => {}
                        }
                    }

                    assert_eq!(ascii, unicode);
                    assert_eq!(app.tasks, tasks);
                    assert!(!app.can_undo() && !app.can_redo());
                }
            }
        }
    }

    #[test]
    fn carryover_checkboxes_preserve_layout_and_search_markers() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();

        let mut app = App::new(vec![Task {
            status: TaskStatus::Carried,
            ..Task::new("carry words across wrapped lines")
        }]);
        app.styles.accent = app.styles.accent.fg(Color::Rgb(169, 182, 101));
        let mut terminal = test_terminal(24, 12);
        render(&mut terminal, &mut app);
        let mapping = app.row_tasks.clone();

        assert!(mapping.len() > 1);
        assert!(screen(&terminal).contains("› [↪] carry"));
        assert_eq!(
            terminal.backend().buffer()[(3, 1)].fg,
            app.styles.accent.fg.unwrap()
        );
        assert_eq!(terminal.backend().buffer()[(1, 2)].symbol(), " ");

        app.key(
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
            &fixture.storage,
        )
        .unwrap();
        app.paste("carry");
        render(&mut terminal, &mut app);

        assert_eq!(app.row_tasks, mapping);
        assert!(screen(&terminal).contains('■'));
        assert!(!screen(&terminal).contains("[↪]"));

        app.key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &fixture.storage,
        )
        .unwrap();
        render(&mut terminal, &mut app);

        assert_eq!(app.row_tasks, mapping);
        assert!(screen(&terminal).contains("[↪]"));
        assert_eq!(app.tasks[0].status, TaskStatus::Carried);
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn inline_add_preview_matches_saved_position_without_changing_tasks() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        for count in [0usize, 4] {
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
                                status: completed_if(index % 2 == 0),
                                ..Task::new(format!("task {index}"))
                            })
                            .collect(),
                    );
                    app.list.select(selected);
                    app.styles.accent = app.styles.accent.fg(Color::Rgb(169, 182, 101));
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
                    let mut terminal = test_terminal(40, 10);
                    let rows = render(&mut terminal, &mut app);

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
                    assert_eq!(buffer[(rows.x + 2, y)].fg, Color::Reset);
                    assert_eq!(buffer[(rows.x + 6, y)].fg, app.styles.accent.fg.unwrap());
                    assert_eq!(buffer[(rows.x + 8, y)].fg, Color::Reset);
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
                    assert_ne!(app.tasks[index].status, TaskStatus::Completed);
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

                    render(&mut terminal, &mut app);
                    assert!(app.row_tasks.iter().all(Option::is_some));
                    assert!(!screen(&terminal).contains("add ›"));
                    assert!(!terminal.backend().cursor_visible());
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
                Task::new("before"),
                Task {
                    status: TaskStatus::Completed,
                    ..Task::new(format!("{} end", "original e\u{301}界👩‍💻 ".repeat(30)))
                },
                Task {
                    status: TaskStatus::Completed,
                    ..Task::new("after")
                },
            ]);
            app.list.select(Some(1));
            fixture.storage.save(&app.tasks).unwrap();
            let original = app.tasks.clone();
            let bytes = std::fs::read(fixture.task_path()).unwrap();
            let mut terminal = test_terminal(26, 8);
            let mut rows = render(&mut terminal, &mut app);

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

            rows = render(&mut terminal, &mut app);
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
            render(&mut terminal, &mut app);

            assert!(screen(&terminal).contains("edit › New original"));
            assert_eq!(app.tasks, original);
            assert_eq!(std::fs::read(fixture.task_path()).unwrap(), bytes);
            assert!(!app.can_undo());

            app.key(KeyEvent::new(ending, KeyModifiers::NONE), &fixture.storage)
                .unwrap();
            rows = render(&mut terminal, &mut app);

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
                assert_eq!(app.tasks[1].status, TaskStatus::Completed);

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
                        status: completed_if(index >= 10),
                        ..Task::new(format!("task {index} {}end", "e\u{301}界👩‍💻 ".repeat(12)))
                    })
                    .collect(),
            );
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

            let mut terminal = test_terminal(40, 12);

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
                            let rows = render(&mut terminal, &mut app);

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
            let rows = render(&mut terminal, &mut app);
            let end = (app.scroll + usize::from(rows.height)).min(app.row_tasks.len());

            assert!(app.row_tasks[app.scroll..end].contains(&Some(18)));
            assert_eq!(app.tasks, original);
            assert_eq!(app.list.selected(), Some(18));
            assert!(!app.can_undo() && !app.can_redo());
            assert!(!fixture.task_path().exists());
        }
    }

    #[test]
    fn search_keeps_its_bottom_prompt_and_cursor_while_tab_cycles_matches() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();

        let mut app = App::new(vec![
            Task {
                status: TaskStatus::Completed,
                ..Task::new("needle")
            },
            Task {
                status: TaskStatus::Completed,
                ..Task::new("another needle")
            },
        ]);
        app.key(
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
            &fixture.storage,
        )
        .unwrap();
        app.paste("needle");
        let mut terminal = test_terminal(40, 12);

        for _ in 0..2 {
            let rows = render(&mut terminal, &mut app);
            assert_eq!(rows.height, 9);
            assert!(
                screen(&terminal)
                    .lines()
                    .last()
                    .unwrap()
                    .starts_with("search › needle")
            );
            assert_eq!(app.row_tasks, [Some(0), Some(1)]);
            assert!(screen(&terminal).contains("■"));
            assert!(terminal.backend().cursor_visible());
            assert_eq!(terminal.backend().cursor_position().y, 11);

            app.key(
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                &fixture.storage,
            )
            .unwrap();
        }

        app.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &fixture.storage,
        )
        .unwrap();
        render(&mut terminal, &mut app);
        assert_eq!(app.list.selected(), Some(0));
        assert!(screen(&terminal).contains("[✓] needle"));
        assert!(!screen(&terminal).contains("search ›"));
        assert_eq!(app.tasks[0].status, TaskStatus::Completed);
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn tab_hint_sits_at_the_right_of_the_search_row_only_when_it_fits_after_the_query() {
        let mut app = App::new(vec![Task::new("needle")]);

        for (width, query, shown) in [
            (40, "", true),
            (40, "needle", true),
            (30, "needle", true),
            (29, "needle", false),
            (40, "a much longer search query", false),
            (12, "", false),
        ] {
            app.mode = Mode::Searching {
                input: query.into(),
                original: Some(0),
            };
            let mut terminal = test_terminal(width, 8);

            render(&mut terminal, &mut app);
            let rendered = screen(&terminal);
            let search_row = rendered.lines().last().unwrap();

            assert!(search_row.contains("search ›"), "{width} {query:?}");
            assert_eq!(
                search_row.ends_with(SEARCH_HINT),
                shown,
                "{width} {query:?}: {search_row:?}"
            );
            assert_eq!(rendered.contains(SEARCH_HINT), shown);
        }

        app.mode = Mode::Normal;
        let mut terminal = test_terminal(40, 8);
        render(&mut terminal, &mut app);

        assert!(!screen(&terminal).contains(SEARCH_HINT));
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
                status: completed_if(completed),
                ..Task::new(text)
            })
            .into(),
        );

        let original = app.tasks.clone();
        app.list.select(Some(2));
        app.styles.accent = app.styles.accent.fg(Color::Rgb(20, 40, 60));
        let mut terminal = test_terminal(40, 10);
        let key = |app: &mut App, code| {
            app.key(KeyEvent::new(code, KeyModifiers::NONE), &fixture.storage)
                .unwrap()
        };
        key(&mut app, KeyCode::Char('/'));
        app.paste("i\u{307}foo");
        render(&mut terminal, &mut app);
        let buffer = terminal.backend().buffer();

        // Tabs sit on the border and the row under them is left blank, so tasks start on row 2.
        assert_eq!(buffer[(1, 2)].symbol(), "›");
        assert_eq!(buffer[(1, 4)].symbol(), " ");

        for y in [2, 3] {
            assert_eq!(buffer[(4, y)].symbol(), "■");
            assert_eq!(buffer[(4, y)].fg, app.styles.accent.fg.unwrap());
        }

        assert_eq!(buffer[(4, 4)].symbol(), "□");
        assert_eq!(buffer[(4, 4)].fg, Color::Reset);
        assert_eq!(buffer[(7, 2)].symbol(), "w");
        assert!(buffer[(7, 2)].modifier.contains(Modifier::BOLD));

        // Only the focused result's matches take the accent.
        let accent = app.styles.accent.fg.unwrap();

        for (y, expected, match_fg) in [(2, "İfooİFOO", accent), (3, "İfoo", Color::Reset)] {
            let mut highlighted = String::new();

            for x in 7..39 {
                let cell = &buffer[(x, y)];

                if cell.modifier.contains(Modifier::REVERSED) {
                    highlighted.push_str(cell.symbol());
                    assert_eq!(cell.fg, match_fg);
                    assert_eq!(cell.bg, Color::Reset);
                    assert!(!cell.modifier.contains(Modifier::DIM));
                    assert_eq!(cell.modifier.contains(Modifier::CROSSED_OUT), y == 3);
                }
            }

            assert_eq!(highlighted, expected);
        }

        key(&mut app, KeyCode::Tab);
        render(&mut terminal, &mut app);
        assert!(screen(&terminal).contains("search › i\u{307}foo"));

        // The accent follows focus to the completed task and leaves the first one.
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(13, 2)].fg, Color::Reset);
        assert!(buffer[(13, 2)].modifier.contains(Modifier::REVERSED));
        assert_eq!(buffer[(7, 3)].fg, accent);
        assert!(buffer[(7, 3)].modifier.contains(Modifier::REVERSED));

        key(&mut app, KeyCode::Enter);
        render(&mut terminal, &mut app);

        assert_eq!(app.list.selected(), Some(1));
        assert_eq!(app.tasks, original);
        assert_eq!(terminal.backend().buffer()[(4, 2)].symbol(), " ");
        assert_eq!(terminal.backend().buffer()[(4, 3)].symbol(), "✓");

        for y in 2..5 {
            for x in 7..39 {
                assert_eq!(terminal.backend().buffer()[(x, y)].bg, Color::Reset);
            }
        }

        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn focused_search_matches_use_the_accent_across_wrapped_rows_and_repeated_matches() {
        let accent = Color::Rgb(20, 40, 60);
        let background = Color::Rgb(250, 248, 240);

        let mut app = App::new(vec![Task::new("abcdef abcdefabcdef")]);
        app.styles.normal = app.styles.normal.bg(background);
        app.styles.accent = app.styles.accent.fg(accent);
        app.mode = Mode::Searching {
            input: "abcdef".into(),
            original: Some(0),
        };
        app.list.select(Some(0));
        let mut terminal = test_terminal(17, 8);

        render(&mut terminal, &mut app);

        // Text starts at column 7 with nine cells per row: "abcdef", "abcdefabc", "def", so
        // the last of the three matches spans a wrap.
        let buffer = terminal.backend().buffer();
        let mut highlighted = String::new();

        for y in 1..7 {
            for x in 7..16 {
                let cell = &buffer[(x, y)];

                if cell.modifier.contains(Modifier::REVERSED) {
                    highlighted.push_str(cell.symbol());
                    assert_eq!(cell.fg, accent);
                    assert_eq!(cell.bg, background);
                } else if cell.symbol() != " " {
                    assert_ne!(cell.fg, accent);
                    assert_eq!(cell.bg, background);
                }
            }
        }

        assert_eq!(highlighted, "abcdefabcdefabcdef");
    }

    #[test]
    fn search_spanning_wrapped_rows_preserves_mapping_and_reveals_results_after_resize() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();

        let mut app = App::new(vec![
            Task::new("tag: abCDef abcdef"),
            Task::new("nonmatching"),
            Task {
                status: TaskStatus::Completed,
                ..Task::new(format!("{}tag: abcdef", "long ".repeat(50)))
            },
        ]);
        let original = app.tasks.clone();
        let mut terminal = test_terminal(14, 9);
        render(&mut terminal, &mut app);
        let mapping = app.row_tasks.clone();
        app.key(
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
            &fixture.storage,
        )
        .unwrap();
        app.paste("tag: abcd");
        render(&mut terminal, &mut app);

        assert_eq!(app.row_tasks, mapping);

        // Tabs sit on the border and the row under them is left blank, so tasks start on row 2.
        for (y, length) in [(2, 4), (3, 4)] {
            for x in 7..7 + length {
                assert!(
                    terminal.backend().buffer()[(x, y)]
                        .modifier
                        .contains(Modifier::REVERSED)
                );
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
            render(&mut terminal, &mut app);
            app.key(
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                &fixture.storage,
            )
            .unwrap();
            render(&mut terminal, &mut app);
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
        render(&mut terminal, &mut app);

        assert_eq!(app.list.selected(), Some(0));
        assert_eq!(app.tasks, original);
        assert!(!app.can_undo());
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn search_without_matches_says_so_and_keeps_markers_aligned() {
        let fixture = crate::storage::tests::Fixture::new();

        let mut app = App::new(vec![Task {
            status: TaskStatus::Completed,
            ..Task::new("ordinary")
        }]);
        app.mode = Mode::Searching {
            input: "missing".into(),
            original: Some(0),
        };
        app.list.select(None);
        let mut terminal = test_terminal(60, 12);
        render(&mut terminal, &mut app);

        assert!(screen(&terminal).contains("No matches"));
        assert_eq!(terminal.backend().buffer()[(1, 1)].symbol(), " ");
        assert_eq!(terminal.backend().buffer()[(4, 1)].symbol(), "□");
        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn redo_restores_a_wrapped_selection_into_view() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();

        let mut app = App::new(vec![
            Task::new("wrapped task ".repeat(40).trim_end()),
            Task::new("other active task"),
            Task {
                status: TaskStatus::Completed,
                ..Task::new("done")
            },
        ]);
        let key = |app: &mut App, code, modifiers| {
            app.key(KeyEvent::new(code, modifiers), &fixture.storage)
                .unwrap()
        };

        key(&mut app, KeyCode::Char(' '), KeyModifiers::NONE);
        let modified = app.tasks.clone();
        key(&mut app, KeyCode::Char('u'), KeyModifiers::NONE);
        assert!(app.can_redo());

        let mut terminal = test_terminal(18, 7);
        app.scroll = usize::MAX;
        key(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);

        assert_eq!(app.tasks, modified);

        render(&mut terminal, &mut app);
        assert_eq!(app.list.selected(), Some(0));
        assert_eq!(app.row_tasks[app.scroll], Some(0));
        assert!(!app.can_redo());

        key(&mut app, KeyCode::Char('D'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('u'), KeyModifiers::NONE);

        assert!(app.can_redo());
    }

    #[test]
    fn inline_inputs_keep_the_viewport_height_selection_and_mouse_mapping() {
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let fixture = crate::storage::tests::Fixture::new();

        let mut app = App::new(
            (0..12)
                .map(|index| Task::new(format!("task {index} {}", "wrapped words ".repeat(4))))
                .collect(),
        );

        let mut terminal = test_terminal(60, 12);
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
            let rows = render(&mut terminal, &mut app);
            heights.push(rows.height);
            let end = (app.scroll + usize::from(rows.height)).min(app.row_tasks.len());

            assert!(app.row_tasks[app.scroll..end].contains(&Some(11)));

            if app.mode == Mode::Normal {
                let index = app.row_tasks[app.scroll].unwrap();
                let before = app.tasks[index].status;
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

                assert_ne!(app.tasks[index].status, before);

                app.list.select(Some(11));
                app.reveal_selection = true;
            }
        }

        // Add and edit live inside the checklist, so only the borders take rows.
        assert_eq!(heights, [10, 10, 10, 10]);
    }

    #[test]
    fn tiny_terminals_and_resizing_are_safe_in_every_mode() {
        let mut terminal = test_terminal(40, 12);

        let mut app = App::new(vec![Task::new("日本語 🪷 ".repeat(8).trim_end())]);
        app.styles = Styles::resolve(
            crate::themes::BuiltinTheme::Nord,
            &crate::config::StyleOverrides::default(),
            None,
        );

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
                    Mode::ConfirmClear,
                    Mode::Help {
                        scroll: usize::MAX,
                        max_scroll: usize::MAX,
                        page: 0,
                    },
                ] {
                    app.mode = mode;
                    let mut rows = Rect::default();
                    let completed = terminal.draw(|frame| rows = draw(frame, &mut app)).unwrap();

                    // TestBackend can retain stale placeholders after overlapping wide glyphs.
                    // Verify backgrounds on the authoritative completed frame instead.
                    assert!(
                        completed
                            .buffer
                            .content
                            .iter()
                            .all(|cell| cell.bg == Color::Rgb(46, 52, 64))
                    );
                    assert!(rows.right() <= width && rows.bottom() <= height);
                    assert!(app.row_tasks.iter().flatten().all(|&index| index == 0));

                    let buffer = terminal.backend().buffer();

                    if terminal.backend().cursor_visible() {
                        assert!(buffer.area.contains(terminal.backend().cursor_position()));
                    }
                }
            }
        }
    }

    #[test]
    fn page_navigation_targets_visual_rows_in_mixed_task_heights() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = test_terminal(18, 5);

        let mut app = App::new(
            [
                "short",
                "abcd abcd abcd",
                "short",
                "abcd abcd abcd abcd abcd",
                "short",
            ]
            .map(Task::new)
            .into(),
        );
        let mut rows = render(&mut terminal, &mut app);
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

            rows = render(&mut terminal, &mut app);
            let end = (app.scroll + usize::from(rows.height)).min(app.row_tasks.len());
            assert!(app.row_tasks[app.scroll..end].contains(&Some(selected)));
        }

        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn half_page_navigation_targets_visual_rows_in_mixed_task_heights() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = test_terminal(18, 7);

        let mut app = App::new(
            [
                "short",
                "abcd abcd abcd",
                "short",
                "abcd abcd abcd abcd abcd",
                "short",
            ]
            .map(Task::new)
            .into(),
        );
        let mut rows = render(&mut terminal, &mut app);
        assert_eq!(rows.height, 5);
        assert_eq!(app.row_tasks, [0, 1, 1, 2, 3, 3, 3, 4].map(Some));

        // Each step covers two of the five rows, so the selection crosses tall tasks a row
        // at a time but still advances at least one task, and stops at either end.
        for (character, selected) in [
            ('d', 1),
            ('d', 2),
            ('d', 3),
            ('d', 4),
            ('d', 4),
            ('u', 3),
            ('u', 1),
            ('u', 0),
            ('u', 0),
        ] {
            let key = KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL);
            app.key(key, &fixture.storage).unwrap();
            assert_eq!(app.list.selected(), Some(selected), "Ctrl+{character}");

            rows = render(&mut terminal, &mut app);
            let end = (app.scroll + usize::from(rows.height)).min(app.row_tasks.len());
            assert!(app.row_tasks[app.scroll..end].contains(&Some(selected)));
        }

        assert!(!fixture.task_path().exists());
    }

    #[test]
    fn page_navigation_uses_current_viewport_wrapping_and_resize() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = test_terminal(40, 12);

        let mut app = App::new(
            (0..40)
                .map(|index| Task::new(format!("task {index}")))
                .collect(),
        );

        // Page size excludes the borders.
        let mut rows = render(&mut terminal, &mut app);
        assert_eq!(app.viewport_height, rows.height);
        assert_eq!(rows.height, 10);

        let original = app.tasks.clone();

        let mut press = |app: &mut App, code, kind| {
            app.key(
                KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind),
                &fixture.storage,
            )
            .unwrap();
            rows = render(&mut terminal, app);
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

        app.tasks[1].text = "word ".repeat(60);
        terminal.backend_mut().resize(18, 8);
        render(&mut terminal, &mut app);
        let page = KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE);
        let up = KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE);
        app.key(page, &fixture.storage).unwrap();

        assert_eq!(app.list.selected(), Some(1));

        render(&mut terminal, &mut app);
        // Simulate reading partway through the oversized selected task with the mouse wheel.
        app.scroll += 5;
        app.reveal_selection = false;
        render(&mut terminal, &mut app);
        app.key(page, &fixture.storage).unwrap();

        assert_eq!(app.list.selected(), Some(2));

        render(&mut terminal, &mut app);
        app.key(up, &fixture.storage).unwrap();
        assert_eq!(app.list.selected(), Some(1));

        render(&mut terminal, &mut app);
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
            render(&mut terminal, &mut app);
            let expected = usize::from(app.viewport_height.max(1));
            app.key(page, &fixture.storage).unwrap();

            // Only task 1 wraps: with wrapping off, or a one-cell width, each task has one row.
            assert_eq!(
                app.list.selected(),
                Some(if wrap && width > 6 { 1 } else { expected })
            );

            rows = render(&mut terminal, &mut app);

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
        let mut terminal = test_terminal(40, 10);

        let mut app = App::new(
            (0..1000)
                .map(|index| Task::new(format!("task {index}")))
                .collect(),
        );

        app.navigate(false, None);
        render(&mut terminal, &mut app);
        assert!(app.scroll > 0);
        assert!(screen(&terminal).contains("task 999"));

        app.navigate(true, None);
        render(&mut terminal, &mut app);
        assert_eq!(app.scroll, 0);
        assert!(screen(&terminal).contains("task 0"));
    }

    #[test]
    fn only_leading_tags_are_bold_across_wrapping_and_completion_styles() {
        for (text, tag) in [
            ("foo: first bar:", "foo:"),
            ("foo:bar", ""),
            ("https://example.com", ""),
            ("10:30 call", ""),
            ("no tag: task", ""),
            ("task foo:", ""),
            (":empty", ""),
            ("e\u{301}👩‍💻: body", "e\u{301}👩‍💻:"),
            ("abcdefghijkl: body", "abcdefghijkl:"),
            ("a:\u{301} body", ""),
        ] {
            for completed in [false, true] {
                for (width, wrap) in [(12, true), (40, true), (40, false)] {
                    let mut app = App::new(vec![Task {
                        status: completed_if(completed),
                        ..Task::new(text)
                    }]);

                    let original = app.tasks.clone();
                    app.wrap = wrap;
                    let mut terminal = test_terminal(width, 20);
                    render(&mut terminal, &mut app);
                    let buffer = terminal.backend().buffer();
                    let mut bold = String::new();

                    // A section adds tabs and a blank row under them, so the task moves to row 2.
                    let first_row = if tag.is_empty() { 1 } else { 2 };

                    for y in first_row..19 {
                        for x in 7..width - 1 {
                            let cell = &buffer[(x, y)];

                            if cell.symbol() == " " {
                                continue;
                            }

                            if cell.modifier.contains(Modifier::BOLD) {
                                bold.push_str(cell.symbol());
                            }

                            assert_eq!(cell.fg, Color::Reset);
                            assert_eq!(cell.modifier.contains(Modifier::CROSSED_OUT), completed);
                            assert_eq!(cell.modifier.contains(Modifier::DIM), completed);
                        }
                    }

                    assert_eq!(bold, tag, "{text:?}, width={width}, wrap={wrap}");
                    assert!(!buffer[(3, first_row)].modifier.contains(Modifier::BOLD));
                    assert_eq!(buffer[(1, first_row)].fg, app.styles.accent.fg.unwrap());
                    assert_eq!(app.tasks, original);
                    assert!(!app.can_undo());
                }
            }
        }
    }

    #[test]
    fn completed_text_stays_muted_with_accented_marker_and_checkbox() {
        let mut terminal = test_terminal(40, 10);

        let mut app = App::new(vec![Task {
            status: TaskStatus::Completed,
            ..Task::new("Completed")
        }]);
        app.styles.accent = app.styles.accent.fg(Color::Rgb(169, 182, 101));
        render(&mut terminal, &mut app);
        let buffer = terminal.backend().buffer();
        let marker = &buffer[(1, 1)];
        let checkbox = &buffer[(3, 1)];
        let text = &buffer[(7, 1)];

        assert_eq!(marker.symbol(), "›");
        assert_eq!(marker.fg, app.styles.accent.fg.unwrap());
        assert_eq!(marker.bg, Color::Reset);
        assert_eq!(checkbox.fg, app.styles.accent.fg.unwrap());
        assert_eq!(text.fg, Color::Reset);
        assert!(text.modifier.contains(Modifier::CROSSED_OUT));
        assert!(text.modifier.contains(Modifier::DIM));
        assert_eq!(text.bg, Color::Reset);
    }

    #[test]
    fn disabled_wrapping_keeps_one_row_and_input_shows_its_tail() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = test_terminal(12, 8);

        let mut app = App::new(vec![Task::new("日本語".repeat(100)), Task::new("next")]);

        let toggle = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE);
        app.key(toggle, &fixture.storage).unwrap();
        assert!(!app.wrap);

        app.mode = Mode::Adding("a long input ending in 界z".into(), AddPlacement::End);
        render(&mut terminal, &mut app);
        let rendered = screen(&terminal);

        assert!(rendered.contains("[ ] next"));
        assert!(rendered.contains("add › z"));

        app.mode = Mode::Normal;
        app.key(toggle, &fixture.storage).unwrap();
        assert!(app.wrap);

        render(&mut terminal, &mut app);
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
                let mut app = App::new(vec![Task::new(text.clone())]);
                app.mode = if edit {
                    Mode::Editing {
                        index: 0,
                        input: text.clone().into(),
                    }
                } else {
                    Mode::Adding(text.clone().into(), AddPlacement::End)
                };
                let mut terminal = test_terminal(width, 5);

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
                    render(&mut terminal, &mut app);
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
                let mut terminal = test_terminal(width, 1);
                terminal
                    .draw(|frame| {
                        draw_input_line(
                            frame,
                            frame.area(),
                            &input,
                            label,
                            &Styles::default(),
                            true,
                            false,
                            None,
                        )
                    })
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
            let mut terminal = test_terminal(40, 1);
            terminal
                .draw(|frame| {
                    draw_input_line(
                        frame,
                        frame.area(),
                        &input,
                        label,
                        &Styles::default(),
                        true,
                        false,
                        None,
                    )
                })
                .unwrap();

            assert_eq!(
                usize::from(terminal.get_cursor_position().unwrap().x),
                label.width() + 3
            );
            assert!(screen(&terminal).contains("hidden text"));
        }
    }

    // Draws `text` in an input line and returns which cells after the prompt are bold.
    fn input_bold_cells(text: &str, width: u16, bold_section_prefix: bool) -> Vec<bool> {
        let mut terminal = test_terminal(width, 1);
        let input = TextInput::from(text);

        terminal
            .draw(|frame| {
                draw_input_line(
                    frame,
                    frame.area(),
                    &input,
                    "",
                    &Styles::default(),
                    true,
                    bold_section_prefix,
                    None,
                )
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let prompt_width = "› ".width() as u16;

        (prompt_width..width)
            .map(|x| buffer[(x, 0)].modifier.contains(Modifier::BOLD))
            .collect()
    }

    #[test]
    fn task_inputs_bold_only_a_complete_section_prefix() {
        let complete = input_bold_cells("Wörk: fix", 20, true);
        let without_space = input_bold_cells("Work:fix", 20, true);
        let partial = input_bold_cells("Work:", 20, true);
        let other_input = input_bold_cells("Work: fix", 20, false);

        assert_eq!(
            &complete[..9],
            [true, true, true, true, true, true, false, false, false]
        );
        assert!(without_space.iter().all(|bold| !bold));
        assert!(partial.iter().all(|bold| !bold));
        assert!(other_input.iter().all(|bold| !bold));
    }

    #[test]
    fn scrolled_task_inputs_keep_bolding_the_visible_part_of_the_prefix() {
        // The cursor sits at the end, so the start of "Section: " scrolls out of view.
        let bold = input_bold_cells("Section: abc", 10, true);

        // Visible: "on: abc" and the cursor cell.
        assert_eq!(bold, [true, true, true, true, false, false, false, false]);
    }

    #[test]
    fn input_tail_and_cursor_follow_graphemes_and_cell_widths() {
        for label in ["add", "edit"] {
            for tail in ["e\u{301}", "👩‍💻", "👨‍👩‍👧‍👦", "👍🏽", "🇺🇳", "👩‍💻a"]
            {
                let prefix_width = label.width() + 3;
                let cursor = (prefix_width + tail.width()) as u16;
                let mut terminal = test_terminal(cursor + 1, 1);
                let input = TextInput::from(format!("hidden text {tail}"));
                let accent = Color::Rgb(169, 182, 101);
                terminal
                    .draw(|frame| {
                        draw_input_line(
                            frame,
                            frame.area(),
                            &input,
                            label,
                            &Styles {
                                accent: Styles::default().accent.fg(accent),
                                ..Styles::default()
                            },
                            true,
                            false,
                            None,
                        )
                    })
                    .unwrap();

                assert_eq!(screen(&terminal).trim_end(), format!("{label} › {tail}"));
                assert_eq!(terminal.get_cursor_position().unwrap(), (cursor, 0).into());

                let buffer = terminal.backend().buffer();
                assert_eq!(buffer[(cursor, 0)].symbol(), " ");
                assert_eq!(buffer[(0, 0)].fg, Color::Reset);
                assert_eq!(buffer[((label.width() + 1) as u16, 0)].fg, accent);
                assert_eq!(buffer[(prefix_width as u16, 0)].fg, Color::Reset);
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
                    let mut terminal = test_terminal(width, height);
                    terminal
                        .draw(|frame| {
                            draw_input_line(
                                frame,
                                frame.area(),
                                &TextInput::from("hidden 👩‍💻"),
                                label,
                                &Styles::default(),
                                true,
                                false,
                                None,
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

        let mut app = App::new(["abcd \u{301}X", "last"].map(Task::new).into());
        let mut terminal = test_terminal(12, 5);
        let rows = render(&mut terminal, &mut app);

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

        assert_eq!(app.tasks[0].status, TaskStatus::Completed);
        assert_ne!(app.tasks[1].status, TaskStatus::Completed);
        assert_eq!(fixture.storage.load().unwrap(), app.tasks);
        assert_eq!(app.tasks[0].text, "abcd \u{301}X");

        for (code, selected) in [(KeyCode::PageDown, 1), (KeyCode::PageUp, 0)] {
            app.key(KeyEvent::new(code, KeyModifiers::NONE), &fixture.storage)
                .unwrap();
            render(&mut terminal, &mut app);

            assert_eq!(app.list.selected(), Some(selected));
        }

        terminal.backend_mut().resize(13, 5);
        render(&mut terminal, &mut app);
        assert_eq!(app.row_tasks, [0, 0, 1].map(Some));
        assert_eq!(terminal.backend().buffer()[(11, 1)].symbol(), " \u{301}");
    }

    #[test]
    fn continuation_clicks_toggle_the_correct_task_and_leave_other_rows_alone() {
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = test_terminal(18, 12);

        let mut app = App::new(vec![Task::new("alpha beta gamma delta"), Task::new("next")]);

        let rows = render(&mut terminal, &mut app);
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
        assert_eq!(app.tasks[0].status, TaskStatus::Completed);
        assert_ne!(app.tasks[1].status, TaskStatus::Completed);

        app.mouse(click(4), rows, &fixture.storage).unwrap();
        assert_eq!(app.tasks[1].status, TaskStatus::Completed);

        app.mouse(click(5), rows, &fixture.storage).unwrap();
        assert!(
            app.tasks
                .iter()
                .all(|task| task.status == TaskStatus::Completed)
        );
        assert_eq!(fixture.storage.load().unwrap(), app.tasks);
    }

    #[test]
    fn oversized_task_can_be_scrolled_read_and_navigated_after_resizing() {
        use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
        let fixture = crate::storage::tests::Fixture::new();
        let mut terminal = test_terminal(18, 8);

        let mut app = App::new(vec![
            Task::new(format!("{}THE END", "word ".repeat(30))),
            Task::new("next"),
        ]);

        let mut rows = render(&mut terminal, &mut app);
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
            rows = render(&mut terminal, &mut app);
        }

        assert!(screen(&terminal).contains("THE END"));
        assert!(screen(&terminal).contains("next"));

        app.navigate(true, None);
        render(&mut terminal, &mut app);
        assert_eq!(app.list.selected(), Some(1));

        app.navigate(false, None);
        render(&mut terminal, &mut app);
        assert_eq!(app.scroll, 0);
        assert!(screen(&terminal).contains("[ ] word"));

        terminal.backend_mut().resize(30, 12);
        render(&mut terminal, &mut app);
        assert!(app.row_tasks.len() < total);
        assert!(screen(&terminal).contains("[ ] word"));
        assert!(!fixture.task_path().exists());
    }

    fn help_mode() -> Mode {
        Mode::Help {
            scroll: 0,
            max_scroll: 0,
            page: 0,
        }
    }

    // The scroll offset, its bound, and the page size that the last draw left in the mode.
    fn help_state(app: &App) -> (usize, usize, usize) {
        let Mode::Help {
            scroll,
            max_scroll,
            page,
        } = app.mode
        else {
            panic!("expected the keyboard reference");
        };

        (scroll, max_scroll, page)
    }

    #[test]
    fn the_reference_is_a_centered_titled_modal_that_never_mentions_page_keys() {
        for border_style in [BorderStyle::Square, BorderStyle::Rounded] {
            let mut app = app_with_texts(&["a", "b"]);
            app.border_style = border_style;
            app.mode = help_mode();
            let mut terminal = test_terminal(100, 200);

            render(&mut terminal, &mut app);
            let screen = screen(&terminal);

            let top = screen
                .lines()
                .position(|line| line.contains("Keyboard shortcuts"))
                .unwrap();
            let bottom = screen
                .lines()
                .position(|line| line.contains("? or Esc to close"))
                .unwrap();
            let top_line: Vec<char> = screen.lines().nth(top).unwrap().chars().collect();
            let (open, close) = if border_style == BorderStyle::Rounded {
                ('╭', '╮')
            } else {
                ('┌', '┐')
            };
            let left = top_line.iter().position(|c| *c == open).unwrap();
            let right = top_line.iter().rposition(|c| *c == close).unwrap();

            assert!(top.abs_diff(199 - bottom) <= 1, "vertically centered");
            assert!(
                left > 0 && left.abs_diff(99 - right) <= 1,
                "horizontally centered"
            );
            assert!(
                screen.contains("Next / previous tab    l / h, Tab / Shift+Tab  Alt+F / Alt+B")
            );
            assert!(screen.contains("Cancel                 Esc                     Ctrl+G"));

            for unlisted in ["Page", "PgUp", "PgDn", "Ctrl+V", "Alt+V"] {
                assert!(!screen.contains(unlisted), "{unlisted}");
            }

            assert_eq!(help_state(&app), (0, 0, bottom - top - 1));
        }
    }

    #[test]
    fn the_widest_side_by_side_row_fits_the_wide_minimum_width() {
        let mut app = app_with_texts(&["a"]);
        app.mode = help_mode();
        let mut terminal = test_terminal(HELP_WIDE_MIN_WIDTH, 80);

        render(&mut terminal, &mut app);
        let screen = screen(&terminal);

        for (_, rows) in HELP_GROUPS {
            for (action, keys, alternative) in rows.iter() {
                let row = format!("│ {action}  ");
                let line = screen.lines().find(|line| line.contains(&row)).unwrap();

                assert!(line.contains(keys) && line.contains(alternative), "{line}");
            }
        }
    }

    #[test]
    fn between_the_minimums_the_reference_puts_keys_under_their_actions() {
        let mut app = app_with_texts(&["a"]);
        app.mode = help_mode();
        let mut terminal = test_terminal(HELP_WIDE_MIN_WIDTH - 1, 200);

        render(&mut terminal, &mut app);
        let lines: Vec<String> = screen(&terminal).lines().map(str::to_owned).collect();
        let action = lines
            .iter()
            .position(|line| line.contains("│ Next / previous tab "))
            .unwrap();

        assert!(lines[action + 1].contains("│   l / h, Tab / Shift+Tab, Alt+F / Alt+B"));

        // Keys that don't fit on one line get a line each instead of being cut off.
        terminal.backend_mut().resize(HELP_MIN_WIDTH, 200);
        render(&mut terminal, &mut app);
        let lines: Vec<String> = screen(&terminal).lines().map(str::to_owned).collect();
        let action = lines
            .iter()
            .position(|line| line.contains("│ Next / previous tab "))
            .unwrap();

        assert!(lines[action + 1].contains("│   l / h, Tab / Shift+Tab "));
        assert!(lines[action + 2].contains("│   Alt+F / Alt+B "));
    }

    #[test]
    fn below_the_minimum_size_the_reference_asks_for_a_larger_terminal() {
        for (width, height) in [(HELP_MIN_WIDTH - 1, 30), (80, HELP_MIN_HEIGHT - 1)] {
            let mut app = app_with_texts(&["a"]);
            app.mode = Mode::Help {
                scroll: 7,
                max_scroll: 9,
                page: 4,
            };
            let mut terminal = test_terminal(width, height);

            render(&mut terminal, &mut app);
            let screen = screen(&terminal);

            assert!(!screen.contains("Keyboard shortcuts"));
            assert!(!screen.contains("[ ] a"));
            assert_eq!(
                screen.split_whitespace().collect::<Vec<_>>().join(" "),
                HELP_TOO_SMALL
            );
            assert_eq!(help_state(&app), (7, 9, 4), "kept for a later resize");
        }
    }

    #[test]
    fn resizing_keeps_the_reference_scroll_within_the_shown_layout() {
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = app_with_texts(&["a"]);
        let mut terminal = test_terminal(80, 20);
        let end = ratatui::crossterm::event::KeyCode::End;

        press_keys(&mut app, &fixture, "?");
        render(&mut terminal, &mut app);
        app.key(end.into(), &fixture.storage).unwrap();
        render(&mut terminal, &mut app);
        let (scroll, max_scroll, page) = help_state(&app);

        assert!(scroll > 0 && scroll == max_scroll);
        assert!(screen(&terminal).contains("Cancel"));

        // A taller terminal shows more, so the offset shrinks to keep the last line at the
        // bottom; a narrower one switches layout and gains lines, keeping the offset.
        terminal.backend_mut().resize(80, 40);
        render(&mut terminal, &mut app);
        let (taller_scroll, taller_max, taller_page) = help_state(&app);

        assert!(taller_page > page && taller_max < max_scroll);
        assert_eq!(taller_scroll, taller_max);

        for (width, height) in [(50, 20), (20, 20), (80, 3), (80, 200), (80, 20)] {
            terminal.backend_mut().resize(width, height);
            render(&mut terminal, &mut app);
            let (scroll, max_scroll, _) = help_state(&app);

            assert!(scroll <= max_scroll, "{width}×{height}");
        }

        assert!(screen(&terminal).contains("Keyboard shortcuts"));
    }

    #[test]
    fn closing_the_reference_redraws_the_checklist_exactly_as_before() {
        let fixture = crate::storage::tests::Fixture::new();
        let texts: Vec<String> = (0..30).map(|index| format!("Work: task {index}")).collect();
        let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
        let mut app = app_with_texts(&texts);
        let mut terminal = test_terminal(70, 12);

        for _ in 0..20 {
            press_keys(&mut app, &fixture, "j");
            render(&mut terminal, &mut app);
        }

        let before = screen(&terminal);
        let rows = render(&mut terminal, &mut app);

        press_keys(&mut app, &fixture, "?");
        render(&mut terminal, &mut app);
        press_keys(&mut app, &fixture, "j");
        let help_rows = render(&mut terminal, &mut app);
        press_keys(&mut app, &fixture, "?");
        let after_rows = render(&mut terminal, &mut app);

        assert_eq!(screen(&terminal), before);
        assert_eq!((help_rows, after_rows), (rows, rows));
        assert_eq!(app.list.selected(), Some(20));
    }

    #[test]
    fn the_help_hint_sits_in_the_bottom_right_corner_only_on_the_plain_checklist() {
        let mut app = app_with_texts(&["a", "b"]);
        let mut terminal = test_terminal(40, 8);

        let rows = render(&mut terminal, &mut app);

        assert!(row_text(&terminal, 7).ends_with("─? for help┘"));
        assert!(!screen(&terminal).contains("[0/2]"), "no task counter");

        for mode in [
            help_mode(),
            Mode::Adding("draft".into(), AddPlacement::End),
            Mode::Searching {
                input: "a".into(),
                original: Some(0),
            },
            Mode::ConfirmClear,
        ] {
            app.mode = mode;
            render(&mut terminal, &mut app);

            assert!(!screen(&terminal).contains("? for help"), "{:?}", app.mode);
        }

        app.mode = Mode::Normal;

        assert_eq!(render(&mut terminal, &mut app), rows, "no row is reserved");
    }

    #[test]
    fn the_help_hint_is_omitted_when_narrow_or_while_a_status_message_shows() {
        let mut app = app_with_texts(&["a", "b"]);
        let fits = "? for help".width() + 2;
        let mut terminal = test_terminal(fits as u16, 6);

        render(&mut terminal, &mut app);

        assert!(row_text(&terminal, 5).contains("? for help"));

        terminal.backend_mut().resize(fits as u16 - 1, 6);
        render(&mut terminal, &mut app);

        assert!(!screen(&terminal).contains("help"));

        terminal.backend_mut().resize(40, 6);
        app.status_message = Some(crate::status_message::StatusMessage::new(
            "Wrap off",
            crate::status_message::DEFAULT_DURATION,
        ));
        render(&mut terminal, &mut app);

        assert!(row_text(&terminal, 5).contains("Wrap off"));
        assert!(!screen(&terminal).contains("? for help"));
    }

    #[test]
    fn disabling_the_help_hint_hides_only_the_hint_and_question_mark_still_opens_help() {
        let fixture = crate::storage::tests::Fixture::new();
        let mut app = app_with_texts(&["a", "b"]);
        app.show_help_hint = false;
        let mut terminal = test_terminal(40, 6);

        render(&mut terminal, &mut app);

        assert!(!screen(&terminal).contains("? for help"));

        app.status_message = Some(crate::status_message::StatusMessage::new(
            "Wrap off",
            crate::status_message::DEFAULT_DURATION,
        ));
        render(&mut terminal, &mut app);

        assert!(row_text(&terminal, 5).contains("Wrap off"));

        press_keys(&mut app, &fixture, "?");
        render(&mut terminal, &mut app);

        assert!(matches!(app.mode, Mode::Help { .. }));
        assert!(screen(&terminal).contains("Keyboard shortcuts"));
    }
}
