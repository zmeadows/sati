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
fn version_does_not_hide_invalid_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_sati"))
        .args(["--version", "--unknown"])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown argument: --unknown"));
}

#[test]
fn ascii_markers_are_documented_and_combine_with_existing_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_sati"))
        .args(["--ascii-markers", "--help"])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(output.status.success());

    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--ascii-markers") && help.contains("[x]") && help.contains("[>]"));

    let output = Command::new(env!("CARGO_BIN_EXE_sati"))
        .args(["--ascii-markers", "--no-wrap", "--version"])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("sati {}\n", env!("CARGO_PKG_VERSION"))
    );
}
