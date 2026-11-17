// Inline Markdown inside task text. Only a few forms are recognized: links to http(s) URLs,
// either `[title](https://…)`, shown as its title, or a bare `https://…`, shown shortened; and
// emphasis with asterisks, `*italic*`, `**bold**`, and `***both***`. The raw text is what tasks
// store and edit; markup changes only how a row is shown, exported, and opened.

use std::io;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

// A shortened bare URL never takes more terminal cells than this, `…` included.
const BARE_URL_MAX_WIDTH: usize = 30;

// Punctuation that more likely ends the sentence (or emphasis) around a bare URL than the URL
// itself.
const TRAILING_PUNCTUATION: &str = ".,;:!?'\"*";

// The longest run of asterisks that marks emphasis: `***` is bold and italic at once.
const MAX_EMPHASIS_MARKER: usize = 3;

// A piece of raw task text: plain text, a link with its title (`None` for a bare URL), or the
// asterisks that open or close emphasis.
#[derive(Debug, PartialEq)]
pub enum Segment<'a> {
    Plain(&'a str),
    Link {
        title: Option<&'a str>,
        url: &'a str,
    },
    Emphasis {
        marker: &'a str,
        opening: bool,
    },
}

// A link in display text: the bytes showing it, and where it leads.
#[derive(Debug, PartialEq)]
pub struct Link {
    pub range: Range<usize>,
    pub url: String,
}

// Task text as a row shows it: each link replaced by its title or shortened URL, and emphasis
// markers removed, with the bytes they emphasized in `bold` and `italic`.
#[derive(Debug, Default, PartialEq)]
pub struct DisplayText {
    pub text: String,
    pub links: Vec<Link>,
    pub bold: Vec<Range<usize>>,
    pub italic: Vec<Range<usize>>,
}

// Split raw text into plain text, links, and emphasis markers, left to right. Anything that is
// not quite markup stays plain text.
pub fn segments(raw: &str) -> Vec<Segment<'_>> {
    let links = find_links(raw);
    let link_ranges: Vec<Range<usize>> = links.iter().map(|(range, _)| range.clone()).collect();

    let mut markup: Vec<(Range<usize>, Segment)> = links;
    for (range, opening) in emphasis_markers(raw, &link_ranges) {
        let marker = &raw[range.clone()];
        markup.push((range, Segment::Emphasis { marker, opening }));
    }
    markup.sort_by_key(|(range, _)| range.start);

    let mut segments = Vec::new();
    let mut plain_start = 0;

    for (range, segment) in markup {
        if plain_start < range.start {
            segments.push(Segment::Plain(&raw[plain_start..range.start]));
        }

        segments.push(segment);
        plain_start = range.end;
    }

    if plain_start < raw.len() {
        segments.push(Segment::Plain(&raw[plain_start..]));
    }

    segments
}

// Each link in raw text, left to right. A Markdown link wins over a bare URL in its title.
fn find_links(raw: &str) -> Vec<(Range<usize>, Segment<'_>)> {
    let mut links = Vec::new();
    let mut index = 0;

    while let Some(character) = raw[index..].chars().next() {
        let rest = &raw[index..];
        let after_word = raw[..index]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);

        let found = if character == '[' {
            markdown_link(rest).map(|(length, title, url)| (length, Some(title), url))
        } else if !after_word && let Some(length) = bare_url_length(rest) {
            Some((length, None, &rest[..length]))
        } else {
            None
        };

        let Some((length, title, url)) = found else {
            index += character.len_utf8();
            continue;
        };

        links.push((index..index + length, Segment::Link { title, url }));
        index += length;
    }

    links
}

