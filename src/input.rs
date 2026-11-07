use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextInput {
    text: String,
    cursor: usize,
}

impl TextInput {
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Clamp a byte offset to the preceding grapheme boundary.
    pub fn set_cursor(&mut self, offset: usize) {
        self.cursor = self
            .text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(self.text.len()))
            .take_while(|&index| index <= offset)
            .last()
            .unwrap_or(0);
    }

    pub fn insert(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.snap_forward();
    }

    pub fn move_left(&mut self) {
        if let Some((start, _)) = self.text[..self.cursor].grapheme_indices(true).next_back() {
            self.cursor = start;
        }
    }

    pub fn move_right(&mut self) {
        if let Some(grapheme) = self.text[self.cursor..].graphemes(true).next() {
            self.cursor += grapheme.len();
        }
    }

    fn previous_word_start(&self) -> usize {
        let mut start = self.cursor;
        let mut found_word = false;

        for (index, grapheme) in self.text[..self.cursor].grapheme_indices(true).rev() {
            let whitespace = grapheme.chars().all(char::is_whitespace);

            if found_word && whitespace {
                break;
            }

            start = index;
            found_word |= !whitespace;
        }

        start
    }

    pub fn move_word_left(&mut self) {
        self.cursor = self.previous_word_start();
    }

    pub fn move_word_right(&mut self) {
        let mut found_whitespace = false;
        let mut next = self.text.len();

        for (offset, grapheme) in self.text[self.cursor..].grapheme_indices(true) {
            let whitespace = grapheme.chars().all(char::is_whitespace);

            if found_whitespace && !whitespace {
                next = self.cursor + offset;
                break;
            }

            found_whitespace |= whitespace;
        }

        self.cursor = next;
    }

    pub fn delete_word_after(&mut self) {
        let start = self.cursor;
        self.move_word_right();

        // From whitespace, continue through the following word as well.
        if self.text[start..self.cursor]
            .chars()
            .all(char::is_whitespace)
        {
            self.move_word_right();
        }

        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.snap_forward();
    }

    pub fn delete_word_before(&mut self) {
        let start = self.previous_word_start();
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.snap_forward();
    }

    pub fn delete_to_end(&mut self) {
        self.text.truncate(self.cursor);
    }

    pub fn delete_to_start(&mut self) {
        self.text.drain(..self.cursor);
        self.cursor = 0;
    }

    pub fn delete_before(&mut self) {
        if let Some((start, _)) = self.text[..self.cursor].grapheme_indices(true).next_back() {
            self.set_cursor(start);
            self.delete_after();
        }
    }

    pub fn delete_after(&mut self) {
        if let Some(grapheme) = self.text[self.cursor..].graphemes(true).next() {
            self.text
                .replace_range(self.cursor..self.cursor + grapheme.len(), "");
            self.snap_forward();
        }
    }

    // Insertion/deletion can join neighboring clusters; never leave the cursor inside one.
    fn snap_forward(&mut self) {
        self.cursor = self
            .text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(self.text.len()))
            .find(|&index| index >= self.cursor)
            .unwrap();
    }
}

impl From<String> for TextInput {
    fn from(text: String) -> Self {
        let cursor = text.len();
        Self { text, cursor }
    }
}

impl From<&str> for TextInput {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_boundary(input: &TextInput) {
        assert!(input.text().is_char_boundary(input.cursor()));
        assert!(
            input
                .text()
                .grapheme_indices(true)
                .map(|(index, _)| index)
                .chain(std::iter::once(input.text().len()))
                .any(|index| index == input.cursor())
        );
    }

    #[test]
    fn forward_word_deletion_handles_whitespace_punctuation_and_joined_graphemes() {
        for (text, cursor, expected, expected_cursor) in [
            ("", 0, "", 0),
            ("one  two", 0, "two", 0),
            ("  one  two", 0, "two", 0),
            ("one,word! two", 0, "two", 0),
            ("one  two", 3, "one", 3),
            ("one  ", 3, "one", 3),
            ("one", 3, "one", 3),
            ("e\u{301}界 👩‍💻 last", 0, "👩‍💻 last", 0),
            ("🇦x 🇧", "🇦".len(), "🇦🇧", "🇦🇧".len()),
        ] {
            let mut input = TextInput::from(text);
            input.set_cursor(cursor);
            input.delete_word_after();

            assert_eq!(input.text(), expected);
            assert_eq!(input.cursor(), expected_cursor);
            assert_boundary(&input);
        }
    }

