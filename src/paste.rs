// Lists pasted into the checklist. Each line becomes a task, with the bullets and checkboxes of
// common plain-text checklists (Markdown, Org, Obsidian, todo.txt, and sati's own export)
// removed and their done or carried marks kept. Markdown headings name the section of the
// items under them.

use crate::model::{
    Task, TaskStatus, input_section_name, invalid_section_name, single_line_character,
};

// The tasks a pasted `text` lists. A line's own "Name: " prefix sets its section; otherwise the
// nearest heading above it does, and then `default_section`. Blank lines, horizontal rules,
// and lines with nothing after their markers add nothing.
pub fn parse_list(text: &str, default_section: Option<&str>) -> Vec<Task> {
    let mut tasks = Vec::new();
    let mut heading_section: Option<String> = None;
    let mut after_org_headline = false;

    for raw_line in text.split(['\n', '\r', '\u{2028}', '\u{2029}']) {
        let line: String = raw_line.chars().map(single_line_character).collect();
        let line = line.trim();

        if line.is_empty() || is_horizontal_rule(line) {
            continue;
        }

        if let Some(title) = markdown_heading(line) {
            // A heading that can't be a section name ends the previous heading's section too.
            heading_section = heading_section_name(title);
            continue;
        }

        // Org files put timestamps on the lines under a headline.
        if after_org_headline && is_org_planning_line(line) {
            continue;
        }

        let item = list_item(line);
        after_org_headline = item.org_headline;

        let body = if item.markdown {
            collapse_self_titled_links(&unescape_markdown(item.body))
        } else {
            item.body.to_owned()
        };
        let body = body.trim();

        if body.is_empty() {
            continue;
        }

        let section = heading_section.as_deref().or(default_section);
        let input = match section {
            Some(section) if input_section_name(body).is_none() => format!("{section}: {body}"),
            _ => body.to_owned(),
        };

        let mut task = Task::new(input);
        task.status = item.status;
        tasks.push(task);
    }

    tasks
}

// One line of a pasted list with its list syntax taken off.
struct ListItem<'a> {
    body: &'a str,
    status: TaskStatus,
    // The line used Markdown list syntax, so backslash escapes in its body are undone.
    markdown: bool,
    org_headline: bool,
}

fn list_item(line: &str) -> ListItem<'_> {
    if let Some((body, status)) = org_todo_headline(line) {
        return ListItem {
            body,
            status,
            markdown: false,
            org_headline: true,
        };
    }

    if let Some(body) = todo_txt_completed(line) {
        return ListItem {
            body,
            status: TaskStatus::Completed,
            markdown: false,
            org_headline: false,
        };
    }

    let marker = list_marker(line);
    let after_marker = marker.unwrap_or(line);

    match checkbox(after_marker) {
        Some((body, status, bracketed)) => ListItem {
            body,
            status,
            markdown: marker.is_some() || bracketed,
            org_headline: false,
        },
        None => ListItem {
            body: after_marker,
            status: TaskStatus::Pending,
            markdown: marker.is_some(),
            org_headline: false,
        },
    }
}

// The title of a Markdown heading such as "## Groceries" or "## Groceries ##".
fn markdown_heading(line: &str) -> Option<&str> {
    let title = line.trim_start_matches('#');
    let level = line.len() - title.len();

    if !(1..=6).contains(&level) || !title.starts_with(' ') {
        return None;
    }

    Some(title.trim_end_matches('#').trim())
}

// A heading as a section name. Sections are single words, so "Weekly plan" becomes
// "Weekly-plan"; colons are dropped.
fn heading_section_name(title: &str) -> Option<String> {
    let title = unescape_markdown(title).replace(':', "");
    let name = title.split_whitespace().collect::<Vec<_>>().join("-");

    (!invalid_section_name(&name)).then_some(name)
}

