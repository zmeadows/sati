use std::process::{Command, Stdio};

#[test]
fn version_succeeds_without_a_terminal_or_valid_storage_environment() {
    let output = Command::new(env!("CARGO_BIN_EXE_sati"))
        .arg("--version")
        .env("XDG_DATA_HOME", "relative-path-is-invalid")
        .env_remove("HOME")
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("sati {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn invalid_arguments_fail_with_usage_before_startup() {
    let output = Command::new(env!("CARGO_BIN_EXE_sati"))
        .args(["--no-wrap", "--unknown"])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    let error = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(error.contains("unexpected argument '--unknown'"), "{error}");
    assert!(error.contains("Usage:"), "{error}");
}

#[test]
fn marker_styles_are_documented_and_combine_with_existing_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_sati"))
        .args(["--marker-style", "ascii", "--help"])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(output.status.success());

    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--marker-style") && help.contains("[x]") && help.contains("[>]"));
    assert!(help.contains("--border-style") && help.contains("rounded"));
    assert!(help.contains("--status-messages") && help.contains("--no-status-messages"));
    assert!(help.contains("Input:") && help.contains("--mouse") && help.contains("--no-mouse"));

    let output = Command::new(env!("CARGO_BIN_EXE_sati"))
        .args(["--marker-style", "ascii", "--no-wrap", "--version"])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("sati {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn invalid_style_values_fail_with_the_accepted_values() {
    let output = Command::new(env!("CARGO_BIN_EXE_sati"))
        .args(["--border-style", "round"])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    let error = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(2));
    assert!(error.contains("invalid value 'round'"), "{error}");
    assert!(
        error.contains("[possible values: square, rounded]"),
        "{error}"
    );
}
