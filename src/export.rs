use std::{
    fs,
    io::{self, Write},
    path::Path,
};

use crate::{
    app::{Task, TaskStatus},
    storage::{Result, Storage},
};

pub fn markdown(tasks: &[Task]) -> String {
    let mut output = String::new();

    for task in tasks {
        // Markdown has no carryover checkbox, so carried tasks export as unfinished.
        let checkbox = match task.status {
            TaskStatus::Completed => "- [x] ",
            TaskStatus::Pending | TaskStatus::Carried => "- [ ] ",
        };

        output.push_str(checkbox);

        for character in task.display_text().chars() {
            if "\\`*_{}[]<>!&#|~+-.()".contains(character) {
                output.push('\\');
            }
            output.push(character);
        }

        output.push('\n');
    }

    output
}

pub fn run(data_dir: Option<&Path>) -> Result<()> {
    // A missing default directory just means sati has not saved anything yet, but a missing
    // `--data-dir` is more likely a typo, so it is reported instead of exporting nothing.
    // Export never creates it either way.
    if let Some(directory) = data_dir {
        let metadata = fs::metadata(directory).map_err(|error| {
            format!(
                "could not read data directory {}: {error}",
                directory.display()
            )
        })?;

        if !metadata.is_dir() {
            return Err(
                format!("data directory {} is not a directory", directory.display()).into(),
            );
        }
    }

    let storage = Storage::discover(data_dir)?;
    // Atomic task-file replacement allows a consistent read without the writer lock.
    let contents = markdown(&storage.load()?);

    let mut stdout = io::stdout().lock();
    let written = stdout
        .write_all(contents.as_bytes())
        .and_then(|()| stdout.flush());

    // A reader that stops early (`sati export | head`) is not an export failure.
    match written {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => Ok(result?),
    }
}

pub fn copy(tasks: &[Task], output: &mut impl Write) -> io::Result<()> {
    const BASE64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let contents = markdown(tasks);
    let mut sequence = String::from("\x1b]52;c;");

    for chunk in contents.as_bytes().chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        sequence.push(BASE64[usize::from(a >> 2)] as char);
        sequence.push(BASE64[usize::from((a & 3) << 4 | b >> 4)] as char);
        sequence.push(if chunk.len() > 1 {
            BASE64[usize::from((b & 15) << 2 | c >> 6)] as char
        } else {
            '='
        });
        sequence.push(if chunk.len() > 2 {
            BASE64[usize::from(c & 63)] as char
        } else {
            '='
        });
    }

    sequence.push('\x07');

    if let Err(error) = output.write_all(sequence.as_bytes()) {
        // Best effort: terminate any partial OSC before ordinary rendering resumes.
        let _ = output.write_all(b"\x07");
        let _ = output.flush();
        return Err(error);
    }

    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_is_literal_ordered_unicode_and_independent_of_carryover() {
        let tasks = [
            Task::new("Buy groceries"),
            Task {
                status: TaskStatus::Completed,
                ..Task::new("Walk the dog")
            },
            Task {
                status: TaskStatus::Carried,
                ..Task::new("Work: Review *PR* & <notes> [x] \\ `code`")
            },
            Task::new("1. nested 日本語 👩‍💻 e\u{301}"),
        ];

        assert_eq!(
            markdown(&tasks),
            concat!(
                "- [ ] Buy groceries\n",
                "- [x] Walk the dog\n",
                r"- [ ] Work: Review \*PR\* \& \<notes\> \[x\] \\ \`code\`",
                "\n",
                "- [ ] 1\\. nested 日本語 👩‍💻 e\u{301}\n"
            )
        );
        assert_eq!(markdown(&[]), "");
    }

    #[test]
    fn clipboard_uses_standard_target_base64_and_flushes() {
        #[derive(Default)]
        struct Output {
            bytes: Vec<u8>,
            flushed: bool,
        }
        impl Write for Output {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                self.flushed = true;
                Ok(())
            }
        }

        let tasks = [Task {
            status: TaskStatus::Carried,
            ..Task::new("日本語")
        }];

        let mut output = Output::default();
        copy(&tasks, &mut output).unwrap();
        assert_eq!(output.bytes, b"\x1b]52;c;LSBbIF0g5pel5pys6KqeCg==\x07");
        assert!(output.flushed);

        let mut empty = Vec::new();
        copy(&[], &mut empty).unwrap();
        assert_eq!(empty, b"\x1b]52;c;\x07");

        for (text, expected) in [
            ("a", "LSBbIF0gYQo="),
            ("ab", "LSBbIF0gYWIK"),
            ("abc", "LSBbIF0gYWJjCg=="),
        ] {
            let mut output = Vec::new();
            copy(&[Task::new(text)], &mut output).unwrap();

            assert_eq!(output, format!("\x1b]52;c;{expected}\x07").as_bytes());
        }
    }
}