    #[test]
    fn deleting_to_input_ends_preserves_grapheme_boundaries() {
        for text in ["", "abc", "e\u{301}界👩‍💻👍🏽🇦🇧🇨 last"] {
            for offset in 0..=text.len() {
                let mut input = TextInput::from(text);
                input.set_cursor(offset);
                let cursor = input.cursor();
                let mut end = input.clone();
                end.delete_to_end();

                assert_eq!(end.text(), &text[..cursor]);
                assert_eq!(end.cursor(), cursor);
                assert_boundary(&end);

                input.delete_to_start();
                assert_eq!(input.text(), &text[cursor..]);
                assert_eq!(input.cursor(), 0);
                assert_boundary(&input);
            }
        }
    }

    #[test]
    fn cursor_navigation_clamps_and_treats_non_whitespace_as_words() {
        let text = "  e\u{301}界,👍🏽  👩‍💻\u{2003}last  ";
        let mut input = TextInput::from(text);

        let boundaries: Vec<_> = text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .collect();

        for &expected in boundaries.iter().rev().skip(1) {
            input.move_left();
            assert_eq!(input.cursor(), expected);
        }

        input.move_left();
        assert_eq!(input.cursor(), 0);

        for &expected in boundaries.iter().skip(1) {
            input.move_right();
            assert_eq!(input.cursor(), expected);
        }

        input.move_right();
        assert_eq!(input.cursor(), text.len());

        for expected in [
            "  e\u{301}界,👍🏽  👩‍💻\u{2003}".len(),
            "  e\u{301}界,👍🏽  ".len(),
            2,
            0,
            0,
        ] {
            let mut deleted = input.clone();
            deleted.delete_word_before();
            input.move_word_left();

            assert_eq!(input.cursor(), expected);
            assert_eq!(deleted.cursor(), expected);
            assert_boundary(&deleted);
        }

        for expected in [
            2,
            "  e\u{301}界,👍🏽  ".len(),
            "  e\u{301}界,👍🏽  👩‍💻\u{2003}".len(),
            text.len(),
            text.len(),
        ] {
            input.move_word_right();
            assert_eq!(input.cursor(), expected);
            assert_boundary(&input);
        }

        let mut input = TextInput::from("one  two  ");
        input.delete_word_before();
        assert_eq!(input.text(), "one  ");

        input.delete_word_before();
        assert_eq!(input.text(), "");
    }

    #[test]
    fn edits_work_at_start_middle_and_end_without_splitting_graphemes() {
        let mut input = TextInput::from("ae\u{301}👩‍💻界b");
        assert_eq!(input.cursor(), input.text().len());

        input.set_cursor("ae\u{301}".len());
        input.delete_after();
        assert_eq!(input.text(), "ae\u{301}界b");

        input.delete_before();
        assert_eq!(input.text(), "a界b");
        assert_eq!(input.cursor(), 1);

        input.insert("👍🏽");
        assert_eq!(input.text(), "a👍🏽界b");
        assert_eq!(input.cursor(), "a👍🏽".len());

        input.set_cursor(0);
        input.delete_before();
        assert_eq!(input.text(), "a👍🏽界b");

        input.delete_after();
        assert_eq!(input.text(), "👍🏽界b");

        input.set_cursor(usize::MAX);
        input.delete_after();
        assert_eq!(input.text(), "👍🏽界b");
        assert_boundary(&input);
    }

    #[test]
    fn mutations_repair_boundaries_when_adjacent_clusters_merge() {
        for (text, cursor, inserted, expected_cursor) in [
            ("eb", 1, "\u{301}", "e\u{301}".len()),
            ("👍b", "👍".len(), "🏽", "👍🏽".len()),
            ("👩💻", "👩".len(), "\u{200d}", "👩‍💻".len()),
            ("🇺🇳", 0, "🇦", "🇦🇺".len()),
        ] {
            let mut input = TextInput::from(text);
            input.set_cursor(cursor);
            input.insert(inserted);

            assert_eq!(input.cursor(), expected_cursor);
            assert_boundary(&input);
        }

        let mut input = TextInput::from("🇦x🇧");
        input.set_cursor("🇦".len());
        input.delete_after();

        assert_eq!(input.text(), "🇦🇧");
        assert_eq!(input.cursor(), input.text().len());
        assert_boundary(&input);
    }

    #[test]
    fn arbitrary_byte_offsets_and_edits_always_keep_a_valid_cursor() {
        for text in ["", "ascii", "e\u{301}界", "👩‍💻👍🏽🇺🇳", "🇦🇧🇨🇩🇪"]
        {
            for offset in 0..=text.len() + 1 {
                let mut original = TextInput::from(text);
                original.set_cursor(offset);
                assert_boundary(&original);

                for inserted in ["", "x", "\u{301}", "\u{200d}", "🏽", "🇦"] {
                    let mut input = original.clone();
                    input.insert(inserted);
                    assert_boundary(&input);

                    input.delete_before();
                    assert_boundary(&input);

                    input.delete_after();
                    assert_boundary(&input);
                }
            }
        }
    }
}