// The runs of asterisks outside `links` that open or close emphasis, each with whether it
// opens. A run opens when text follows it and it doesn't end a word, and closes when text
// precedes it and it doesn't start a word, so `2 * 3`, `5*3`, and `*.rs` stay literal. A closer
// pairs with the nearest earlier opener of the same length, dropping any openers between them,
// so pairs always nest. Runs left unpaired, or longer than `***`, are plain text.
fn emphasis_markers(raw: &str, links: &[Range<usize>]) -> Vec<(Range<usize>, bool)> {
    let mut markers = Vec::new();
    let mut openers: Vec<Range<usize>> = Vec::new();
    let mut index = 0;

    while let Some(character) = raw[index..].chars().next() {
        if let Some(link) = links.iter().find(|link| link.start == index) {
            index = link.end;
            continue;
        }

        if character != '*' {
            index += character.len_utf8();
            continue;
        }

        let length = raw[index..].len() - raw[index..].trim_start_matches('*').len();
        let run = index..index + length;
        index = run.end;

        if length > MAX_EMPHASIS_MARKER {
            continue;
        }

        let before = raw[..run.start].chars().next_back();
        let after = raw[run.end..].chars().next();
        let can_open =
            after.is_some_and(|c| !c.is_whitespace()) && !before.is_some_and(char::is_alphanumeric);
        let can_close =
            before.is_some_and(|c| !c.is_whitespace()) && !after.is_some_and(char::is_alphanumeric);

        let opener = openers.iter().rposition(|opener| opener.len() == length);

        if can_close && let Some(position) = opener {
            markers.push((openers[position].clone(), true));
            markers.push((run, false));
            openers.truncate(position);
        } else if can_open {
            openers.push(run);
        }
    }

    markers
}

// A task's text as its row shows it. The section prefix is shown as stored, without markup, so
// it keeps its length for the heading style.
pub fn display_text(section: Option<&str>, body: &str) -> DisplayText {
    let mut display = DisplayText::default();
    let mut open_emphasis: Vec<(usize, usize)> = Vec::new();

    if let Some(section) = section {
        display.text = format!("{section}: ");
    }

    for segment in segments(body) {
        let text = &mut display.text;

        match segment {
            Segment::Plain(plain) => text.push_str(plain),
            Segment::Link { title, url } => {
                let start = text.len();

                match title {
                    Some(title) => text.push_str(title),
                    None => text.push_str(&shorten_url(url)),
                }

                display.links.push(Link {
                    range: start..text.len(),
                    url: url.to_owned(),
                });
            }
            Segment::Emphasis { marker, opening } => {
                if opening {
                    open_emphasis.push((marker.len(), text.len()));
                    continue;
                }

                // Markers come in nested pairs, so this closes the innermost open emphasis.
                let Some((length, start)) = open_emphasis.pop() else {
                    continue;
                };
                let range = start..text.len();

                match length {
                    1 => display.italic.push(range),
                    2 => display.bold.push(range),
                    _ => {
                        display.italic.push(range.clone());
                        display.bold.push(range);
                    }
                }
            }
        }
    }

    display
}

// `[title](url)` at the start of `text`: its length, trimmed title, and URL. The title can't
// contain brackets, and the URL ends at the `)` that balances its opening, so URLs with
// parentheses in them keep them.
fn markdown_link(text: &str) -> Option<(usize, &str, &str)> {
    let close = text.find(']')?;
    let title = &text[1..close];

    if title.contains('[') || title.trim().is_empty() {
        return None;
    }

    let url_start = close + 2;

    if !text[close + 1..].starts_with('(') {
        return None;
    }

    let mut depth = 0;
    let mut url_end = None;

    for (index, character) in text[url_start..].char_indices() {
        match character {
            '(' => depth += 1,
            ')' if depth == 0 => {
                url_end = Some(url_start + index);
                break;
            }
            ')' => depth -= 1,
            _ if character.is_whitespace() => return None,
            _ => {}
        }
    }

    let url_end = url_end?;
    let url = &text[url_start..url_end];
    let scheme = http_scheme_length(url)?;

    if url.len() == scheme {
        return None;
    }

    Some((url_end + 1, title.trim(), url))
}

// The length of a bare http(s) URL at the start of `text`. It runs to the next whitespace,
// without punctuation that more likely belongs to the sentence: "(see https://a.com/x)."
fn bare_url_length(text: &str) -> Option<usize> {
    http_scheme_length(text)?;

    let mut url = &text[..text.find(char::is_whitespace).unwrap_or(text.len())];

    loop {
        let unbalanced = |open: char, close: char| {
            url.ends_with(close) && url.matches(open).count() < url.matches(close).count()
        };

        if url.ends_with(|c| TRAILING_PUNCTUATION.contains(c))
            || unbalanced('(', ')')
            || unbalanced('[', ']')
        {
            url = &url[..url.len() - 1];
        } else {
            break;
        }
    }

    // A scheme with nothing after it is not a link.
    let scheme = http_scheme_length(url)?;
    (url.len() > scheme).then_some(url.len())
}

