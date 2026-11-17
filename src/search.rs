use std::ops::Range;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use unicode_segmentation::UnicodeSegmentation;

// The query as nucleo patterns: case-insensitive fuzzy words that must all match. `Pattern::new`
// takes each word literally: no `^`, `$`, `!`, or `'` syntax.
fn fuzzy_pattern(query: &str) -> Pattern {
    Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    )
}

// Indices of the `names` that fuzzy-match `query`, best first, for the section, template, and
// task pickers. Equal scores keep the given order, and an empty query keeps every name in order.
pub fn fuzzy_rank(query: &str, names: &[&str]) -> Vec<usize> {
    if query.is_empty() {
        return (0..names.len()).collect();
    }

    let pattern = fuzzy_pattern(query);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buffer = Vec::new();
    let mut scored: Vec<(usize, u32)> = Vec::new();

    for (index, name) in names.iter().enumerate() {
        if let Some(score) = pattern.score(Utf32Str::new(name, &mut buffer), &mut matcher) {
            scored.push((index, score));
        }
    }

    // A stable sort, so equal scores stay in the given order.
    scored.sort_by_key(|&(_, score)| std::cmp::Reverse(score));

    scored.into_iter().map(|(index, _)| index).collect()
}

// The graphemes of `text` that a fuzzy match of `query` uses, as byte ranges with neighbors
// merged, for highlighting. Empty when nothing matches.
pub fn fuzzy_match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.trim().is_empty() {
        return Vec::new();
    }

    // Nucleo matches each grapheme by its first character and reports positions in this list.
    // Building it here, rather than with `Utf32Str::new`, keeps those positions grapheme
    // indices even when every grapheme starts with an ASCII character but the text is not
    // all ASCII, where `Utf32Str::new` would fall back to byte positions.
    let graphemes: Vec<(usize, &str)> = text.grapheme_indices(true).collect();
    let first_characters: Vec<char> = graphemes
        .iter()
        .map(|(_, grapheme)| grapheme.chars().next().unwrap())
        .collect();

    let pattern = fuzzy_pattern(query);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut positions = Vec::new();

    let haystack = Utf32Str::Unicode(&first_characters);
    if pattern
        .indices(haystack, &mut matcher, &mut positions)
        .is_none()
    {
        return Vec::new();
    }

    // Each word of the query reports its own positions, so they may repeat or interleave.
    positions.sort_unstable();
    positions.dedup();

    let mut ranges: Vec<Range<usize>> = Vec::new();

    for position in positions {
        let (start, grapheme) = graphemes[position as usize];
        let end = start + grapheme.len();

        match ranges.last_mut() {
            Some(previous) if previous.end == start => previous.end = end,
            _ => ranges.push(start..end),
        }
    }

    ranges
}

pub fn matches(text: &str, query: &str) -> bool {
    !query.is_empty() && text.to_lowercase().contains(&query.to_lowercase())
}

