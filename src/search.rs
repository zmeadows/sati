use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

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
