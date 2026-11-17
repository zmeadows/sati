use std::num::NonZeroU16;
use std::ops::Range;

use ratatui::{
    Frame,
    buffer::CellDiffOption,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::templates::{
    EntryInput, TemplateEditor, TemplateFocus, TemplateViewport, entry_insertion_index,
    template_matches,
};
use crate::app::{App, Mode, MoveTarget, task_noun};
use crate::config::{BorderStyle, TaskMarkerStyle};
use crate::input::TextInput;
use crate::markup::{self, DisplayText, Link};
use crate::model::{SectionView, Task, TaskStatus, current_time, format_age, input_section_name};
use crate::styles::Styles;

pub fn draw(frame: &mut Frame, app: &mut App) -> Rect {
    let area = frame.area();
    let styles = *app.appearance.styles();
    frame.render_widget(Block::default().style(styles.normal), area);

    // The templates screen replaces the checklist, and handles its own clicks.
    if matches!(app.mode, Mode::Templates(_)) {
        app.viewport.tab_targets.clear();
        draw_templates(frame, area, app);
        return Rect::default();
    }

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

    if list_area.height >= 2
        && let Some(status) = status_line(app, &styles, list_area.width)
    {
        block = block.title_bottom(status);
        showing_status = true;
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

    // The move picker types into the strip even before there are any sections to show.
    let moving = matches!(app.mode, Mode::MovingTask(_) | Mode::ConfirmNewSection(_));

    if (!tabs.is_empty() || moving) && list_area.height >= 2 {
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

    if let Mode::ConfirmNewSection(picker) = &app.mode
        && let Some(MoveTarget::New(name)) = picker.targets.get(picker.focused)
    {
        let question = format!("Move to new section {name}?");
        draw_confirmation(frame, area, &styles, app.border_style, &question);
    }

    if let Mode::ConfirmPaste(pasted) = &app.mode {
        draw_paste_confirmation(frame, area, &styles, app, &pasted.tasks);
    }

    if matches!(app.mode, Mode::Help { .. }) {
        draw_help(frame, area, app);
    }

    rows
}

// The status message for the bottom border of a box `width` cells wide, while it is showing.
// Messages may use the whole border between the corners; one too long for it gives way to its
// compact form, and is left out if even that doesn't fit.
fn status_line(app: &App, styles: &Styles, width: u16) -> Option<Line<'static>> {
    let notice = app.status_message.as_ref()?;
    let style = notice.style(std::time::Instant::now(), styles.accent, styles.normal)?;
    let available = usize::from(width).saturating_sub(2);

    let message = if notice.message().width() <= available {
        notice.message()
    } else {
        notice.compact_message()
    };

    if message.width() > available {
        return None;
    }

    Some(Line::styled(message.to_owned(), style).left_aligned())
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

// The prompt drawn before the move picker's query.
const MOVE_LABEL: &str = "move to";

// The section picker's input grows with its query up to a third of the tab strip (but at
// least this many cells), leaving the rest for matching tabs.
const PICKER_INPUT_SHARE: usize = 3;
const PICKER_INPUT_MIN_WIDTH: usize = 10;

// The narrowest shortened tab label worth showing: one cell of the name and `…`.
const MIN_TAB_LABEL_WIDTH: usize = 2;

// Draw section tabs on the box's top border (`strip`). Every tab keeps the same width
// whichever one is active, so labels never move; the active tab only changes color.
// While renaming, the active tab becomes the name input and grows with it. The section
// picker replaces [*] with its query and shows only the matching tabs, best first; the move
// picker does the same after a "move to" label, with its targets and the focused one active.
// Returns each drawn tab's clickable area.
fn draw_section_tabs(
    frame: &mut Frame,
    app: &mut App,
    tabs: &[(SectionView, String)],
    mut strip: Rect,
) -> Vec<(Rect, SectionView)> {
    let styles = *app.appearance.styles();
    let mut tabs = tabs.to_vec();

    // The picker typed into the strip, if any: its query, label, whether it has the cursor,
    // the tabs it shows, and which of them is active.
    let picker = match &app.mode {
        Mode::PickingSectionTab { input, matches, .. } => {
            let shown: Vec<(SectionView, String)> = matches
                .iter()
                .filter_map(|view| tabs.iter().find(|(tab, _)| tab == view).cloned())
                .collect();
            let active = shown.iter().position(|(view, _)| *view == app.view);

            Some((input.clone(), "", true, shown, active))
        }
        Mode::MovingTask(picker) | Mode::ConfirmNewSection(picker) => {
            let shown = move_target_tabs(&picker.targets, &tabs);
            let active = (!shown.is_empty()).then_some(picker.focused);
            let typing = matches!(app.mode, Mode::MovingTask(_));

            Some((picker.input.clone(), MOVE_LABEL, typing, shown, active))
        }
        _ => None,
    };

    let picking = picker.is_some();
    let mut picker_active = None;

    if let Some((input, label, typing, shown, active)) = picker {
        let width = usize::from(strip.width);
        let prompt = if label.is_empty() {
            "› ".to_owned()
        } else {
            format!("{label} › ")
        };
        let prompt_width = 2 + Line::raw(prompt).width() + 1;
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
                &input,
                label,
                &styles,
                typing,
                false,
                None,
            );
        }

        // The border cell after the input leads the matches and holds `‹` when they scroll.
        let used = (TAB_MARKER + input_width).min(width) as u16;
        strip = Rect::new(strip.x + used, strip.y, strip.width - used, 1);

        tabs = shown;
        picker_active = active;
    }

    // Outside the pickers the view is always one of the tabs. In the section picker it may
    // not be a match (it opened on [*], or nothing matches), and then no tab is highlighted.
    let active = if picking {
        picker_active
    } else {
        Some(
            tabs.iter()
                .position(|(view, _)| *view == app.view)
                .unwrap_or(0),
        )
    };

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

// The move picker's targets as tabs: a section's own tab label, [-], or `+Name` for a new
// section. A new section's view is only for drawing: tab clicks act in Normal mode alone, and
// the next draw replaces these targets.
fn move_target_tabs(
    targets: &[MoveTarget],
    tabs: &[(SectionView, String)],
) -> Vec<(SectionView, String)> {
    targets
        .iter()
        .map(|target| match target {
            MoveTarget::Existing {
                view: SectionView::Unsectioned,
                ..
            } => (SectionView::Unsectioned, "[-]".to_owned()),
            MoveTarget::Existing { view, name } => {
                let label = tabs
                    .iter()
                    .find(|(tab, _)| tab == view)
                    .map_or_else(|| name.clone(), |(_, label)| label.clone());

                (view.clone(), label)
            }
            MoveTarget::New(name) => (SectionView::Named(name.to_lowercase()), format!("+{name}")),
        })
        .collect()
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

// Prepared once per draw; the row mappings are moved back to App for navigation and clicks.
// Each line is one visual row, `task_index_by_row` gives the task index drawn on each (none for
// an add row), and `link_targets_by_row` the link cells on each.
struct TaskLayout {
    lines: Vec<Line<'static>>,
    task_index_by_row: Vec<Option<usize>>,
    link_targets_by_row: Vec<Vec<(Range<u16>, String)>>,
    input_row: Option<usize>,
    selected_task_index: Option<usize>,
}

fn draw_tasks(frame: &mut Frame, app: &mut App, area: Rect) {
    let layout = prepare_task_layout(app, area.width);
    let visible_rows = update_task_viewport(app, &layout, area.height);

    let TaskLayout {
        lines,
        task_index_by_row,
        link_targets_by_row,
        input_row,
        ..
    } = layout;

    app.viewport.task_index_by_row = task_index_by_row;
    app.viewport.link_targets_by_row = link_targets_by_row;
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
                .skip(visible_rows.start)
                .take(visible_rows.len())
                .collect::<Vec<_>>(),
        ),
        area,
    );
    mark_hyperlinks(
        frame,
        area,
        &app.viewport.link_targets_by_row[visible_rows.clone()],
    );
    draw_scrollbar(
        frame,
        area,
        total_rows,
        visible_rows.start,
        app.appearance.styles().muted,
    );
    draw_inline_input(frame, app, area, input_row, visible_rows);
}

// Turn each drawn link cell into an OSC 8 hyperlink, so the terminal can open it itself when
// sati leaves it the mouse. Every cell carries its own complete sequence, so redrawing any
// subset of them stays correct; `ForcedWidth` tells ratatui the cell's real width, since the
// escape sequences would otherwise count. Terminals without OSC 8 ignore the sequences.
fn mark_hyperlinks(frame: &mut Frame, area: Rect, rows: &[Vec<(Range<u16>, String)>]) {
    for (offset, targets) in rows.iter().enumerate() {
        let y = area.y + offset as u16;

        for (columns, url) in targets {
            let mut column = columns.start;

            while column < columns.end.min(area.width) {
                let cell = &mut frame.buffer_mut()[(area.x + column, y)];
                let symbol = cell.symbol();
                let width = symbol.width() as u16;

                // A wide glyph's trailing cell has no symbol of its own; it is stepped over.
                let Some(forced) = NonZeroU16::new(width) else {
                    column += 1;
                    continue;
                };

                let linked = markup::osc8_wrapped(url, symbol);
                cell.set_symbol(&linked)
                    .set_diff_option(CellDiffOption::ForcedWidth(forced));
                column += width;
            }
        }
    }
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
    let mut task_index_by_row = Vec::new();
    let mut link_targets_by_row = Vec::new();
    let mut input_row = None;
    let text_width = usize::from(width).saturating_sub(TASK_PREFIX_WIDTH);
    let now = current_time();

    for index in 0..=app.tasks.len() {
        if let Some(position) = inline
            && position == index
        {
            input_row = Some(lines.len());
            lines.push(Line::default());
            task_index_by_row.push(editing.then_some(index));
            // The input shows links as typed, so its row has no link cells.
            link_targets_by_row.push(Vec::new());

            if editing {
                continue;
            }
        }

        let Some(task) = app.tasks.get(index) else {
            break;
        };

        if !app.task_in_active_view(index) {
            continue;
        }

        // Inside a section tab every task shares the prefix, so it shows only the body.
        let in_section = matches!(app.view, SectionView::Named(_));
        let display = app.display_text(index);
        let text = display.text.as_str();

        // Draw the "Work:" part of the composed text as a bold heading; the section comes
        // from its field.
        let section_prefix = if in_section {
            None
        } else {
            task.section.as_deref()
        };

        // A section name has no colon or space, so no link starts inside "Work: ".
        let section_heading_end = section_prefix.map_or(0, |prefix| {
            let end = prefix.len();
            text.grapheme_indices(true)
                .take_while(|(index, _)| *index <= end)
                .last()
                .map_or(0, |(index, grapheme)| index + grapheme.len())
        });

        let searching = app.search_query().is_some();
        let matches = app
            .search_query()
            .map_or_else(Vec::new, |query| crate::search::match_ranges(text, query));

        let wrapped = if app.wrap && text_width > 0 {
            wrap_text(text, text_width)
        } else {
            vec![(0..text.len(), text)]
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

        // While `b` shows ages, each task's age ends its last row at the right edge, and
        // text that would run into it is cut off before a gap. Without room for at least
        // one cell of text before the age, the row shows none.
        let age = app
            .show_ages
            .then(|| format!("[{}]", format_age(task.created_at, now)))
            .filter(|age| text_width > age.width() + AGE_GAP + TRUNCATION.width());
        let last_row = wrapped.len() - 1;

        for (row, (mut source, mut row_text)) in wrapped.into_iter().enumerate() {
            let age = age.as_deref().filter(|_| row == last_row);
            let mut truncated = false;

            if let Some(age) = age {
                let room = text_width - age.width() - AGE_GAP;

                if row_text.width() > room {
                    let kept = fitting_len(row_text, room - TRUNCATION.width());
                    row_text = row_text[..kept].trim_end();
                    source.end = source.start + row_text.len();
                    truncated = true;
                }
            }

            task_index_by_row.push(Some(index));
            link_targets_by_row.push(link_columns(text, source.clone(), &display.links));

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
                row_text,
                source.start,
                section_heading_end,
                &matches,
                &display,
                text_style,
                heading_style,
                match_color,
            ));

            if let Some(age) = age {
                if truncated {
                    line.push_span(Span::styled(TRUNCATION, styles.muted));
                }

                let padding = usize::from(width).saturating_sub(line.width() + age.width());
                line.push_span(Span::raw(" ".repeat(padding)));
                line.push_span(Span::styled(age.to_owned(), styles.muted));
            }

            lines.push(line);
        }
    }

    TaskLayout {
        lines,
        task_index_by_row,
        link_targets_by_row,
        input_row,
        selected_task_index: selected,
    }
}

