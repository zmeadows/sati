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