// The length of a leading `http://` or `https://`, in any letter case.
fn http_scheme_length(text: &str) -> Option<usize> {
    ["https://", "http://"].into_iter().find_map(|scheme| {
        let prefix = text.get(..scheme.len())?;
        prefix.eq_ignore_ascii_case(scheme).then_some(scheme.len())
    })
}

// A bare URL as a row shows it: the host and the start of the path, such as
// "github.com/zmeadows/sati/issu…". `…` also marks a dropped query or fragment.
fn shorten_url(url: &str) -> String {
    let rest = &url[http_scheme_length(url).unwrap_or(0)..];
    let rest = match rest.get(..4) {
        Some(www) if www.eq_ignore_ascii_case("www.") && rest.len() > 4 => &rest[4..],
        _ => rest,
    };

    let (path, dropped) = match rest.find(['?', '#']) {
        Some(0) | None => (rest, false),
        Some(end) => (&rest[..end], true),
    };

    let path = match path.strip_suffix('/') {
        Some(trimmed) if !trimmed.is_empty() => trimmed,
        _ => path,
    };

    let ellipsis = usize::from(dropped);

    if path.width() + ellipsis <= BARE_URL_MAX_WIDTH {
        let mut shown = path.to_owned();

        if dropped {
            shown.push('…');
        }

        return shown;
    }

    let mut shown = String::new();
    let mut used = 0;

    for grapheme in path.graphemes(true) {
        if used + grapheme.width() + 1 > BARE_URL_MAX_WIDTH {
            break;
        }

        used += grapheme.width();
        shown.push_str(grapheme);
    }

    shown.push('…');
    shown
}

// `symbol` as an OSC 8 hyperlink to `url`. OSC 8 URIs must be ASCII, so other bytes are
// percent-encoded; links never contain control characters, which task text can't hold.
pub fn osc8_wrapped(url: &str, symbol: &str) -> String {
    let mut encoded = String::new();

    for byte in url.bytes() {
        if byte.is_ascii() {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }

    format!("\x1b]8;;{encoded}\x1b\\{symbol}\x1b]8;;\x1b\\")
}

// Open `url` with the system's handler for it, usually the default browser. Success means
// only that the opener started. The URL is a single argument, never passed through a shell.
#[cfg(not(test))]
pub fn open(url: &str) -> io::Result<()> {
    use std::process::{Command, Stdio};

    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");

    // `explorer` splits arguments at commas, and `cmd /c start` treats `&` as a separator.
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    };

    #[cfg(not(any(target_os = "macos", windows)))]
    let mut command = Command::new("xdg-open");

    let mut child = command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    // Nothing waits for the opener, so reap it when it exits rather than leaving a zombie.
    std::thread::spawn(move || child.wait());

    Ok(())
}