// The cells before task text on every row: the focus marker and the checkbox, or their blank
// indent on a wrapped row.
const TASK_PREFIX_WIDTH: usize = 6;

// The cells kept free between task text and its age, and the mark that ends text cut off to
// make room for the age.
const AGE_GAP: usize = 2;
const TRUNCATION: &str = "…";

// The length of the longest start of `text` that fits in `cells` terminal cells.
fn fitting_len(text: &str, cells: usize) -> usize {
    let mut used = 0;

    for (index, grapheme) in text.grapheme_indices(true) {
        used += grapheme.width();

        if used > cells {
            return index;
        }
    }

    text.len()
}

// The columns of each link's cells on a row drawing `source` of `text`, counted from the
// row's left edge.
fn link_columns(text: &str, source: Range<usize>, links: &[Link]) -> Vec<(Range<u16>, String)> {
    let mut targets = Vec::new();

    for link in links {
        let start = link.range.start.max(source.start);
        let end = link.range.end.min(source.end);

        if start >= end {
            continue;
        }

        let first = TASK_PREFIX_WIDTH + text[source.start..start].width();
        let last = first + text[start..end].width();
        let column = |cells: usize| u16::try_from(cells).unwrap_or(u16::MAX);

        targets.push((column(first)..column(last), link.url.clone()));
    }

    targets
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
        layout.selected_task_index.map(|selected| {
            let start = layout
                .task_index_by_row
                .iter()
                .position(|&task| task == Some(selected))
                .unwrap_or(0);

            let end = layout
                .task_index_by_row
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
    visible_rows: Range<usize>,
) {
    let inline = match &app.mode {
        Mode::Adding(input, _) => Some((input, "add")),
        Mode::Editing { input, .. } => Some((input, "edit")),
        _ => None,
    };

    if let Some((input, label)) = inline
        && let Some(row) = input_row
        && row >= visible_rows.start
        && row < visible_rows.end
    {
        let indent = area.width.saturating_sub(1).min(2);
        draw_input_line(
            frame,
            Rect::new(
                area.x + indent,
                area.y + (row - visible_rows.start) as u16,
                area.width - indent,
                1,
            ),
            input,
            label,
            app.appearance.styles(),
            true,
            true,
            app.section_suggestion_suffix().as_deref(),
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

#[allow(clippy::too_many_arguments)]
fn task_spans(
    text: &str,
    offset: usize,
    section_heading_end: usize,
    matches: &[Range<usize>],
    markup: &DisplayText,
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
        let in_heading = source < section_heading_end;

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

        if markup.bold.iter().any(|range| range.contains(&source)) {
            next = next.add_modifier(Modifier::BOLD);
        }

        if markup.italic.iter().any(|range| range.contains(&source)) {
            next = next.add_modifier(Modifier::ITALIC);
        }

        if markup.links.iter().any(|link| link.range.contains(&source)) {
            next = next.add_modifier(Modifier::UNDERLINED);
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
    suggestion_suffix: Option<&str>,
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
        input_section_name(text).map_or(0, |name| {
            let space = name.len() + 1;
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
    if let Some(suffix) = suggestion_suffix {
        let mut room = usize::from(area.width).saturating_sub(prefix_width + visible_width);
        let mut shown = String::new();

        for grapheme in suffix.graphemes(true) {
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

// Asks whether to add a pasted list as separate tasks, previewing as many of them as fit, one
// row each. When some don't fit, the last row says how many are left out.
fn draw_paste_confirmation(
    frame: &mut Frame,
    area: Rect,
    styles: &Styles,
    app: &App,
    tasks: &[Task],
) {
    // Inside the border, the preview sits between the title and a blank line, and a blank line
    // and the footer.
    const SURROUNDING_ROWS: usize = 4;

    let wanted_height = tasks.len().saturating_add(SURROUNDING_ROWS + 2);
    let width = area.width.min(60);
    let height = area
        .height
        .min(u16::try_from(wanted_height).unwrap_or(u16::MAX));

    let dialog = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );

    frame.render_widget(Clear, dialog);
    frame.render_widget(Block::default().style(styles.normal), dialog);
    let block = Block::bordered()
        .border_type(border_type(app.border_style))
        .border_style(styles.border);
    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    let title = Line::raw(format!("Add {} {}?", tasks.len(), task_noun(tasks.len()))).centered();
    let footer = Line::raw("Enter: Add  J: Join  Esc: Cancel").centered();
    let inner_height = usize::from(inner.height);

    let lines = if inner_height > SURROUNDING_ROWS {
        let room = inner_height - SURROUNDING_ROWS;
        let shown = if tasks.len() <= room {
            tasks.len()
        } else {
            room - 1
        };
        let width = usize::from(inner.width);

        let mut lines = vec![title, Line::raw("")];

        for task in &tasks[..shown] {
            lines.push(paste_preview_line(task, styles, app.task_markers, width));
        }

        if shown < tasks.len() {
            let more = format!(" … and {} more", tasks.len() - shown);
            lines.push(Line::styled(more, styles.muted));
        }

        lines.push(Line::raw(""));
        lines.push(footer);
        lines
    } else if inner_height >= 2 {
        vec![title, footer]
    } else {
        vec![title]
    };

    frame.render_widget(Paragraph::new(lines).style(styles.normal), inner);
}

// A pasted task as its checklist row would show it, cut off with "…" to fit `width` cells.
fn paste_preview_line(
    task: &Task,
    styles: &Styles,
    markers: TaskMarkerStyle,
    width: usize,
) -> Line<'static> {
    let checkbox = task_checkbox(task, markers);
    let mut text = markup::display_text(task.section.as_deref(), &task.text).text;
    let room = width.saturating_sub(checkbox.width());

    if text.width() > room {
        let kept = fitting_len(&text, room.saturating_sub(TRUNCATION.width()));
        text.truncate(kept);
        text.push_str(TRUNCATION);
    }

    let text_style = if task.status == TaskStatus::Completed {
        styles.completed.add_modifier(Modifier::CROSSED_OUT)
    } else {
        styles.normal
    };

    Line::from(vec![
        Span::styled(checkbox, styles.normal),
        Span::styled(text, text_style),
    ])
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
        ("Move task to section", "M", ""),
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
        ("Copy task text", "c", ""),
        ("Copy list as Markdown", "C", ""),
    ]),
    ("Templates", &[
        ("Open templates", "i", "Alt+I"),
        ("Add template to tab", "Space, Enter", ""),
        ("Find template", "/", ""),
        ("New template", "a", ""),
        ("Edit entries", "e", "Alt+E"),
        ("Rename template", "E", ""),
        ("Delete template", "d, Delete", ""),
        ("Close templates", "Esc, q, i", "Ctrl+G"),
    ]),
    ("Other", &[
        ("Search", "/", "Ctrl+F, Ctrl+S"),
        ("Open links in task", "v", ""),
        ("Toggle wrapping", "w", ""),
        ("Show task ages", "hold b", ""),
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

    // Leave the offset alone, so it is clamped once a usable size returns.
    if area.width < HELP_MIN_WIDTH || area.height < HELP_MIN_HEIGHT {
        draw_too_small(frame, area, &styles, HELP_TOO_SMALL);
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

// Covers `area` with a centered request for more room, for a view without a usable layout.
fn draw_too_small(frame: &mut Frame, area: Rect, styles: &Styles, message: &str) {
    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().style(styles.normal), area);

    if area.is_empty() {
        return;
    }

    let message: Vec<Line> = wrap_text(message, usize::from(area.width))
        .into_iter()
        .map(|(_, line)| Line::styled(line, styles.muted))
        .collect();
    let height = (message.len() as u16).min(area.height);
    let top = area.y + (area.height - height) / 2;

    frame.render_widget(
        Paragraph::new(message).alignment(Alignment::Center),
        Rect::new(area.x, top, area.width, height),
    );
}

const TEMPLATES_TITLE: &str = "Templates";
const TEMPLATES_LIST_HINT: &str = "␣ use · e edit · a new · / find";
const TEMPLATES_EDITOR_HINT: &str = "a add · e edit · d delete · Esc done";
const TEMPLATES_TOO_SMALL: &str = "Enlarge terminal to view templates.";

// Wide enough for the hints and a useful entry; tall enough for the borders above, between,
// and below the panels, one name row, and three entry rows.
const TEMPLATES_MIN_WIDTH: u16 = 40;
const TEMPLATES_MIN_HEIGHT: u16 = 7;

// Cells the title leaves on the top border for the find and new inputs; a target tab whose
// name would crowd them out is left out of the title.
const TEMPLATES_BORDER_INPUT_MIN_WIDTH: u16 = 12;

// Draw the templates screen in place of the checklist: matching names on top, and the focused
// template's entries (or its editor) below. Like the checklist, it records where its rows are
// for the mouse and keeps its scroll offsets in view.
fn draw_templates(frame: &mut Frame, area: Rect, app: &mut App) {
    let styles = *app.appearance.styles();
    app.viewport.templates = TemplateViewport::default();

    if area.width < TEMPLATES_MIN_WIDTH || area.height < TEMPLATES_MIN_HEIGHT {
        draw_too_small(frame, area, &styles, TEMPLATES_TOO_SMALL);
        return;
    }

    // With the tab strip hidden, the title names the tab a template will be added to. A
    // checklist without sections has no tabs, so there is nothing to name.
    let mut title = Line::from(vec![
        Span::styled("─ ", styles.border),
        Span::styled(TEMPLATES_TITLE, styles.heading),
        Span::styled(" ", styles.border),
    ]);

    if let Some((_, label)) = app
        .section_tabs()
        .into_iter()
        .find(|(view, _)| *view == app.view)
    {
        // The label is colored like the active tab it names.
        let target = [
            Span::styled("→ ", styles.border),
            Span::styled(label, styles.accent),
            Span::styled(" ", styles.border),
        ];
        // Besides the title, the top border keeps its left corner, a border cell after the
        // title, and `─┐` after the input; see the border input below.
        let title_width = title.width() + target.iter().map(Span::width).sum::<usize>();
        let border_input_room = usize::from(area.width).saturating_sub(title_width + 4);

        if border_input_room >= usize::from(TEMPLATES_BORDER_INPUT_MIN_WIDTH) {
            title.spans.extend(target);
        }
    }

    let title_width = title.width() as u16;
    let status = status_line(app, &styles, area.width);

    let App {
        mode,
        templates,
        viewport,
        wrap,
        task_markers,
        border_style,
        ..
    } = app;

    let Mode::Templates(picker) = mode else {
        return;
    };

    let matches = template_matches(templates, picker.filter.text());
    let focused = matches
        .contains(&picker.selected)
        .then_some(picker.selected);
    let editor = match &picker.focus {
        TemplateFocus::Editing(editor) => Some(editor),
        _ => None,
    };

    let entries: &[String] = focused.map_or(&[], |index| &templates[index].entries);
    let inner_width = area.width - 2;
    let layout = template_entry_layout(entries, editor, inner_width, *wrap, *task_markers, &styles);

    // Names get the rows they need, up to a third of the screen or whatever the entries
    // leave unused, and the entries get the rest. While editing, only the name shows.
    let content_rows = usize::from(area.height - 3);

    let list_rows = if editor.is_some() {
        1
    } else {
        let needed = matches.len().max(1);
        let unused_by_entries = content_rows.saturating_sub(layout.lines.len().max(1));
        let share = (content_rows / 3).max(unused_by_entries).max(1);

        needed.min(share).min(content_rows - 1)
    };

    let entry_rows = content_rows - list_rows;
    let list_area = Rect::new(area.x + 1, area.y + 1, inner_width, list_rows as u16);
    let divider_y = list_area.bottom();
    let entries_area = Rect::new(area.x + 1, divider_y + 1, inner_width, entry_rows as u16);

    let hint = match &picker.focus {
        TemplateFocus::List | TemplateFocus::Filtering => Some(TEMPLATES_LIST_HINT),
        TemplateFocus::Editing(editor) if editor.input.is_none() => Some(TEMPLATES_EDITOR_HINT),
        _ => None,
    };

    let mut block = Block::bordered()
        .border_type(border_type(*border_style))
        .border_style(styles.border)
        .title(title);

    // As on the checklist, a brief status message takes the bottom border from the hint.
    if let Some(status) = status {
        block = block.title_bottom(status);
    } else if let Some(hint) = hint
        && hint.width() + HELP_BORDERS <= usize::from(area.width)
    {
        block = block.title_bottom(Line::styled(hint, styles.muted).right_aligned());
    }

    frame.render_widget(block, area);

    // Both border styles join the divider to the sides with the same T pieces.
    let divider = format!("├{}┤", "─".repeat(usize::from(inner_width)));
    frame
        .buffer_mut()
        .set_string(area.x, divider_y, divider, styles.border);

    // The filter and a new template's name are typed on the top border, after the title. A
    // filter kept with Enter stays there, without the cursor.
    let border_input = match &picker.focus {
        TemplateFocus::Naming(input) => Some((input, "new", true)),
        TemplateFocus::Filtering => Some((&picker.filter, "find", true)),
        _ if !picker.filter.text().is_empty() => Some((&picker.filter, "find", false)),
        _ => None,
    };

    if let Some((input, label, typing)) = border_input {
        // A border cell after the title, then the input between spaces, then two border
        // cells before the corner.
        let x = area.x + 1 + title_width + 1;
        let width = area.right().saturating_sub(x + 2);

        frame
            .buffer_mut()
            .set_string(x, area.y, " ".repeat(usize::from(width)), styles.normal);

        let input_area = Rect::new(x + 1, area.y, width.saturating_sub(2), 1);
        draw_input_line(
            frame, input_area, input, label, &styles, typing, false, None,
        );
    }

    let mut template_by_row = Vec::new();

    if let Some(index) = focused.filter(|_| editor.is_some()) {
        let name = Line::from(vec![
            Span::raw(" "),
            Span::styled(
                templates[index].name.clone(),
                styles.heading.add_modifier(Modifier::BOLD),
            ),
        ]);

        frame.render_widget(Paragraph::new(name), list_area);
    } else if matches.is_empty() {
        let message = if templates.is_empty() {
            " No templates yet. Press a to create one."
        } else {
            " No matching templates"
        };

        frame.render_widget(Paragraph::new(message).style(styles.muted), list_area);
    } else {
        // Keep the focused name in view.
        let position = matches
            .iter()
            .position(|&index| Some(index) == focused)
            .unwrap_or(0);
        let mut scroll = picker
            .list_scroll
            .min(matches.len().saturating_sub(list_rows));

        if position < scroll {
            scroll = position;
        } else if position >= scroll + list_rows {
            scroll = position + 1 - list_rows;
        }

        picker.list_scroll = scroll;

        let mut lines = Vec::new();

        for &index in matches.iter().skip(scroll).take(list_rows) {
            let marker = if Some(index) == focused {
                focus_marker(*task_markers)
            } else {
                " "
            };

            lines.push(Line::from(vec![
                Span::styled(marker, styles.accent.add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::styled(templates[index].name.clone(), styles.normal),
            ]));
            template_by_row.push(index);
        }

        frame.render_widget(Paragraph::new(lines), list_area);
        draw_scrollbar(frame, list_area, matches.len(), scroll, styles.muted);

        // Renaming types over the focused name, as renaming a section types over its tab.
        if let TemplateFocus::Renaming(input) = &picker.focus
            && let Some(row) = template_by_row
                .iter()
                .position(|&index| Some(index) == focused)
        {
            let row_area = Rect::new(
                list_area.x + 2,
                list_area.y + row as u16,
                list_area.width - 2,
                1,
            );

            draw_input_line(frame, row_area, input, "rename", &styles, true, false, None);
        }
    }

    // The editor keeps its focused entry or input in view; the preview scrolls freely.
    let total_rows = layout.lines.len();
    let mut scroll = picker
        .entries_scroll
        .min(total_rows.saturating_sub(entry_rows));

    if editor.is_some()
        && let Some(focus) = layout.focus_rows.clone()
    {
        if focus.start < scroll || focus.len() > entry_rows {
            scroll = focus.start;
        } else if focus.end > scroll + entry_rows {
            scroll = focus.end - entry_rows;
        }
    }

    picker.entries_scroll = scroll;
    let visible_rows = scroll..(scroll + entry_rows).min(total_rows);

    if focused.is_some() && total_rows == 0 {
        let message = if editor.is_some() {
            " Press a to add an entry"
        } else {
            " No entries yet. Press e to add some."
        };

        frame.render_widget(Paragraph::new(message).style(styles.muted), entries_area);
    }

    let visible_lines: Vec<Line> = layout.lines[visible_rows.clone()].to_vec();
    frame.render_widget(Paragraph::new(visible_lines), entries_area);
    draw_scrollbar(frame, entries_area, total_rows, scroll, styles.muted);

    if let Some((input, kind)) = editor.and_then(|editor| editor.input.as_ref())
        && let Some(row) = layout.input_row
        && visible_rows.contains(&row)
    {
        let label = match kind {
            EntryInput::Add(_) => "add",
            EntryInput::Edit(_) => "edit",
        };
        let indent = entries_area.width.saturating_sub(1).min(2);
        let row_area = Rect::new(
            entries_area.x + indent,
            entries_area.y + (row - scroll) as u16,
            entries_area.width - indent,
            1,
        );

        draw_input_line(frame, row_area, input, label, &styles, true, false, None);
    }

    // While editing, the names can't be clicked: the editor stays on its template.
    viewport.templates = TemplateViewport {
        list: if editor.is_some() {
            Rect::default()
        } else {
            list_area
        },
        template_by_row,
        entries: entries_area,
        entry_by_row: layout.entry_by_row[visible_rows].to_vec(),
    };

    if picker.focus == TemplateFocus::ConfirmDelete
        && let Some(index) = focused
    {
        let question = format!("Delete template {}?", templates[index].name);
        draw_confirmation(frame, area, &styles, *border_style, &question);
    }
}

// The entry rows of a template, built like task rows: `entry_by_row` gives the entry on each
// row (none for an add row), and `focus_rows` the rows of the editor's selected entry or input.
struct EntryLayout {
    lines: Vec<Line<'static>>,
    entry_by_row: Vec<Option<usize>>,
    input_row: Option<usize>,
    focus_rows: Option<Range<usize>>,
}

fn template_entry_layout(
    entries: &[String],
    editor: Option<&TemplateEditor>,
    width: u16,
    wrap: bool,
    markers: TaskMarkerStyle,
    styles: &Styles,
) -> EntryLayout {
    let input = editor.and_then(|editor| editor.input.as_ref());
    let selected_entry = editor.and_then(|editor| editor.selected);

    let inline = input.map(|(_, kind)| match kind {
        EntryInput::Add(placement) => {
            entry_insertion_index(selected_entry, entries.len(), *placement)
        }
        EntryInput::Edit(index) => *index,
    });
    let editing = matches!(input, Some((_, EntryInput::Edit(_))));

    // The preview has no selection, and an open input takes the focus from it.
    let selected = if inline.is_some() {
        None
    } else {
        selected_entry
    };

    let mut lines = Vec::new();
    let mut entry_by_row = Vec::new();
    let mut input_row = None;
    let mut focus_rows = None;
    let text_width = usize::from(width).saturating_sub(TASK_PREFIX_WIDTH);

    for index in 0..=entries.len() {
        if inline == Some(index) {
            input_row = Some(lines.len());
            focus_rows = Some(lines.len()..lines.len() + 1);
            lines.push(Line::default());
            entry_by_row.push(editing.then_some(index));

            if editing {
                continue;
            }
        }

        let Some(entry) = entries.get(index) else {
            break;
        };

        let text = markup::display_text(None, entry).text;
        let wrapped = if wrap && text_width > 0 {
            wrap_text(&text, text_width)
        } else {
            vec![(0..text.len(), text.as_str())]
        };

        let is_selected = selected == Some(index);
        let first_row = lines.len();

        for (row, (_, row_text)) in wrapped.into_iter().enumerate() {
            let marker = if row == 0 && is_selected {
                focus_marker(markers)
            } else {
                " "
            };
            let checkbox = if row == 0 { " [ ] " } else { "     " };
            let checkbox_style = if is_selected {
                styles.accent
            } else {
                styles.normal
            };

            lines.push(Line::from(vec![
                Span::styled(marker, styles.accent.add_modifier(Modifier::BOLD)),
                Span::styled(checkbox, checkbox_style),
                Span::styled(row_text.to_owned(), styles.normal),
            ]));
            entry_by_row.push(Some(index));
        }

        if is_selected {
            focus_rows = Some(first_row..lines.len());
        }
    }

    EntryLayout {
        lines,
        entry_by_row,
        input_row,
        focus_rows,
    }
}

#[cfg(test)]
mod tests;
