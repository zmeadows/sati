// Links inside task text. Only two forms are recognized, and only for http(s) URLs:
// `[title](https://…)`, shown as its title, and a bare `https://…`, shown shortened. The raw
// text is what tasks store and edit; links change only how a row is shown, exported, and opened.

use std::io;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

// A shortened bare URL never takes more terminal cells than this, `…` included.
const BARE_URL_MAX_WIDTH: usize = 30;

// Punctuation that more likely ends the sentence around a bare URL than the URL itself.
const TRAILING_PUNCTUATION: &str = ".,;:!?'\"";

// A piece of raw task text: plain text, or a link with its title (`None` for a bare URL).
#[derive(Debug, PartialEq)]
pub enum Segment<'a> {
    Plain(&'a str),
    Link {
        title: Option<&'a str>,
        url: &'a str,
    },
}

// A link in display text: the bytes showing it, and where it leads.
#[derive(Debug, PartialEq)]
pub struct Link {
    pub range: Range<usize>,
    pub url: String,
}

// Task text as a row shows it, with each link replaced by its title or shortened URL.
#[derive(Debug, PartialEq)]
pub struct DisplayText {
    pub text: String,
    pub links: Vec<Link>,
}

// Split raw text into plain text and links, left to right. A Markdown link wins over a bare
// URL in its title, and anything that is not quite a link stays plain text.
pub fn segments(raw: &str) -> Vec<Segment<'_>> {
    let mut segments = Vec::new();
    let mut plain_start = 0;
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

        if plain_start < index {
            segments.push(Segment::Plain(&raw[plain_start..index]));
        }

        segments.push(Segment::Link { title, url });
        index += length;
        plain_start = index;
    }

    if plain_start < raw.len() {
        segments.push(Segment::Plain(&raw[plain_start..]));
    }

    segments
}

pub fn display_text(raw: &str) -> DisplayText {
    let mut text = String::new();
    let mut links = Vec::new();

    for segment in segments(raw) {
        match segment {
            Segment::Plain(plain) => text.push_str(plain),
            Segment::Link { title, url } => {
                let start = text.len();

                match title {
                    Some(title) => text.push_str(title),
                    None => text.push_str(&shorten_url(url)),
                }

                links.push(Link {
                    range: start..text.len(),
                    url: url.to_owned(),
                });
            }
        }
    }

    DisplayText { text, links }
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
        let display = display_text(raw);
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
