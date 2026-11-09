use super::*;

// Parses arguments as if typed after `sati`, keeping only the interactive options.
fn parse_cli(arguments: &[&str]) -> std::result::Result<InteractiveOptions, clap::Error> {
    let arguments = std::iter::once("sati").chain(arguments.iter().copied());

    Cli::try_parse_arguments(arguments).map(|cli| cli.options)
}

fn resolve_defaults(arguments: &[&str]) -> EffectiveOptions {
    parse_cli(arguments)
        .unwrap()
        .resolve(config::Config::default())
}

// Records bytes even when flushing fails, making ambiguous command delivery explicit.
#[derive(Default)]
struct FailOnce {
    bytes: Vec<u8>,
    write_remaining: Option<usize>,
    flush_remaining: Option<usize>,
}

impl io::Write for FailOnce {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = match self.write_remaining {
            Some(0) => {
                self.write_remaining = None;
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            Some(remaining) => {
                let count = bytes.len().min(remaining);
                self.write_remaining = Some(remaining - count);
                count
            }
            None => bytes.len(),
        };

        self.bytes.extend_from_slice(&bytes[..count]);

        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(remaining) = self.flush_remaining.as_mut() {
            if *remaining == 0 {
                self.flush_remaining = None;
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            *remaining -= 1;
        }

        Ok(())
    }
}

#[test]
fn finishing_preserves_application_and_terminal_error_reporting() {
    assert!(finish_session(Ok(()), Ok(())).is_ok());
    assert_eq!(
        finish_session(Err("task save failed".into()), Ok(()))
            .unwrap_err()
            .to_string(),
        "task save failed"
    );

    let error = finish_session(Ok(()), Err(io::ErrorKind::BrokenPipe.into()))
        .unwrap_err()
        .to_string();

    assert!(error.contains("could not restore terminal"));

    let both = finish_session(
        Err("task save failed".into()),
        Err(io::ErrorKind::BrokenPipe.into()),
    )
    .unwrap_err()
    .to_string();

    assert!(both.contains("task save failed") && both.contains("could not restore terminal"));
}

#[test]
fn background_output_tracks_changes_and_restores_once_even_after_output_failure() {
    use ratatui::style::Color;
    let modes = TerminalModes::default();
    let mut output = Vec::new();
    let mut current = None;

    assert!(
        !modes
            .update_background(&mut output, &mut current, None)
            .unwrap()
    );
    assert!(output.is_empty());

    let color = Some(Color::Rgb(40, 40, 40));
    assert!(
        modes
            .update_background(&mut output, &mut current, color)
            .unwrap()
    );
    assert!(
        !modes
            .update_background(&mut output, &mut current, color)
            .unwrap()
    );
    assert_eq!(output, b"\x1b]11;#282828\x1b\\");
    assert!(
        modes
            .update_background(&mut output, &mut current, Some(Color::Reset))
            .unwrap()
    );
    assert_eq!(output, b"\x1b]11;#282828\x1b\\\x1b]111\x1b\\");

    output.clear();
    modes.restore(&mut output).unwrap();
    assert!(output.is_empty());

    modes
        .update_background(&mut output, &mut current, Some(Color::Rgb(1, 2, 3)))
        .unwrap();
    output.clear();
    modes.restore(&mut output).unwrap();
    modes.restore(&mut output).unwrap();

    assert_eq!(output, b"\x1b]111\x1b\\");

    // Whether writing or flushing fails, the terminal may have applied the color.
    for (write_remaining, flush_remaining) in [(Some(0), None), (None, Some(0))] {
        let modes = TerminalModes::default();
        let mut current = None;
        let mut broken = FailOnce {
            write_remaining,
            flush_remaining,
            ..FailOnce::default()
        };

        assert!(
            modes
                .update_background(&mut broken, &mut current, color)
                .is_err()
        );
        assert!(current.is_none());

        let mut recovery = Vec::new();
        modes.restore(&mut recovery).unwrap();
        assert_eq!(recovery, b"\x1b]111\x1b\\");
    }
}

#[test]
fn runtime_themes_synchronize_resolved_backgrounds_and_leave_config_unchanged() {
    use ratatui::{backend::TestBackend, crossterm::event::KeyCode, style::Color};
    use themes::BuiltinTheme::*;

    let fixture = storage::tests::Fixture::new();
    let path = fixture.directory.join("config.toml");

    for (configured, cli_theme, initial, changes) in [
        (
            "gruvbox-material",
            Some("onedark"),
            OneDark,
            [('t', GithubDark), ('T', OneDark), ('T', Nightfox)],
        ),
        (
            "monokai",
            None,
            Monokai,
            [('t', ModusVivendi), ('T', Monokai), ('t', ModusVivendi)],
        ),
        (
            "modus-vivendi-tritanopia",
            None,
            ModusVivendiTritanopia,
            [
                ('t', Default),
                ('T', ModusVivendiTritanopia),
                ('t', Default),
            ],
        ),
        (
            "solarized-dark",
            None,
            SolarizedDark,
            [
                ('t', SolarizedLight),
                ('t', GruvboxLight),
                ('T', SolarizedLight),
            ],
        ),
    ] {
        for custom_background in [None, Some(Color::Rgb(32, 36, 45))] {
            let mut text =
                format!("theme='{configured}'\n[styles.accent]\nfg='red'\nmodifiers=['bold']\n");

            if custom_background.is_some() {
                text.push_str("[styles.normal]\nbg='#20242d'\n");
            }

            std::fs::write(&path, &text).unwrap();
            let mut args = vec!["--accent", "blue"];

            if let Some(name) = cli_theme {
                args.extend(["--theme", name]);
            }

            let options = parse_cli(&args)
                .unwrap()
                .resolve(config::Config::load(Some(&path)).unwrap());
            let mut app = App::new(Vec::new());
            app.appearance = options.appearance;
            let modes = TerminalModes::default();
            let mut current = None;
            let mut output = Vec::new();
            let mut terminal = ratatui::Terminal::new(TestBackend::new(40, 10)).unwrap();

            for (key, expected) in std::iter::once((None, initial))
                .chain(changes.map(|(key, theme)| (Some(key), theme)))
            {
                if let Some(key) = key {
                    app.handle_key(KeyCode::Char(key).into(), &fixture.storage)
                        .unwrap();
                    assert_eq!(
                        app.status_message.as_ref().unwrap().message(),
                        format!("Theme: {}", expected.id())
                    );
                }

                assert_eq!(app.appearance.theme(), expected);
                assert_eq!(
                    app.appearance.styles().normal.fg,
                    expected.base_styles().normal.fg
                );
                assert_eq!(app.appearance.styles().accent.fg, Some(Color::Blue));
                assert_eq!(
                    app.appearance.styles().accent.add_modifier,
                    ratatui::style::Modifier::BOLD
                );

                let background = custom_background.or(expected.base_styles().normal.bg);
                assert_eq!(app.appearance.styles().normal.bg, background);

                let previous = current;
                output.clear();
                let changed = modes
                    .update_background(&mut output, &mut current, app.appearance.styles().normal.bg)
                    .unwrap();

                let expected_output = if previous == background {
                    String::new()
                } else if let Some(Color::Rgb(r, g, b)) = background {
                    format!("\x1b]11;#{r:02x}{g:02x}{b:02x}\x1b\\")
                } else {
                    "\x1b]111\x1b\\".into()
                };

                assert_eq!(changed, previous != background);
                assert_eq!(output, expected_output.as_bytes());

                terminal.clear().unwrap();
                terminal
                    .draw(|frame| {
                        ui::draw(frame, &mut app);
                    })
                    .unwrap();

                assert!(
                    terminal
                        .backend()
                        .buffer()
                        .content
                        .iter()
                        .all(|cell| cell.bg == background.unwrap_or(Color::Reset))
                );
            }

            output.clear();
            modes.restore(&mut output).unwrap();
            assert_eq!(
                output,
                if current.is_some() {
                    b"\x1b]111\x1b\\".as_slice()
                } else {
                    b""
                }
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
            assert!(!fixture.task_path().exists());
            assert!(!app.can_undo() && !app.can_redo());
        }
    }
}

#[test]
fn every_theme_is_selectable_from_cli_over_toml() {
    for &(_, name) in themes::BuiltinTheme::ALL {
        let config: config::Config = toml::from_str("theme='gruvbox-material'").unwrap();
        let cli = parse_cli(&["--theme", name]).unwrap();
        let selected = name.parse::<themes::BuiltinTheme>().unwrap();

        assert_eq!(
            *cli.resolve(config).appearance.styles(),
            styles::Styles::resolve(selected, &config::StyleOverrides::default(), None)
        );
    }
}

#[test]
fn cli_theme_overrides_toml_in_both_directions() {
    for (configured, cli, expected) in [
        (
            "default",
            Some("gruvbox-material"),
            ratatui::style::Color::Rgb(212, 190, 152),
        ),
        (
            "gruvbox-material",
            Some("default"),
            ratatui::style::Color::Reset,
        ),
        (
            "gruvbox-material",
            None,
            ratatui::style::Color::Rgb(212, 190, 152),
        ),
    ] {
        let config = toml::from_str(&format!("theme='{configured}'")).unwrap();
        let args = cli.map_or_else(Vec::new, |name| vec!["--theme", name]);
        let cli = parse_cli(&args).unwrap();

        assert_eq!(
            cli.resolve(config).appearance.styles().normal.fg,
            Some(expected)
        );
    }
}

#[test]
fn theme_selection_keeps_cascading_and_final_accent_precedence() {
    let config: config::Config = toml::from_str("theme='default'\n[styles.normal]\nfg='lightblue'\nmodifiers=['bold']\n[styles.muted]\nmodifiers=[]\n[styles.accent]\nfg='red'\nmodifiers=['bold']").unwrap();
    let cli = parse_cli(&["--theme", "default", "--accent", "blue"]).unwrap();

    assert_eq!(cli.theme, Some(themes::BuiltinTheme::Default));

    let styles = *cli.resolve(config).appearance.styles();
    assert_eq!(styles.normal.fg, Some(ratatui::style::Color::LightBlue));
    assert_eq!(styles.muted.fg, styles.normal.fg);
    assert!(styles.muted.add_modifier.is_empty());
    assert_eq!(styles.completed, styles.muted);
    assert_eq!(styles.border, styles.muted);
    assert_eq!(styles.accent.fg, Some(ratatui::style::Color::Blue));
    assert_eq!(styles.accent.add_modifier, ratatui::style::Modifier::BOLD);
    assert_eq!(
        styles.heading,
        styles.normal.fg(ratatui::style::Color::LightCyan)
    );
    assert_eq!(
        styles.carryover_marker,
        styles.normal.fg(ratatui::style::Color::Yellow)
    );

    // Clap lists the available themes whenever a name is missing or unknown.
    for args in [
        vec!["--theme"],
        vec!["--theme", "unknown"],
        vec!["--theme", "Default"],
    ] {
        let error = parse_cli(&args).unwrap_err().to_string();

        assert!(error.contains("gruvbox-material"), "{error}");
    }

    assert!(parse_cli(&["--theme", "default", "--theme", "default"]).is_err());
    assert!(parse_cli(&["--list-themes", "export"]).is_err());
}

#[test]
fn cli_accent_overrides_only_the_resolved_foreground() {
    let config = || {
        toml::from_str::<config::Config>("[styles.accent]\nfg='blue'\nmodifiers=['bold']").unwrap()
    };

    assert!(parse_cli(&[]).unwrap().accent.is_none());

    let options = parse_cli(&[]).unwrap().resolve(config());

    assert_eq!(
        options.appearance.styles().accent.fg,
        Some(ratatui::style::Color::Blue)
    );

    let options = parse_cli(&["--accent", "red"]).unwrap().resolve(config());

    assert_eq!(
        options.appearance.styles().accent.fg,
        Some(ratatui::style::Color::Red)
    );
    assert_eq!(
        options.appearance.styles().accent.add_modifier,
        ratatui::style::Modifier::BOLD
    );
}

#[test]
fn display_precedence_and_last_flag_of_a_pair_cover_both_boolean_directions() {
    let fields = ["wrap", "status_messages"];

    let flags = [
        ("--wrap", "--no-wrap"),
        ("--status-messages", "--no-status-messages"),
    ];

    let values = |options: EffectiveOptions| [options.wrap, options.status_messages];

    for (index, (yes, no)) in flags.into_iter().enumerate() {
        for configured in [true, false] {
            let text = format!("[display]\n{} = {configured}", fields[index]);
            let config = || toml::from_str::<config::Config>(&text).unwrap();
            assert_eq!(
                values(parse_cli(&[]).unwrap().resolve(config()))[index],
                configured
            );

            for (flag, expected) in [(yes, true), (no, false)] {
                let options = parse_cli(&[flag]).unwrap().resolve(config());

                assert_eq!(values(options)[index], expected);
            }
        }

        // Each flag of a pair overrides the other, so the last one given wins.
        for (pair, expected) in [([yes, no], false), ([no, yes], true)] {
            assert_eq!(values(resolve_defaults(&pair))[index], expected);
        }
    }

    assert!(parse_cli(&["--config", "one", "--config", "two"]).is_err());
    assert!(parse_cli(&["--config"]).is_err());
}

#[test]
fn style_options_take_precedence_over_config_which_takes_precedence_over_defaults() {
    use config::{BorderStyle, TaskMarkerStyle};

    let config = |text: &str| toml::from_str::<config::Config>(text).unwrap();
    let styles = |options: EffectiveOptions| (options.border_style, options.task_markers);

    assert_eq!(
        styles(resolve_defaults(&[])),
        (BorderStyle::Square, TaskMarkerStyle::Unicode)
    );

    let configured = "[display]\nborder_style = 'rounded'\ntask_markers = 'ascii'";

    assert_eq!(
        styles(parse_cli(&[]).unwrap().resolve(config(configured))),
        (BorderStyle::Rounded, TaskMarkerStyle::Ascii)
    );

    let overridden = parse_cli(&["--border-style", "square", "--task-markers", "unicode"])
        .unwrap()
        .resolve(config(configured));

    assert_eq!(
        styles(overridden),
        (BorderStyle::Square, TaskMarkerStyle::Unicode)
    );

    let from_defaults = resolve_defaults(&["--border-style=rounded", "--task-markers=ascii"]);

    assert_eq!(
        styles(from_defaults),
        (BorderStyle::Rounded, TaskMarkerStyle::Ascii)
    );
}

#[test]
fn style_options_reject_unknown_missing_and_removed_values() {
    for args in [
        vec!["--border-style", "round"],
        vec!["--border-style", "Rounded"],
        vec!["--border-style"],
        vec!["--task-markers", "emoji"],
        vec!["--task-markers"],
        vec!["--marker-style", "ascii"],
        vec!["--round-corners"],
        vec!["--square-corners"],
        vec!["--ascii-markers"],
        vec!["--unicode-markers"],
        vec!["--notifications"],
        vec!["--no-notifications"],
    ] {
        assert!(parse_cli(&args).is_err(), "accepted {args:?}");
    }
}

#[test]
fn terminal_commands_encode_repeat_and_paste_reporting_and_restoration() {
    use ratatui::crossterm::{Command, event::EnableBracketedPaste};

    // Check ANSI encoding without assuming the running terminal supports it.
    let mut enabled = String::new();
    EnableBracketedPaste.write_ansi(&mut enabled).unwrap();
    PushKeyboardEnhancementFlags(KEYBOARD_FLAGS)
        .write_ansi(&mut enabled)
        .unwrap();

    assert!(enabled.contains("\x1b[?2004h"));

    let flags: u8 = enabled
        .split("\x1b[>")
        .nth(1)
        .unwrap()
        .split('u')
        .next()
        .unwrap()
        .parse()
        .unwrap();

    // Protocol bits: disambiguation, event types, shifted keys, and all keys (including Enter).
    assert_eq!(flags & 0b1111, 0b1111);

    let mut restored = String::new();
    PopKeyboardEnhancementFlags
        .write_ansi(&mut restored)
        .unwrap();
    DisableBracketedPaste.write_ansi(&mut restored).unwrap();
    DisableMouseCapture.write_ansi(&mut restored).unwrap();
    Show.write_ansi(&mut restored).unwrap();

    assert!(restored.contains("\x1b[?2004l"));
    assert!(restored.contains("\x1b[<1u"));
    assert!(restored.contains("\x1b[?1006l"));
    assert!(restored.ends_with("\x1b[?25h"));
}

#[test]
fn optional_features_only_fall_back_for_unsupported_modes_or_query_timeouts() {
    assert!(optional_mode(Ok(())).unwrap());
    assert!(!optional_mode(Err(io::ErrorKind::Unsupported.into())).unwrap());
    assert!(keyboard_support(Ok(true)).unwrap());
    assert!(!keyboard_support(Ok(false)).unwrap());

    for kind in [io::ErrorKind::Unsupported, io::ErrorKind::TimedOut] {
        assert!(!keyboard_support(Err(kind.into())).unwrap());
    }

    assert!(
        !keyboard_support(Err(io::Error::other(
            "The keyboard enhancement status could not be read within a normal duration"
        )))
        .unwrap()
    );

    for kind in [
        io::ErrorKind::BrokenPipe,
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::Other,
    ] {
        assert_eq!(optional_mode(Err(kind.into())).unwrap_err().kind(), kind);
        assert_eq!(keyboard_support(Err(kind.into())).unwrap_err().kind(), kind);
    }

    for (kind, message) in [
        (io::ErrorKind::Other, "keyboard query reader failed"),
        (
            io::ErrorKind::Other,
            "The keyboard enhancement status could not be read within a normal duration: reader failed",
        ),
        (
            io::ErrorKind::Other,
            "The keyboard enhancement status could not be read within a normal duration ",
        ),
        (
            io::ErrorKind::PermissionDenied,
            "The keyboard enhancement status could not be read within a normal duration",
        ),
    ] {
        let error = keyboard_support(Err(io::Error::new(kind, message))).unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(error.to_string(), message);
    }
}

#[test]
#[cfg(unix)]
fn session_cleanup_orders_cursor_screen_and_background_for_success_and_error() {
    for themed in [false, true] {
        for failed in [false, true] {
            let modes = TerminalModes::default();
            let mut output = Vec::new();
            modes
                .update_background(
                    &mut output,
                    &mut None,
                    themed.then_some(ratatui::style::Color::Rgb(40, 40, 40)),
                )
                .unwrap();
            // Capture cleanup of an initialized screen without changing a real TTY.
            modes.alternate.store(true, Ordering::Relaxed);
            modes.cursor.store(true, Ordering::Relaxed);
            output.clear();

            let result = if failed {
                Err("task save failed".into())
            } else {
                Ok(())
            };

            let finished = finish_session(result, modes.restore(&mut output));
            assert_eq!(finished.is_err(), failed);

            let mut expected = b"\x1b[?25h\x1b[?1049l".to_vec();

            if themed {
                expected.extend_from_slice(b"\x1b]111\x1b\\");
            }

            assert_eq!(output, expected);

            output.clear();
            modes.restore(&mut output).unwrap();
            assert!(output.is_empty());
        }
    }
}

#[test]
fn cleanup_without_enabled_modes_sends_nothing() {
    let modes = TerminalModes::default();
    let mut output = Vec::new();
    modes.restore(&mut output).unwrap();

    assert!(output.is_empty());
}

#[test]
#[cfg(windows)]
fn windows_backend_does_not_enable_kitty_protocol() {
    assert!(!terminal::supports_keyboard_enhancement().unwrap());

    let modes = TerminalModes::default();
    assert!(!modes.keyboard.load(Ordering::Relaxed));
    assert!(!modes.paste.load(Ordering::Relaxed));
}

#[test]
#[cfg(unix)]
fn partial_mouse_enable_is_disabled_even_when_queuing_fails() {
    let mut mouse = String::new();
    EnableMouseCapture.write_ansi(&mut mouse).unwrap();
    let mut disable_mouse = String::new();
    DisableMouseCapture.write_ansi(&mut disable_mouse).unwrap();

    // Each complete prefix can already enable reporting before a later write fails.
    for written in ["\x1b[?1000h".len(), mouse.len() - 1] {
        let modes = TerminalModes::default();
        let mut output = FailOnce {
            write_remaining: Some(written),
            ..FailOnce::default()
        };

        assert_eq!(
            modes
                .enable_input(&mut output, true, true)
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(output.bytes, mouse.as_bytes()[..written]);

        output.bytes.clear();
        modes.restore(&mut output).unwrap();
        assert_eq!(output.bytes, disable_mouse.as_bytes());

        output.bytes.clear();
        modes.restore(&mut output).unwrap();
        assert!(output.bytes.is_empty());
    }
}

#[test]
#[cfg(unix)]
fn partial_setup_and_cleanup_failures_do_not_leave_other_modes_enabled() {
    let mut mouse = String::new();
    EnableMouseCapture.write_ansi(&mut mouse).unwrap();
    let modes = TerminalModes::default();
    let mut output = FailOnce {
        write_remaining: Some(mouse.len()),
        ..FailOnce::default()
    };

    assert_eq!(
        modes
            .enable_input(&mut output, true, true)
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );

    modes.restore(&mut output).unwrap();
    let restored = String::from_utf8(output.bytes).unwrap();
    assert!(restored.contains("\x1b[?1006l"));
    assert!(!restored.contains("\x1b[?2004l"));
    assert!(!restored.contains("\x1b[<1u"));

    let modes = TerminalModes::default();
    modes.enable_input(&mut Vec::new(), true, true).unwrap();
    let mut output = FailOnce {
        write_remaining: Some(0),
        ..FailOnce::default()
    };

    assert_eq!(
        modes.restore(&mut output).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );

    let restored = String::from_utf8(output.bytes).unwrap();
    assert!(restored.contains("\x1b[?2004l"));
    assert!(restored.contains("\x1b[?1006l"));

    let mut repeated = Vec::new();
    modes.restore(&mut repeated).unwrap();
    assert!(repeated.is_empty());

    // A failed flush after queuing mouse capture still leaves it to be disabled.
    let mut disable_mouse = String::new();
    DisableMouseCapture.write_ansi(&mut disable_mouse).unwrap();
    let modes = TerminalModes::default();
    let mut output = FailOnce {
        flush_remaining: Some(0),
        ..FailOnce::default()
    };

    assert_eq!(
        modes
            .enable_input(&mut output, true, true)
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(output.bytes, mouse.as_bytes());

    output.bytes.clear();
    modes.restore(&mut output).unwrap();
    assert_eq!(output.bytes, disable_mouse.as_bytes());
}

#[test]
#[cfg(unix)]
fn cleanup_retries_only_safe_resets_after_partial_writes_and_flush_failures() {
    let mut mouse = String::new();
    DisableMouseCapture.write_ansi(&mut mouse).unwrap();

    let commands = [
        "\x1b[<1u",
        "\x1b[?2004l",
        &mouse,
        "\x1b[?25h",
        "\x1b[?1049l",
        "\x1b]111\x1b\\",
    ];

    for (failed, command) in commands.iter().enumerate() {
        let preceding: usize = commands[..failed].iter().map(|command| command.len()).sum();

        for (partial, flush) in [(0, false), (command.len() / 2, false), (0, true)] {
            let modes = TerminalModes::default();

            for enabled in [
                &modes.keyboard,
                &modes.paste,
                &modes.mouse,
                &modes.cursor,
                &modes.alternate,
                &modes.background,
            ] {
                enabled.store(true, Ordering::Relaxed);
            }

            let mut output = FailOnce {
                write_remaining: (!flush).then_some(preceding + partial),
                flush_remaining: flush.then_some(failed),
                ..FailOnce::default()
            };

            assert_eq!(
                modes.restore(&mut output).unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );

            let mut expected = String::new();

            for (index, command) in commands.iter().enumerate() {
                expected.push_str(if index == failed && !flush {
                    &command[..partial]
                } else {
                    command
                });
            }

            // Every later cleanup step still runs, including OSC 111.
            assert_eq!(
                output.bytes,
                expected.as_bytes(),
                "step {failed}, flush {flush}"
            );

            output.bytes.clear();
            modes.restore(&mut output).unwrap();

            // A failed keyboard pop is deliberately consumed, even after a partial write.
            let retry = if failed == 0 {
                b"".as_slice()
            } else {
                command.as_bytes()
            };

            assert_eq!(output.bytes, retry, "step {failed}, flush {flush}");

            output.bytes.clear();
            modes.restore(&mut output).unwrap();
            assert!(output.bytes.is_empty());
        }
    }
}

#[test]
#[cfg(windows)]
fn failed_windows_mouse_cleanup_never_retries_the_raw_console_snapshot() {
    let modes = TerminalModes::default();
    modes.mouse.store(true, Ordering::Relaxed);
    // Crossterm flushes before its Windows API call. Fail there without touching a console.
    let mut output = FailOnce {
        flush_remaining: Some(0),
        ..FailOnce::default()
    };

    assert_eq!(
        modes.restore(&mut output).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );

    modes.restore(&mut output).unwrap();
    assert!(output.bytes.is_empty());
}

#[test]
#[cfg(unix)]
fn input_cleanup_matches_enabled_modes_and_is_not_repeated() {
    let mut enable_mouse = String::new();
    EnableMouseCapture.write_ansi(&mut enable_mouse).unwrap();

    let mut disable_mouse = String::new();
    DisableMouseCapture.write_ansi(&mut disable_mouse).unwrap();

    for mouse in [false, true] {
        for enhanced in [false, true] {
            let modes = TerminalModes::default();
            let mut output = Vec::new();
            modes.enable_input(&mut output, enhanced, mouse).unwrap();
            let enabled = String::from_utf8(output).unwrap();

            assert_eq!(enabled.contains(&enable_mouse), mouse);
            assert_eq!(enabled.contains("\x1b[?1000h"), mouse);
            assert!(enabled.contains("\x1b[?2004h"));
            assert_eq!(enabled.contains("\x1b[>15u"), enhanced);
            assert_eq!(modes.mouse.load(Ordering::Relaxed), mouse);

            let mut restored = Vec::new();
            modes.restore(&mut restored).unwrap();
            let restored = String::from_utf8(restored).unwrap();

            assert!(restored.contains("\x1b[?2004l"));
            assert_eq!(restored.contains(&disable_mouse), mouse);
            assert_eq!(restored.contains("\x1b[?1000l"), mouse);
            assert_eq!(restored.contains("\x1b[<1u"), enhanced);

            let mut repeated = Vec::new();
            modes.restore(&mut repeated).unwrap();
            assert!(repeated.is_empty());
        }
    }
}

#[test]
#[cfg(unix)]
fn failed_setup_without_mouse_restores_paste_but_never_touches_mouse_reporting() {
    let modes = TerminalModes::default();
    let mut output = FailOnce {
        write_remaining: Some("\x1b[?2004h".len()),
        ..FailOnce::default()
    };

    assert_eq!(
        modes
            .enable_input(&mut output, true, false)
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(output.bytes, b"\x1b[?2004h");

    output.bytes.clear();
    modes.restore(&mut output).unwrap();

    assert_eq!(output.bytes, b"\x1b[?2004l");
}

#[test]
fn mouse_capture_defaults_on_and_cli_overrides_toml_in_both_directions() {
    assert!(resolve_defaults(&[]).mouse);

    for configured in [true, false] {
        let text = format!("[input]\nmouse = {configured}");
        let config = || toml::from_str::<config::Config>(&text).unwrap();

        assert_eq!(parse_cli(&[]).unwrap().resolve(config()).mouse, configured);

        for (flag, expected) in [("--mouse", true), ("--no-mouse", false)] {
            let options = parse_cli(&[flag]).unwrap().resolve(config());

            assert_eq!(options.mouse, expected, "{flag} over {configured}");
        }
    }

    // The last flag of the pair wins, and mouse flags leave display options alone.
    assert!(!resolve_defaults(&["--mouse", "--no-mouse"]).mouse);
    assert!(resolve_defaults(&["--no-mouse", "--mouse"]).mouse);
    assert_eq!(
        resolve_defaults(&["--no-mouse"]),
        EffectiveOptions {
            mouse: false,
            ..resolve_defaults(&[])
        }
    );
    assert!(parse_cli(&["--no-mouse=true"]).is_err());
    assert!(Cli::try_parse_arguments(["sati", "export", "--no-mouse"]).is_err());
    assert!(Cli::try_parse_arguments(["sati", "--no-mouse", "export"]).is_err());
}

#[test]
fn duration_and_help_hint_come_from_config_or_defaults() {
    let defaults = resolve_defaults(&[]);

    assert_eq!(
        defaults.status_message_duration,
        std::time::Duration::from_millis(3560)
    );
    assert!(defaults.show_help_hint);

    let text = "[display]\nstatus_message_duration_ms = 1500\nshow_help_hint = false";
    let config = toml::from_str::<config::Config>(text).unwrap();
    let options = parse_cli(&["--no-wrap"]).unwrap().resolve(config);

    assert_eq!(
        options.status_message_duration,
        std::time::Duration::from_millis(1500)
    );
    assert!(!options.show_help_hint);
    assert!(!options.wrap);

    for flag in [
        "--status-message-duration-ms",
        "--show-help-hint",
        "--no-help-hint",
    ] {
        assert!(parse_cli(&[flag]).is_err(), "accepted {flag}");
    }
}

#[test]
fn the_defaults_file_lists_every_option_at_its_built_in_default() {
    let config: config::Config =
        toml::from_str(include_str!("../../docs/config_defaults.toml")).unwrap();

    // Exhaustive patterns: a new display or input option must be added to the file.
    let config::Display {
        wrap,
        border_style,
        task_markers,
        status_messages,
        status_message_duration_ms,
        show_help_hint,
    } = &config.display;
    let config::Input { mouse } = &config.input;

    assert!(wrap.is_some() && border_style.is_some() && task_markers.is_some());
    assert!(status_messages.is_some() && status_message_duration_ms.is_some());
    assert!(show_help_hint.is_some() && mouse.is_some());

    assert_eq!(
        parse_cli(&[]).unwrap().resolve(config),
        resolve_defaults(&[])
    );
}

#[test]
fn wrapping_is_enabled_by_default_and_can_be_disabled() {
    let defaults = resolve_defaults(&[]);
    assert!(defaults.wrap);
    assert_eq!(defaults.border_style, config::BorderStyle::Square);
    assert_eq!(defaults.task_markers, config::TaskMarkerStyle::Unicode);
    assert!(defaults.status_messages);
    assert_eq!(
        defaults.appearance.styles().accent.fg,
        Some(styles::DEFAULT_ACCENT)
    );

    let options = resolve_defaults(&["--task-markers", "ascii", "--no-wrap"]);

    assert_eq!(options.task_markers, config::TaskMarkerStyle::Ascii);
    assert!(!options.wrap);

    for removed in ["--keybinds", "--no-keybinds", "--unknown"] {
        assert!(parse_cli(&[removed]).is_err());
    }

    let options = resolve_defaults(&["--border-style", "rounded"]);

    assert_eq!(options.border_style, config::BorderStyle::Rounded);
    assert!(options.wrap);
}

#[test]
fn help_and_version_requests_stop_parsing_before_startup() {
    use clap::error::ErrorKind;

    for (args, kind) in [
        (vec!["--help"], ErrorKind::DisplayHelp),
        (vec!["-h"], ErrorKind::DisplayHelp),
        (vec!["export", "--help"], ErrorKind::DisplayHelp),
        (vec!["--version"], ErrorKind::DisplayVersion),
        (vec!["--no-wrap", "--version"], ErrorKind::DisplayVersion),
    ] {
        assert_eq!(parse_cli(&args).unwrap_err().kind(), kind, "{args:?}");
    }
}

#[test]
fn export_is_a_subcommand_that_takes_no_display_options() {
    let export = |arguments: &[&str]| {
        let arguments = ["sati", "export"]
            .into_iter()
            .chain(arguments.iter().copied());

        Cli::try_parse_arguments(arguments)
            .unwrap()
            .command
            .unwrap()
    };
    let section_export = |section: Option<&str>, unsectioned| CliCommand::Export {
        section: section.map(str::to_owned),
        unsectioned,
    };

    assert_eq!(export(&[]), section_export(None, false));
    assert_eq!(export(&["Work"]), section_export(Some("Work"), false));
    assert_eq!(export(&["--unsectioned"]), section_export(None, true));
    assert_eq!(
        export(&["--data-dir", "work", "home"]),
        section_export(Some("home"), false)
    );

    assert!(
        Cli::try_parse_arguments(["sati"])
            .unwrap()
            .command
            .is_none()
    );

    for args in [
        vec!["sati", "--no-wrap", "export"],
        vec!["sati", "export", "--no-wrap"],
        vec!["sati", "export", "work", "home"],
        vec!["sati", "export", "work", "--unsectioned"],
        vec!["sati", "help"],
        vec!["sati", "--config", "sati.toml", "export"],
        vec!["sati", "--data-dir", "work", "--theme", "default", "export"],
    ] {
        assert!(
            Cli::try_parse_arguments(&args).is_err(),
            "accepted {args:?}"
        );
    }

    let error = Cli::try_parse_arguments(["sati", "--no-mouse", "export"]).unwrap_err();

    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    assert!(
        error
            .to_string()
            .contains("'export' cannot be used with '--no-mouse'")
    );
}

#[test]
fn data_dir_is_global_optional_and_independent_of_config() {
    let parse = |arguments: &[&str]| {
        let arguments = std::iter::once("sati").chain(arguments.iter().copied());

        Cli::try_parse_arguments(arguments).unwrap()
    };

    assert!(parse(&[]).data_dir.is_none());
    assert!(parse(&["export"]).data_dir.is_none());

    for arguments in [
        vec!["--data-dir", "work", "export"],
        vec!["export", "--data-dir", "work"],
        vec!["export", "--data-dir=work"],
    ] {
        let cli = parse(&arguments);

        assert_eq!(
            cli.command,
            Some(CliCommand::Export {
                section: None,
                unsectioned: false
            }),
            "{arguments:?}"
        );
        assert_eq!(cli.data_dir, Some(PathBuf::from("work")), "{arguments:?}");
    }

    let cli = parse(&["--config", "work.toml", "--data-dir", "work", "--no-wrap"]);

    assert!(cli.command.is_none());
    assert_eq!(cli.data_dir, Some(PathBuf::from("work")));
    assert_eq!(cli.options.config, Some(PathBuf::from("work.toml")));

    let cli = parse(&["--data-dir", "work"]);

    assert!(cli.options.config.is_none());

    for arguments in [
        vec!["sati", "--data-dir"],
        vec!["sati", "--data-dir", ""],
        vec!["sati", "--data-dir", "a", "--data-dir", "b"],
    ] {
        assert!(
            Cli::try_parse_arguments(&arguments).is_err(),
            "accepted {arguments:?}"
        );
    }

    // Clap lets a global argument appear once on each level; the subcommand's value wins.
    let cli = parse(&["--data-dir", "a", "export", "--data-dir", "b"]);

    assert_eq!(cli.data_dir, Some(PathBuf::from("b")));
}

#[test]
#[cfg(unix)]
fn data_dir_keeps_non_utf8_paths_intact() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    let directory = OsStr::from_bytes(b"checklists/\xffwork");
    let interactive = [OsStr::new("sati"), OsStr::new("--data-dir"), directory];
    let export = [
        OsStr::new("sati"),
        OsStr::new("export"),
        OsStr::new("--data-dir"),
        directory,
    ];

    let interactive = Cli::try_parse_arguments(interactive).unwrap();
    let export = Cli::try_parse_arguments(export).unwrap();

    assert_eq!(interactive.data_dir, Some(PathBuf::from(directory)));
    assert_eq!(export.data_dir, Some(PathBuf::from(directory)));
}

#[test]
fn cli_definition_passes_clap_consistency_checks() {
    use clap::CommandFactory;

    Cli::command().debug_assert();
}

#[test]
fn accent_accepts_hex_colors_and_combines_with_no_wrap() {
    for values in [
        ["--accent", "#a9B665", "--no-wrap"],
        ["--no-wrap", "--accent", "#A9b665"],
    ] {
        let options = resolve_defaults(&values);
        assert!(!options.wrap);
        assert_eq!(
            options.appearance.styles().accent.fg,
            Some(ratatui::style::Color::Rgb(169, 182, 101))
        );
    }

    for color in ["#000000", "#ffffff"] {
        assert!(parse_accent(color).is_ok());
    }
}

#[test]
fn accent_accepts_native_named_and_indexed_colors() {
    use ratatui::style::Color;

    for (text, expected) in [
        ("yellow", Color::Yellow),
        ("lightblue", Color::LightBlue),
        ("3", Color::Indexed(3)),
        ("255", Color::Indexed(255)),
    ] {
        assert_eq!(parse_accent(text).unwrap(), expected);
    }
}

#[test]
fn accent_rejects_missing_or_invalid_values() {
    assert!(parse_cli(&["--accent"]).is_err());

    for color in [
        "",
        "orange",
        "reset",
        "256",
        "e78a4e",
        "#abc",
        "#12345678",
        "#zzzzzz",
        "#é1234",
        "--no-wrap",
    ] {
        assert!(parse_cli(&["--accent", color]).is_err(), "accepted {color}");
    }
}