// Tests record opened URLs instead of launching a browser.
#[cfg(test)]
pub fn open(url: &str) -> io::Result<()> {
    tests::OPENED.with(|opened| opened.borrow_mut().push(url.to_owned()));
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;

    use super::*;

    thread_local! {
        pub(crate) static OPENED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    // The URLs `open` was given on this thread since the last call.
    pub(crate) fn take_opened() -> Vec<String> {
        OPENED.with(|opened| opened.take())
    }

    // The display text and each link's shown text and URL.
    fn shown(raw: &str) -> (String, Vec<(String, String)>) {
        let display = display_text(None, raw);
        let links = display
            .links
            .iter()
            .map(|link| {
                (
                    display.text[link.range.clone()].to_owned(),
                    link.url.clone(),
                )
            })
            .collect();

        (display.text, links)
    }

    // Raw text, its display text, and each link's shown text and URL.
    type Case<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

    #[track_caller]
    fn assert_shown(cases: &[Case]) {
        for (raw, text, links) in cases {
            let expected: Vec<(String, String)> = links
                .iter()
                .map(|(shown, url)| ((*shown).to_owned(), (*url).to_owned()))
                .collect();

            assert_eq!(shown(raw), ((*text).to_owned(), expected), "{raw:?}");
        }
    }

    #[test]
    fn markdown_links_show_their_titles() {
        #[rustfmt::skip]
        let cases: &[Case] = &[
            ("Read [the docs](https://ratatui.rs/) today", "Read the docs today", &[("the docs", "https://ratatui.rs/")]),
            ("[ spaced ](http://a.com)", "spaced", &[("spaced", "http://a.com")]),
            ("[Ünïcode 日本](https://例え.jp/パス)", "Ünïcode 日本", &[("Ünïcode 日本", "https://例え.jp/パス")]),
            ("[wiki](https://en.wikipedia.org/wiki/Rust_(language)) end", "wiki end", &[("wiki", "https://en.wikipedia.org/wiki/Rust_(language)")]),
            ("[a](HTTPS://A.COM)", "a", &[("a", "HTTPS://A.COM")]),
            ("[a](https://a.com)[b](https://b.com)", "ab", &[("a", "https://a.com"), ("b", "https://b.com")]),
            ("[see https://x.com](https://y.com)", "see https://x.com", &[("see https://x.com", "https://y.com")]),
            ("[[nested]](https://a.com)", "[[nested]](a.com)", &[("a.com", "https://a.com")]),
            ("[a] (https://a.com)", "[a] (a.com)", &[("a.com", "https://a.com")]),
            ("[a](file:///etc/passwd)", "[a](file:///etc/passwd)", &[]),
            ("[a](relative/path)", "[a](relative/path)", &[]),
            ("[a](https://)", "[a](https://)", &[]),
            ("[ ](https://a.com)", "[ ](a.com)", &[("a.com", "https://a.com")]),
            ("[a](https://a.com/x y)", "[a](a.com/x y)", &[("a.com/x", "https://a.com/x")]),
            ("[a](https://a.com", "[a](a.com", &[("a.com", "https://a.com")]),
            ("plain [brackets] and (parens)", "plain [brackets] and (parens)", &[]),
        ];

        assert_shown(cases);
    }

    #[test]
    fn bare_urls_are_found_at_word_starts_without_trailing_punctuation() {
        #[rustfmt::skip]
        let cases: &[Case] = &[
            ("https://example.com", "example.com", &[("example.com", "https://example.com")]),
            ("see https://a.com/x.", "see a.com/x.", &[("a.com/x", "https://a.com/x")]),
            ("(see https://a.com/x)", "(see a.com/x)", &[("a.com/x", "https://a.com/x")]),
            ("(https://a.com/x_(y))", "(a.com/x_(y))", &[("a.com/x_(y)", "https://a.com/x_(y)")]),
            ("\"https://a.com\", then", "\"a.com\", then", &[("a.com", "https://a.com")]),
            ("http://a.com?! ok", "a.com?! ok", &[("a.com", "http://a.com")]),
            ("xhttps://a.com", "xhttps://a.com", &[]),
            ("https://", "https://", &[]),
            ("https://.", "https://.", &[]),
            ("ftp://a.com", "ftp://a.com", &[]),
            ("Work: https://a.com", "Work: a.com", &[("a.com", "https://a.com")]),
            ("10:30 call", "10:30 call", &[]),
        ];

        assert_shown(cases);
    }

    #[test]
    fn bare_urls_shorten_to_host_and_path() {
        #[rustfmt::skip]
        let cases = [
            ("https://www.example.com/", "example.com"),
            ("https://www.", "www."),
            ("http://example.com/search?q=rust", "example.com/search…"),
            ("https://example.com/#top", "example.com…"),
            ("https://example.com?q", "example.com…"),
            ("https://github.com/zmeadows/sati/issues/42?tab=x", "github.com/zmeadows/sati/issu…"),
            ("https://例え.jp/ずっと長いパスの名前です", "例え.jp/ずっと長いパスの名前…"),
        ];

        for (url, expected) in cases {
            let shortened = shorten_url(url);
            assert_eq!(shortened, expected, "{url:?}");
            assert!(shortened.width() <= BARE_URL_MAX_WIDTH);
        }
    }

    #[test]
    fn osc8_percent_encodes_non_ascii() {
        assert_eq!(
            osc8_wrapped("https://例.jp/a?b=c;d", "x"),
            "\x1b]8;;https://%E4%BE%8B.jp/a?b=c;d\x1b\\x\x1b]8;;\x1b\\"
        );
    }

    // The display text and the shown text of each bold and each italic range.
    fn emphasized(raw: &str) -> (String, Vec<String>, Vec<String>) {
        let display = display_text(None, raw);
        let shown = |ranges: &[Range<usize>]| -> Vec<String> {
            ranges
                .iter()
                .map(|range| display.text[range.clone()].to_owned())
                .collect()
        };

        (
            display.text.clone(),
            shown(&display.bold),
            shown(&display.italic),
        )
    }

    // Raw text, its display text, and its bold and italic parts.
    type EmphasisCase<'a> = (&'a str, &'a str, &'a [&'a str], &'a [&'a str]);

    #[track_caller]
    fn assert_emphasized(cases: &[EmphasisCase]) {
        for (raw, text, bold, italic) in cases {
            let owned = |parts: &[&str]| -> Vec<String> {
                parts.iter().map(|part| (*part).to_owned()).collect()
            };

            assert_eq!(
                emphasized(raw),
                ((*text).to_owned(), owned(bold), owned(italic)),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn asterisks_mark_emphasis_only_when_they_pair_around_text() {
        #[rustfmt::skip]
        let cases: &[EmphasisCase] = &[
            ("call *Bob* today", "call Bob today", &[], &["Bob"]),
            ("**urgent**: pay rent", "urgent: pay rent", &["urgent"], &[]),
            ("***both*** and more", "both and more", &["both"], &["both"]),
            ("**bold *nested* bold**", "bold nested bold", &["bold nested bold"], &["nested"]),
            ("*a **b* c**", "a **b c**", &[], &["a **b"]),
            ("*a *b*", "*a b", &[], &["b"]),
            ("*日本語* 👩‍💻", "日本語 👩‍💻", &[], &["日本語"]),
            ("(*aside*).", "(aside).", &[], &["aside"]),
            ("2 * 3 * 4", "2 * 3 * 4", &[], &[]),
            ("5*3 = 15 and 2*4", "5*3 = 15 and 2*4", &[], &[]),
            ("rm *.rs and *.toml", "rm *.rs and *.toml", &[], &[]),
            ("* not a list *", "* not a list *", &[], &[]),
            ("**unclosed *mismatched", "**unclosed *mismatched", &[], &[]),
            ("**stray * inside**", "stray * inside", &["stray * inside"], &[]),
            ("****four****", "****four****", &[], &[]),
            ("lone * and ** and ***", "lone * and ** and ***", &[], &[]),
        ];

        assert_emphasized(cases);
    }

    #[test]
    fn emphasis_wraps_links_but_not_their_insides() {
        #[rustfmt::skip]
        let cases: &[EmphasisCase] = &[
            ("**[docs](https://a.com)**", "docs", &["docs"], &[]),
            ("*see https://a.com/x*", "see a.com/x", &[], &["see a.com/x"]),
            ("[*title*](https://a.com)", "*title*", &[], &[]),
            ("*a [b](https://a.com/*x*) c*", "a b c", &[], &["a b c"]),
            ("https://a.com/*x*", "a.com/*x*", &[], &[]),
        ];

        assert_emphasized(cases);

        assert_eq!(
            shown("**[docs](https://a.com)**").1,
            [("docs".to_owned(), "https://a.com".to_owned())]
        );
    }

    #[test]
    fn a_section_prefix_is_shown_literally() {
        let display = display_text(Some("*Work*"), "**fix** *it*");

        assert_eq!(display.text, "*Work*: fix it");
        assert_eq!(display.bold, std::slice::from_ref(&(8..11)));
        assert_eq!(display.italic, std::slice::from_ref(&(12..14)));
    }

    #[test]
    fn segments_keep_plain_text_and_raw_links() {
        assert_eq!(
            segments("a [b](https://b.com) https://c.com."),
            [
                Segment::Plain("a "),
                Segment::Link {
                    title: Some("b"),
                    url: "https://b.com"
                },
                Segment::Plain(" "),
                Segment::Link {
                    title: None,
                    url: "https://c.com"
                },
                Segment::Plain("."),
            ]
        );
    }
}
