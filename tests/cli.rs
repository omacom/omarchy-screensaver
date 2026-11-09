use std::process::{Command, Stdio};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_omarchy-launch-screensaver"))
}

#[test]
fn help_describes_native_rust_contract() {
    let output = binary().arg("--help").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Rust + Wayland + ttfx"));
    assert!(stdout.contains("No terminal, Qt, C++, or ttfx child"));
}

#[test]
fn seeded_headless_render_has_content() {
    let output = binary()
        .args([
            "--headless",
            "--frames",
            "80",
            "--effect",
            "print",
            "--cols",
            "40",
            "--rows",
            "12",
            "--seed",
            "1",
            "--input",
            "assets/screensaver.txt",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("frames=80 cells=40x12"));
    assert!(stdout.contains("backend=rust"));
    assert!(!stdout.contains("non_space=0"));
}

#[test]
fn headless_render_survives_closed_standard_streams() {
    let status = binary()
        .args([
            "--headless",
            "--frames",
            "10",
            "--effect",
            "print",
            "--cols",
            "40",
            "--rows",
            "12",
            "--seed",
            "1",
            "--input",
            "assets/screensaver.txt",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn invalid_argument_returns_usage_error() {
    let output = binary().arg("--not-an-option").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown arg --not-an-option"));
}

/// Renders one headless frame with HOME pointing at a shell.json holding
/// `config`, and returns the effect the screensaver picked.
fn effect_picked_with_shell_json(test: &str, config: &str, args: &[&str]) -> String {
    let home =
        std::env::temp_dir().join(format!("omarchy-screensaver-{test}-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".config/omarchy")).unwrap();
    std::fs::write(home.join(".config/omarchy/shell.json"), config).unwrap();
    let output = binary()
        .env("HOME", &home)
        .args([
            "--headless",
            "--frames",
            "1",
            "--input",
            "assets/screensaver.txt",
        ])
        .args(args)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&home).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    stdout
        .split_whitespace()
        .find_map(|field| field.strip_prefix("effect="))
        .expect("headless output names the effect")
        .to_owned()
}

#[test]
fn shell_json_effects_limit_the_random_pick() {
    for seed in ["1", "2", "3", "4", "5"] {
        let effect = effect_picked_with_shell_json(
            &format!("limit-{seed}"),
            r#"{"screensaver": {"effects": ["print", "wipe"]}}"#,
            &["--seed", seed],
        );
        assert!(effect == "print" || effect == "wipe", "picked {effect}");
    }
}

#[test]
fn shell_json_effects_ttfx_lacks_are_skipped() {
    let effect = effect_picked_with_shell_json(
        "skip",
        r#"{"screensaver": {"effects": ["no-such-effect", "print"]}}"#,
        &["--seed", "1"],
    );
    assert_eq!(effect, "print");
}

#[test]
fn shell_json_with_only_unknown_effects_still_starts() {
    effect_picked_with_shell_json(
        "unknown",
        r#"{"screensaver": {"effects": ["no-such-effect"]}}"#,
        &["--seed", "1"],
    );
}

#[test]
fn command_line_effects_override_shell_json() {
    let config = r#"{"screensaver": {"effects": ["print"]}}"#;
    assert_eq!(
        effect_picked_with_shell_json("include", config, &["--include-effects", "wipe"]),
        "wipe"
    );
    assert_eq!(
        effect_picked_with_shell_json("effect", config, &["--effect", "wipe"]),
        "wipe"
    );
}

#[test]
fn excluding_every_shell_json_effect_still_starts() {
    for seed in ["1", "2", "3", "4", "5"] {
        let effect = effect_picked_with_shell_json(
            &format!("exclude-all-{seed}"),
            r#"{"screensaver": {"effects": ["print"]}}"#,
            &["--exclude-effects", "print", "--seed", seed],
        );
        assert_ne!(effect, "print");
    }
}

#[test]
fn excluded_effects_win_over_shell_json() {
    let effect = effect_picked_with_shell_json(
        "exclude",
        r#"{"screensaver": {"effects": ["print", "wipe"]}}"#,
        &["--exclude-effects", "print", "--seed", "1"],
    );
    assert_eq!(effect, "wipe");
}