// "---", "***", "___", or "- - -": a Markdown horizontal rule.
fn is_horizontal_rule(line: &str) -> bool {
    let mut marks = line.chars().filter(|character| *character != ' ');
    let Some(first) = marks.next() else {
        return false;
    };

    matches!(first, '-' | '*' | '_')
        && marks.clone().all(|mark| mark == first)
        && marks.count() >= 2
}

// "* TODO body" or "** DONE body": an Org headline with a TODO keyword.
fn org_todo_headline(line: &str) -> Option<(&str, TaskStatus)> {
    let after_stars = line.trim_start_matches('*');

    if after_stars.len() == line.len() || !after_stars.starts_with(' ') {
        return None;
    }

    let after_stars = after_stars.trim_start();

    for (keyword, status) in [
        ("TODO", TaskStatus::Pending),
        ("DONE", TaskStatus::Completed),
    ] {
        if let Some(body) = after_stars.strip_prefix(keyword)
            && (body.is_empty() || body.starts_with(' '))
        {
            return Some((body.trim_start(), status));
        }
    }

    None
}

fn is_org_planning_line(line: &str) -> bool {
    ["CLOSED:", "SCHEDULED:", "DEADLINE:"]
        .iter()
        .any(|keyword| line.starts_with(keyword))
}

// "x 2026-11-17 body" or "x 2026-11-17 2026-11-01 body": a completed todo.txt task, with its
// completion and creation dates.
fn todo_txt_completed(line: &str) -> Option<&str> {
    let after_mark = line.strip_prefix("x ")?;
    let mut body = skip_date(after_mark)?;

    if let Some(after_creation) = skip_date(body) {
        body = after_creation;
    }

    Some(body)
}

// `text` after a leading "YYYY-MM-DD" and the spaces after it.
fn skip_date(text: &str) -> Option<&str> {
    let date = text.get(..10)?;
    let is_date = date.char_indices().all(|(index, character)| match index {
        4 | 7 => character == '-',
        _ => character.is_ascii_digit(),
    });

    let rest = &text[10..];

    (is_date && (rest.is_empty() || rest.starts_with(' '))).then(|| rest.trim_start())
}

// `line` after a bullet ("- ", "* ", "+ ", "• ", ...) or number ("1. ", "1) "). Numbers have
// at most three digits, so a line starting with a year ("2026. was a year") stays whole.
fn list_marker(line: &str) -> Option<&str> {
    let ends_marker = |rest: &str| rest.is_empty() || rest.starts_with(' ');

    for bullet in ['-', '*', '+', '•', '◦', '‣', '⁃', '▪'] {
        if let Some(rest) = line.strip_prefix(bullet)
            && ends_marker(rest)
        {
            return Some(rest.trim_start());
        }
    }

    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();

    if (1..=3).contains(&digits)
        && let Some(rest) = line[digits..].strip_prefix(['.', ')'])
        && ends_marker(rest)
    {
        return Some(rest.trim_start());
    }

    None
}

// A checkbox at the start of `text`, the body after it, its status, and whether it was a
// bracketed one ("[x]") rather than a symbol ("☑"). A checkbox needs a space or the end of the
// line after it, so "[link](https://...)" stays text.
fn checkbox(text: &str) -> Option<(&str, TaskStatus, bool)> {
    let (rest, status, bracketed) = if let Some(inside) = text.strip_prefix('[') {
        // "[]" is a common way to type an empty box.
        if let Some(rest) = inside.strip_prefix(']') {
            (rest, TaskStatus::Pending, true)
        } else {
            let mark = inside.chars().next()?;
            let rest = inside[mark.len_utf8()..].strip_prefix(']')?;

            (rest, bracketed_status(mark)?, true)
        }
    } else {
        let mark = text.chars().next()?;
        let status = symbol_status(mark)?;

        (&text[mark.len_utf8()..], status, false)
    };

    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }

    Some((rest.trim_start(), status, bracketed))
}

