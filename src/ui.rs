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
use crate::config::{BorderStyle, TaskMarkerStyle};
use crate::input::TextInput;
use crate::model::{SectionView, Task, TaskStatus, task_prefix};
use crate::styles::Styles;

pub fn draw(frame: &mut Frame, app: &mut App) -> Rect {
    let area = frame.area();
    let styles = *app.appearance.styles();
    frame.render_widget(Block::default().style(styles.normal), area);
    let searching = matches!(app.mode, Mode::Searching { .. });
    // Only search uses a separate input row; add/edit live inside the checklist.
    let input_height = u16::from(searching && area.height > 0);
    let sections =
        Layout::vertical([Constraint::Min(0), Constraint::Length(input_height)]).split(area);

    let tabs = app.section_tabs();
    app.viewport.tab_targets.clear();
    let list_area = sections[0];

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type(app.border_style))
        .border_style(styles.border);

    let mut showing_status = false;

    if let Some(notice) = &app.status_message
        && let Some(status_style) =
            notice.style(std::time::Instant::now(), styles.accent, styles.normal)
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
        block = block.title_bottom(Line::styled(HELP_HINT, styles.muted).right_aligned());
    }

    let mut rows = block.inner(list_area);
    frame.render_widget(block, list_area);

    if !tabs.is_empty() && list_area.height >= 2 {
        // Tabs sit on the top border between the corners.
        let strip_width = list_area.width.saturating_sub(2);
        let strip = Rect::new(list_area.x + 1, list_area.y, strip_width, 1);
        app.viewport.tab_targets = draw_section_tabs(frame, app, &tabs, strip);

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
                for (target, view) in &app.viewport.tab_targets {
                    let Some(position) = tabs.iter().position(|(tab, _)| tab == view) else {
                        continue;
                    };

                    if position >= JUMPABLE_TABS {
                        continue;
                    }

                    let bracketed = matches!(view, SectionView::All | SectionView::Unsectioned);
                    let x = target.x + 1 + u16::from(bracketed);
                    let digit = (position + 1) % JUMPABLE_TABS;

                    frame
                        .buffer_mut()
                        .set_string(x, blank_row, digit.to_string(), styles.accent);
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

        draw_input_line(frame, sections[1], input, label, &styles, true, false, None);

        // The hint sits at the right edge only while the prompt, query, and cursor cell
        // leave room for it, so it never covers typing.
        let prompt_width = label.width() + " › ".width();
        let typed_width = prompt_width + input.text().width() + 1;
        let needed_width = typed_width + SEARCH_HINT_GAP + SEARCH_HINT.width();

        if needed_width <= usize::from(sections[1].width) {
            frame.render_widget(
                Paragraph::new(Line::styled(SEARCH_HINT, styles.muted).right_aligned()),
                sections[1],
            );
        }
    }

    if app.mode == Mode::ConfirmClearNonCarried {
        // Without "tasks", even "Clear unsectioned except carried?" fits the 40-column dialog.
        let scope = app.scope_label().unwrap_or_else(|| "all".to_owned());
        let question = format!("Clear {scope} except carried?");
        draw_confirmation(frame, area, &styles, app.border_style, &question);
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
    let styles = *app.appearance.styles();
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
                .set_string(x, strip.y, " ".repeat(input_width), styles.normal);

            draw_input_line(
                frame,
                Rect::new(x + 1, strip.y, (input_width as u16).saturating_sub(2), 1),
                input,
                "",
                &styles,
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
    let saved_scroll = if picking { 0 } else { app.viewport.tab_scroll };
    let mut start = saved_scroll.min(target);

    while start < target && fitting_tabs(&widths, start, width) <= target {
        start += 1;
    }

    if !picking {
        app.viewport.tab_scroll = start;
    }

    let end = fitting_tabs(&widths, start, width);

    let y = strip.y;
    let mut targets = Vec::new();

    if start > 0 {
        frame.buffer_mut().set_string(strip.x, y, "‹", styles.muted);
    }

    let mut x = strip.x + TAB_MARKER as u16;

    for (index, (view, label)) in tabs.iter().enumerate().take(end).skip(start) {
        let tab_width = widths[index] as u16;

        let style = if Some(index) == active {
            styles.accent
        } else {
            styles.muted
        };

        if Some(index) == active
            && let Some(input) = rename_input
        {
            // A name that can't be used is drawn struck through; the notice says why.
            let mut input_styles = styles;

            if app.rename_problem().is_some() {
                input_styles.normal = styles.muted.add_modifier(Modifier::CROSSED_OUT);
            }

            // Blank the whole tab first so the border doesn't show under the cursor cell.
            frame
                .buffer_mut()
                .set_string(x, y, " ".repeat(usize::from(tab_width)), styles.normal);

            draw_input_line(
                frame,
                Rect::new(x + 1, y, tab_width.saturating_sub(2), 1),
                input,
                RENAME_LABEL,
                &input_styles,
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
        frame.buffer_mut().set_string(x - 1, y, "›", styles.muted);
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
    let layout = prepare_task_layout(app, area.width);
    let visible = update_task_viewport(app, &layout, area.height);

    let TaskLayout {
        lines,
        row_tasks,
        input_row,
        ..
    } = layout;

    app.viewport.row_tasks = row_tasks;
    let total_rows = lines.len();

    if lines.is_empty() {
        frame.render_widget(
            Paragraph::new(" Press a to add a task").style(app.appearance.styles().muted),
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
    draw_scrollbar(
        frame,
        area,
        total_rows,
        visible.start,
        app.appearance.styles().muted,
    );
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

fn prepare_task_layout(app: &App, width: u16) -> TaskLayout {
    let styles = app.appearance.styles();
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
    let mut row_tasks = Vec::new();
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

        // Draw the "Work:" part of the composed text as a bold heading; the section comes
        // from its field.
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

        // A completed task is struck through in one color, section prefix included.
        let (text_style, heading_style) = if completed {
            let style = styles.completed.add_modifier(Modifier::CROSSED_OUT);
            (style, style)
        } else {
            (styles.normal, styles.heading)
        };

        // Matches are reversed, so the focused result's accent becomes their background and
        // the theme's background becomes their text, legible in light and dark themes alike.
        let match_color = if selected == Some(index) {
            styles.accent.fg.unwrap_or(Color::Reset)
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
                task_checkbox(task, app.task_markers)
            };

            let checkbox_style = if searching {
                if matches.is_empty() {
                    styles.muted
                } else {
                    styles.accent
                }
            } else if selected == Some(index) {
                styles.accent
            } else if completed {
                styles.muted
            } else {
                styles.normal
            };

            let mut line = Line::from(vec![Span::styled(
                if row == 0 && selected == Some(index) {
                    focus_marker(app.task_markers)
                } else {
                    " "
                },
                styles.accent.add_modifier(Modifier::BOLD),
            )]);

            // Only the arrow of a carried checkbox takes the carryover-marker color, so its
            // brackets still show selection like any other checkbox. Search markers replace it.
            let carried_arrow = if row == 0 && !searching && task.status == TaskStatus::Carried {
                prefix
                    .strip_prefix(" [")
                    .and_then(|rest| rest.strip_suffix("] "))
            } else {
                None
            };

            if let Some(arrow) = carried_arrow {
                line.push_span(Span::styled(" [", checkbox_style));
                line.push_span(Span::styled(arrow, styles.carryover_marker));
                line.push_span(Span::styled("] ", checkbox_style));
            } else {
                line.push_span(Span::styled(prefix, checkbox_style));
            }

            line.spans.extend(task_spans(
                text,
                source.start,
                tag_end,
                &matches,
                text_style,
                heading_style,
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
    app.viewport.height = height;

    if layout.lines.is_empty() {
        app.viewport.scroll = 0;
        return 0..0;
    }

    let height = usize::from(height);
    app.viewport.scroll = app
        .viewport
        .scroll
        .min(layout.lines.len().saturating_sub(height));

    if height == 0 {
        return app.viewport.scroll..app.viewport.scroll;
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
        && (app.viewport.reveal_selection
            || end <= app.viewport.scroll
            || start >= app.viewport.scroll + height)
    {
        if end - start > height || start < app.viewport.scroll {
            app.viewport.scroll = start;
        } else if end > app.viewport.scroll + height {
            app.viewport.scroll = end - height;
        }
    }

    app.viewport.reveal_selection = false;

    app.viewport.scroll..(app.viewport.scroll + height).min(layout.lines.len())
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
            app.appearance.styles(),
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

fn focus_marker(style: TaskMarkerStyle) -> &'static str {
    match style {
        TaskMarkerStyle::Unicode => "›",
        TaskMarkerStyle::Ascii => ">",
    }
}

fn task_checkbox(task: &Task, style: TaskMarkerStyle) -> &'static str {
    match (task.status, style) {
        (TaskStatus::Pending, _) => " [ ] ",
        (TaskStatus::Completed, TaskMarkerStyle::Unicode) => " [✓] ",
        (TaskStatus::Completed, TaskMarkerStyle::Ascii) => " [x] ",
        (TaskStatus::Carried, TaskMarkerStyle::Unicode) => " [↪] ",
        (TaskStatus::Carried, TaskMarkerStyle::Ascii) => " [>] ",
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
    heading: Style,
    match_color: Color,
) -> Vec<Span<'static>> {
    // Matches derive from the base style rather than the heading, so a match in the section
    // prefix looks like any other match.
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

        let in_match = matches
            .get(matching)
            .is_some_and(|range| range.contains(&source));
        let in_heading = source < tag_end;

        let mut next = if in_match {
            highlight
        } else if in_heading {
            heading
        } else {
            base
        };

        if in_heading {
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

    // A task input draws a complete "Work: " prefix as a bold heading, as task rows draw their
    // section. The prefix ends at a grapheme boundary so a mark joined to its space stays
    // with it.
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
        styles.heading.add_modifier(Modifier::BOLD),
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
#[rustfmt::skip]
const HELP_GROUPS: &[(&str, &[HelpRow])] = &[
    ("Tasks", &[
        ("Next / previous task", "j / k, ↓ / ↑", "Ctrl+N / Ctrl+P"),
        ("First / last task", "g / G, Home / End", ""),
        ("Half screen down / up", "", "Ctrl+D / Ctrl+U"),
        ("Repeat a move", "count first, e.g. 5j", ""),
    ]),
    ("Sections", &[
        ("Next / previous tab", "l / h, Tab / Shift+Tab", "Alt+F / Alt+B"),
        ("Go to tab 1–10", "s1 … s9, s0", "Alt+S, digit"),
        ("Find tab by name", "`", "Alt+G"),
        ("Back to last position", "'", ""),
        ("Move tab right / left", "L / H", "Alt+N / Alt+P"),
        ("Rename section", "E", ""),
    ]),
    ("Changing tasks", &[
        ("Check / uncheck", "Space, Enter", ""),
        ("Append task", "a", ""),
        ("Add task below", "o", "Ctrl+O"),
        ("Add task above", "O", ""),
        ("Edit task", "e", "Alt+E"),
        ("Move task down / up", "J / K", "Alt+↓ / Alt+↑"),
        ("Toggle carryover", "m", ""),
        ("Delete task", "d, Delete", ""),
        ("Clear completed", "X", ""),
        ("Clear except carried", "D", ""),
    ]),
    ("Copy and undo", &[
        ("Copy task", "y", "Alt+W"),
        ("Paste below / above", "p / P", "Ctrl+Y (below)"),
        ("Undo", "u", "Ctrl+Z"),
        ("Redo", "", "Ctrl+R"),
        ("Copy list as Markdown", "c", ""),
    ]),
    ("Other", &[
        ("Search", "/", "Ctrl+F, Ctrl+S"),
        ("Toggle wrapping", "w", ""),
        ("Next / previous theme", "t / T", ""),
        ("Keyboard shortcuts", "?", ""),
        ("Quit", "q", ""),
    ]),
    ("While typing", &[
        ("Move cursor", "← / →", "Ctrl+B / Ctrl+F"),
        ("Start / end of text", "Home / End", "Ctrl+A / Ctrl+E"),
        ("Previous / next word", "Ctrl+← / Ctrl+→", "Alt+B / Alt+F"),
        ("Delete previous word", "Ctrl/Alt+Backspace", "Ctrl+W"),
        ("Delete next word", "", "Alt+D"),
        ("Delete character", "Delete", "Ctrl+D"),
        ("Delete to end / start", "", "Ctrl+K / Ctrl+U"),
        ("Next / previous match", "Tab / Shift+Tab", "Ctrl+N / Ctrl+P"),
        ("Confirm", "Enter", ""),
        ("Cancel", "Esc", "Ctrl+G"),
    ]),
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
    let styles = *app.appearance.styles();

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

        lines.push(Line::styled(format!("{padding}{heading}"), styles.heading));

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
            Span::styled(HELP_TITLE, styles.heading),
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
mod tests;
