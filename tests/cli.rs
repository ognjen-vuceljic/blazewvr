use std::process::Command;

fn run(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_blazewvr"))
        .args(args)
        .output()
        .expect("binary should run");
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

#[test]
fn run_subcommand_prints_stub() {
    let out = run(&["run", "script.dwl", "-i", "payload.json"]);
    assert!(out.contains("script.dwl"));
    assert!(out.contains("payload.json"));
}

#[test]
fn repl_subcommand_prints_stub() {
    let out = run(&["repl"]);
    assert!(out.contains("not yet implemented"));
}

#[test]
fn validate_subcommand_prints_stub() {
    let out = run(&["validate", "foo.dwl"]);
    assert!(out.contains("foo.dwl"));
}