// The marks between brackets that Markdown, Org, Obsidian, and sati itself use. Cancelled
// ("[-]") counts as done, and forwarded ("[>]", or sati's "[↪]") as carried. Others, such as a
// letter or a digit, aren't checkboxes.
#[rustfmt::skip]
fn bracketed_status(mark: char) -> Option<TaskStatus> {
    match mark {
        'x' | 'X' | '✓' | '✔' | '-'       => Some(TaskStatus::Completed),
        '>' | '↪'                         => Some(TaskStatus::Carried),
        ' ' | '/' | '!' | '?' | '*' | '<' => Some(TaskStatus::Pending),
        _                                 => None,
    }
}

#[rustfmt::skip]
fn symbol_status(mark: char) -> Option<TaskStatus> {
    match mark {
        '☐'                         => Some(TaskStatus::Pending),
        '☑' | '☒' | '✅' | '✓' | '✔' => Some(TaskStatus::Completed),
        _                           => None,
    }
}

// Undoes Markdown backslash escapes ("\_" is "_"), which `export` writes for any punctuation
// that Markdown might read as syntax.
fn unescape_markdown(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();

    while let Some(character) = characters.next() {
        if character == '\\'
            && let Some(&next) = characters.peek()
            && next.is_ascii_punctuation()
        {
            output.push(next);
            characters.next();
        } else {
            output.push(character);
        }
    }

    output
}

// `export` writes a bare URL as "[url](url)"; this makes it bare again, as it was typed.
fn collapse_self_titled_links(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(open) = rest.find('[') {
        output.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];

        if let Some(close) = after_open.find("](") {
            let title = &after_open[..close];
            let destination = &after_open[close + 2..];
            let is_url = title.starts_with("https://") || title.starts_with("http://");

            if is_url
                && let Some(after_link) = destination.strip_prefix(title)
                && let Some(after_link) = after_link.strip_prefix(')')
            {
                output.push_str(title);
                rest = after_link;
                continue;
            }
        }

        output.push('[');
        rest = after_open;
    }

    output.push_str(rest);

    output
}

#[cfg(test)]
mod tests {
    use super::parse_list;
    use crate::export;
    use crate::model::{Task, TaskStatus};

    fn parsed(text: &str) -> Vec<(String, TaskStatus)> {
        parsed_in(text, None)
    }

    fn parsed_in(text: &str, default_section: Option<&str>) -> Vec<(String, TaskStatus)> {
        parse_list(text, default_section)
            .into_iter()
            .map(|task| (task.full_text(), task.status))
            .collect()
    }

    fn pending(text: &str) -> (String, TaskStatus) {
        (text.to_owned(), TaskStatus::Pending)
    }

    fn completed(text: &str) -> (String, TaskStatus) {
        (text.to_owned(), TaskStatus::Completed)
    }

    fn carried(text: &str) -> (String, TaskStatus) {
        (text.to_owned(), TaskStatus::Carried)
    }

    #[test]
    fn plain_lines_become_pending_tasks_and_blank_lines_are_skipped() {
        assert_eq!(
            parsed("  milk \r\n\r\n\teggs\n\u{2028}bread\rjam\n"),
            vec![
                pending("milk"),
                pending("eggs"),
                pending("bread"),
                pending("jam")
            ]
        );
    }

    #[test]
    fn bullets_and_numbers_are_removed_at_any_indentation() {
        let text = "- one\n* two\n+ three\n  • four\n    ◦ five\n‣ six\n1. seven\n12) eight";

        assert_eq!(
            parsed(text),
            [
                "one", "two", "three", "four", "five", "six", "seven", "eight"
            ]
            .map(pending)
        );
    }

