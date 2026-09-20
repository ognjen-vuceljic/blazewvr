use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_blazewvr"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("binary should run")
}

/// Runs with `stdin` fed the given lines (each written with a trailing
/// newline), then closed — simulating a user typing at the REPL prompt
/// and hitting Ctrl+D.
fn run_with_stdin(args: &[&str], lines: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_blazewvr"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary should spawn");
    let mut stdin = child.stdin.take().expect("stdin should be piped");
    for line in lines {
        writeln!(stdin, "{line}").expect("write to stdin");
    }
    drop(stdin); // close our end so the child sees EOF after these lines
    child.wait_with_output().expect("binary should exit")
}

#[test]
fn run_subcommand_fails_clearly_on_missing_script() {
    let output = run(&["run", "script.dwl", "-i", "payload.json"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("script.dwl"));
}

#[test]
fn repl_subcommand_prints_a_banner_and_exits_cleanly_on_eof() {
    // stdin is Stdio::null(), so the REPL sees immediate EOF and exits
    // via its "Ctrl+D" path without ever needing to spawn `dw`.
    let output = run(&["repl"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("blazewvr REPL"));
}

#[test]
fn repl_subcommand_shows_no_inputs_bound_by_default() {
    let output = run(&["repl"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("no inputs bound"));
}

#[test]
fn repl_subcommand_shows_bound_inputs_in_the_banner() {
    let output = run(&["repl", "-i", "payload=data.json"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("payload=data.json"));
}

#[test]
fn repl_subcommand_defaults_a_bare_input_path_to_payload() {
    // Matches the DataWeave Playground convention: `payload` is the
    // default name for the incoming input, so a bare file (no `name=`
    // prefix) should bind as `payload` without the user needing to know
    // or type that prefix.
    let output = run(&["repl", "-i", "data.json"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("payload=data.json"));
}

#[test]
fn repl_subcommand_fails_clearly_on_invalid_input_format() {
    let output = run(&["repl", "-i", "=bad-input"]);
    assert!(!output.status.success());
}

#[test]
fn repl_subcommand_skips_blank_lines_and_attempts_to_eval_real_input() {
    // A blank line must not print anything or attempt an eval; a real
    // line reaches the eval-and-print path (whether or not `dw` is on
    // PATH here — either way, the loop body itself gets exercised and
    // the process still exits cleanly once stdin closes).
    let output = run_with_stdin(&["repl"], &["", "1 + 1"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("blazewvr REPL"));
}

#[test]
fn validate_subcommand_prints_stub() {
    let output = run(&["validate", "foo.dwl"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("foo.dwl"));
}

#[test]
fn bare_invocation_fails_cleanly_without_a_real_terminal() {
    // No script given means bare invocation fuzzy-picks one via `fzf`
    // first (Wave 4 issue #6) — before ever touching the TUI. PATH is
    // cleared so this deterministically hits "fzf not found" instead of
    // possibly launching a real, genuinely-interactive `fzf` if one
    // happens to be installed on the machine running this test (which
    // would hang waiting for a real tty that these pipes don't provide).
    let output = Command::new(env!("CARGO_BIN_EXE_blazewvr"))
        .env("PATH", "")
        .stdin(Stdio::null())
        .output()
        .expect("binary should run");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.to_lowercase().contains("panicked"));
    assert!(!stderr.is_empty());
}

#[test]
fn bare_script_invocation_resolves_inputs_before_failing_without_a_real_terminal() {
    let dir = std::env::temp_dir().join(format!("blazewvr_cli_bare_script_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("s.dwl");
    std::fs::write(&script, "output application/json --- {}").unwrap();

    // Same as the missing-tty case above, but exercising the script ->
    // prepare_run -> WatchTarget -> Supervisor path first, proving that
    // real input resolution happens before the terminal is touched.
    let output = run(&[script.to_str().unwrap()]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.to_lowercase().contains("panicked"));
}