/// Map literal lowercase matches back to whole original graphemes.
pub fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let query = query.to_lowercase();

    if query.is_empty() {
        return Vec::new();
    }

    let lowercase = text.to_lowercase();
    // Lowercase the full string for contextual casing (notably Greek sigma).
    // Consume each original grapheme's lowercase scalar count to map expansions,
    // such as İ -> i + combining dot, without reusing lowercase byte offsets.
    let mut chars = lowercase.char_indices().peekable();

    let mapping: Vec<_> = text
        .grapheme_indices(true)
        .map(|(start, grapheme)| {
            let lower_start = chars.peek().map_or(lowercase.len(), |&(index, _)| index);

            for _ in grapheme.chars().flat_map(char::to_lowercase) {
                chars.next();
            }

            let lower_end = chars.peek().map_or(lowercase.len(), |&(index, _)| index);
            (lower_start..lower_end, start..start + grapheme.len())
        })
        .collect();

    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut offset = 0;
    let mut first = 0;

    while let Some(found) = lowercase[offset..].find(&query) {
        let start = offset + found;
        let end = start + query.len();

        while mapping[first].0.end <= start {
            first += 1;
        }

        let last = mapping[first..]
            .iter()
            .take_while(|(lower, _)| lower.start < end)
            .last()
            .unwrap();

        let original = mapping[first].1.start..last.1.end;

        if let Some(previous) = ranges
            .last_mut()
            .filter(|previous| previous.end >= original.start)
        {
            previous.end = previous.end.max(original.end);
        } else {
            ranges.push(original);
        }

        // Advance one scalar from the match start so overlapping occurrences count.
        offset = start + lowercase[start..].chars().next().unwrap().len_utf8();
    }

    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_rank_orders_by_score_and_keeps_ties_in_order() {
        let names = ["Packing list", "Daily", "Daily (weekend)", "daily"];

        assert_eq!(fuzzy_rank("", &names), [0, 1, 2, 3]);
        assert_eq!(fuzzy_rank("dly", &names), [1, 2, 3]);
        assert_eq!(fuzzy_rank("wknd", &names), [2]);
        assert!(fuzzy_rank("xyz", &names).is_empty());
    }

    #[test]
    fn fuzzy_match_ranges_cover_whole_graphemes_and_merge_neighbors() {
        for (text, query, expected) in [
            ("Buy milk", "bmk", vec![(0, 1), (4, 5), (7, 8)]),
            ("Buy milk", "milk", vec![(4, 8)]),
            // Every word must match, and their positions merge in text order.
            ("Buy milk", "milk buy", vec![(0, 3), (4, 8)]),
            ("Buy milk", "xyz", vec![]),
            ("Buy milk", "milk xyz", vec![]),
            ("e\u{301}x", "ex", vec![(0, 4)]),
            ("caf\u{e9} run", "cafe", vec![(0, 5)]),
            ("👩‍💻 code", "code", vec![("👩‍💻".len() + 1, "👩‍💻".len() + 5)]),
            ("anything", "", vec![]),
            ("anything", "  ", vec![]),
        ] {
            let ranges: Vec<_> = fuzzy_match_ranges(text, query)
                .into_iter()
                .map(|range| (range.start, range.end))
                .collect();

            assert_eq!(ranges, expected, "{text:?} {query:?}");
        }
    }

    #[test]
    fn lowercase_matches_map_to_original_graphemes_including_overlaps() {
        for (text, query, expected) in [
            ("Banana", "ANA", vec![(1, 6)]),
            ("foo FOO", "foo", vec![(0, 3), (4, 7)]),
            ("İxİ", "i\u{307}", vec![(0, 2), (3, 5)]),
            ("İxİ", "i", vec![(0, 2), (3, 5)]),
            ("İ", "\u{307}", vec![(0, 2)]),
            ("ΟΣ", "ΟΣ", vec![(0, 4)]),
            ("ΟΣ", "σ", vec![]),
            ("ΟΣ", "ς", vec![(2, 4)]),
            ("Straße", "STRASSE", vec![]),
            ("e\u{301}x", "e", vec![(0, 3)]),
            ("e\u{301}", "é", vec![]),
            ("👩‍💻", "💻", vec![(0, "👩‍💻".len())]),
            ("literal [.]", "[.]", vec![(8, 11)]),
            ("anything", "", vec![]),
            ("", "x", vec![]),
        ] {
            let ranges = match_ranges(text, query);
            assert_eq!(
                ranges
                    .iter()
                    .map(|range| (range.start, range.end))
                    .collect::<Vec<_>>(),
                expected,
                "{text:?} {query:?}"
            );
            assert_eq!(matches(text, query), !ranges.is_empty());

            let boundaries: Vec<_> = text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain(std::iter::once(text.len()))
                .collect();

            for range in ranges {
                assert!(boundaries.contains(&range.start));
                assert!(boundaries.contains(&range.end));
            }
        }
    }
}
