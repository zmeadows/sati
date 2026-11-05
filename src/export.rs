use std::{
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use crate::{
    app::Task,
    storage::{self, Result, Storage},
};

pub const USAGE: &str = "Usage: sati export [--output PATH | -o PATH] [--force]";

pub fn markdown(tasks: &[Task]) -> String {
    let mut output = String::new();
    for task in tasks {
        output.push_str(if task.completed { "- [x] " } else { "- [ ] " });
        for character in task.text.chars() {
            if "\\`*_{}[]<>!&#|~+-.()".contains(character) {
                output.push('\\');
            }
            output.push(character);
        }
        output.push('\n');
    }
    output
}

pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<()> {
    let mut output: Option<PathBuf> = None;
    let mut force = false;
    let mut help = false;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == "--output" || argument == "-o" {
            if output.is_some() {
                return Err("output path specified more than once".into());
            }
            let value = arguments.next().ok_or("--output requires a path")?;
            if ["--force", "--output", "-o", "--help", "-h"]
                .iter()
                .any(|flag| value == *flag)
            {
                return Err("--output requires a path".into());
            }
            output = Some(value.into());
        } else if argument == "--force" {
            force = true;
        } else if argument == "--help" || argument == "-h" {
            help = true;
        } else {
            return Err(format!(
                "unknown export argument: {}. {USAGE}",
                argument.to_string_lossy()
            )
            .into());
        }
    }
    if help {
        println!(
            "{USAGE}\n\nExport the current checklist as Markdown (stdout by default).\n--output, -o PATH  Write to a file.\n--force            Replace an existing output file."
        );
        return Ok(());
    }
    if force && output.is_none() {
        return Err("--force requires an output path".into());
    }
    let storage = Storage::discover()?;
    // Atomic task-file replacement allows a consistent read without the writer lock.
    let contents = markdown(&storage.load()?);
    if let Some(path) = output {
        let path = storage.export_path(&path)?;
        write_file(&path, contents.as_bytes(), force)
    } else {
        io::stdout().lock().write_all(contents.as_bytes())?;
        Ok(())
    }
}

fn write_file(path: &Path, contents: &[u8], force: bool) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("sati-export");
    let (temporary, mut file) = storage::create_temporary(path.parent().unwrap(), name)?;
    let result = (|| -> io::Result<()> {
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        if force {
            fs::rename(&temporary, path)?;
        } else {
            // Unlike a pre-check followed by rename, this cannot replace a concurrent creation.
            fs::hard_link(&temporary, path)?;
        }
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result.map_err(|error| format!("could not export to {}: {error}", path.display()).into())
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
            Task {
                text: "Buy groceries".into(),
                completed: false,
                carried: false,
            },
            Task {
                text: "Walk the dog".into(),
                completed: true,
                carried: false,
            },
            Task {
                text: "Work: Review *PR* & <notes> [x] \\ `code`".into(),
                completed: false,
                carried: true,
            },
            Task {
                text: "1. nested 日本語 👩‍💻 e\u{301}".into(),
                completed: false,
                carried: false,
            },
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
            text: "日本語".into(),
            completed: false,
            carried: true,
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
            copy(
                &[Task {
                    text: text.into(),
                    completed: false,
                    carried: false,
                }],
                &mut output,
            )
            .unwrap();
            assert_eq!(output, format!("\x1b]52;c;{expected}\x07").as_bytes());
        }
    }

    #[test]
    fn file_publication_never_truncates_existing_files_and_cleans_staging() {
        let fixture = crate::storage::tests::Fixture::new();
        let output = fixture.directory.join("tasks.md");
        write_file(&output, b"first\n", false).unwrap();
        assert!(write_file(&output, b"second\n", false).is_err());
        assert_eq!(fs::read(&output).unwrap(), b"first\n");
        write_file(&output, b"second\n", true).unwrap();
        assert_eq!(fs::read(&output).unwrap(), b"second\n");
        assert!(write_file(&fixture.directory.join("missing/tasks.md"), b"new", false).is_err());
        let directory = fixture.directory.join("directory.md");
        fs::create_dir(&directory).unwrap();
        assert!(write_file(&directory, b"new", true).is_err());
        assert!(directory.is_dir());
        assert_eq!(fs::read_dir(&fixture.directory).unwrap().count(), 2);
    }
}