    #[test]
    fn checkboxes_keep_their_status() {
        #[rustfmt::skip]
        let cases = [
            ("- [ ] open",          pending("open")),
            ("- [] open",           pending("open")),
            ("* [x] done",          completed("done")),
            ("+ [X] done",          completed("done")),
            ("1. [x] done",         completed("done")),
            ("[x] done",            completed("done")),
            ("- [✓] done",          completed("done")),
            ("- [-] cancelled",     completed("cancelled")),
            ("- [>] forwarded",     carried("forwarded")),
            ("- [↪] carried",       carried("carried")),
            ("- [/] in progress",   pending("in progress")),
            ("- [!] important",     pending("important")),
            ("☐ open",              pending("open")),
            ("☑ done",              completed("done")),
            ("☒ done",              completed("done")),
            ("✔ done",              completed("done")),
            ("• ✓ done",            completed("done")),
            ("* TODO open",         pending("open")),
            ("** DONE done",        completed("done")),
            ("x 2026-11-17 done",   completed("done")),
            ("x 2026-11-17 2026-11-01 done", completed("done")),
        ];

        for (line, expected) in cases {
            assert_eq!(parsed(line), vec![expected], "line {line:?}");
        }
    }

    #[test]
    fn text_that_only_resembles_list_syntax_is_kept() {
        #[rustfmt::skip]
        let cases = [
            "[link](https://example.com) read",
            "[a] lettered",
            "-5 degrees",
            "*emphasis*",
            "x marks the spot",
            "2026. was a year",
            "#hashtag",
        ];

        for line in cases {
            assert_eq!(parsed(line), vec![pending(line)], "line {line:?}");
        }

        assert_eq!(
            parsed("* TODOS for later"),
            vec![pending("TODOS for later")]
        );
    }

    #[test]
    fn horizontal_rules_and_empty_items_add_nothing() {
        assert_eq!(
            parsed("---\n- [ ]\n* * *\n- \n___\n1.\none"),
            vec![pending("one")]
        );
    }

    #[test]
    fn headings_set_the_section_of_the_items_below_them() {
        let text = "# Groceries\n- milk\n- Home: fix sink\n## Weekly plan: soon ##\n- [x] review\n\
                    ## ???\n- loose";

        assert_eq!(
            parsed(text),
            vec![
                pending("Groceries: milk"),
                pending("Home: fix sink"),
                completed("Weekly-plan-soon: review"),
                pending("???: loose"),
            ]
        );

        // A heading that can't be a section name ends the previous one's section.
        assert_eq!(parsed("# Work\n# :\n- loose"), vec![pending("loose")]);
    }

    #[test]
    fn the_default_section_applies_to_lines_without_their_own_or_a_heading() {
        assert_eq!(
            parsed_in("- a\n- Home: b\n# Work\n- c", Some("Errands")),
            vec![
                pending("Errands: a"),
                pending("Home: b"),
                pending("Work: c")
            ]
        );
    }

    #[test]
    fn org_planning_lines_under_headlines_are_skipped() {
        let text =
            "* DONE ship\n  CLOSED: [2026-11-17 Tue 10:00]\n* TODO plan\nDEADLINE: <2026-12-01>";

        assert_eq!(parsed(text), vec![completed("ship"), pending("plan")]);
        assert_eq!(
            parsed("a\nDEADLINE: friday"),
            vec![pending("a"), pending("DEADLINE: friday")]
        );
    }

    #[test]
    fn markdown_escapes_are_undone_only_in_markdown_items() {
        assert_eq!(
            parsed("- snake\\_case \\[1\\] C:\\temp\nplain\\_text"),
            vec![pending("snake_case [1] C:\\temp"), pending("plain\\_text")]
        );
    }

    #[test]
    fn exported_markdown_pastes_back_as_the_same_tasks() {
        let mut tasks = vec![
            Task::new("Work: fix the [build] (again) #3 a_b \\ c"),
            Task::new("read [the docs](https://example.com/a_(b)) and https://example.com/x_y"),
            Task::new("Home: 1. not a number"),
        ];
        tasks[1].status = TaskStatus::Completed;

        let exported = export::markdown(&tasks, true);
        let pasted: Vec<(String, TaskStatus)> = tasks
            .iter()
            .map(|task| (task.full_text(), task.status))
            .collect();

        assert_eq!(parsed(&exported), pasted, "{exported}");
    }
}
