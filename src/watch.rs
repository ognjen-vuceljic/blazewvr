//! Watch mode: the core interactive loop. The user edits a script (and/or
//! its bound input files) in their own editor; this polls mtimes and
//! re-evaluates on change, printing a status header + colorized result.

use crate::repl::Supervisor;
use crate::{colorize, dwl_highlight, flatten, status};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

/// One `-i name=file` input binding plus the script being watched.
pub struct WatchTarget {
    pub script: PathBuf,
    pub inputs: Vec<(String, PathBuf)>,
}

/// Parses a `-i name=file` argument into `(name, file)`.
pub fn parse_input(s: &str) -> Result<(String, PathBuf), String> {
    match s.split_once('=') {
        Some((name, file)) if !name.is_empty() && !file.is_empty() => {
            Ok((name.to_string(), PathBuf::from(file)))
        }
        _ => Err(format!("invalid input '{s}', expected name=file")),
    }
}

/// All paths whose modification time should be polled for `target`.
fn watched_paths(target: &WatchTarget) -> Vec<PathBuf> {
    let mut paths = vec![target.script.clone()];
    paths.extend(target.inputs.iter().map(|(_, file)| file.clone()));
    paths
}

/// Modification times for `paths`, `None` for any that can't be stat'd.
fn mtimes(paths: &[PathBuf]) -> Vec<Option<SystemTime>> {
    paths
        .iter()
        .map(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
        .collect()
}

/// Reads, flattens, and evaluates the script once, returning the header +
/// colorized result (or error) as the text to print.
///
/// Successful output (JSON/XML/CSV-shaped) uses the generic structural
/// colorizer; error text uses the DataWeave-aware highlighter instead,
/// since `dw` echoes a snippet of the offending DWL source inside its
/// error messages (e.g. `5| payload.items filter (...)`) — that reads as
/// code, not as generic structured data.
pub fn eval_once(sup: &mut Supervisor, target: &WatchTarget) -> String {
    let start = Instant::now();
    let outcome = fs::read_to_string(&target.script)
        .map_err(|e| format!("reading {}: {e}", target.script.display()))
        .and_then(|src| sup.eval(&flatten::flatten(&src)).map_err(|e| e.to_string()));
    let duration = start.elapsed();
    let header = status::render_header(&target.script, &target.inputs, duration, SystemTime::now());

    match outcome {
        Ok(text) => format!("{header}\n{}", colorize::colorize(&text)),
        Err(err) => format!(
            "{header}\n{}",
            dwl_highlight::highlight(&format!("error: {err}"))
        ),
    }
}

/// Polls `target`'s watched paths every `interval`, calling `on_output`
/// with `eval_once`'s result immediately and again on every change.
/// Keeps looping while `should_continue` returns true (checked between
/// polls), so tests can bound the loop instead of running forever.
pub fn run_loop(
    mut sup: Supervisor,
    target: &WatchTarget,
    interval: Duration,
    mut on_output: impl FnMut(&str),
    mut should_continue: impl FnMut() -> bool,
) {
    let paths = watched_paths(target);
    let mut last = mtimes(&paths);
    on_output(&eval_once(&mut sup, target));

    while should_continue() {
        std::thread::sleep(interval);
        let current = mtimes(&paths);
        if current != last {
            last = current;
            on_output(&eval_once(&mut sup, target));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repl::ReplConfig;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn unique_path(label: &str) -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "blazewvr_watch_{label}_{}_{id}",
            std::process::id()
        ))
    }

    /// Same echo-protocol fake REPL used in repl.rs's tests.
    fn fake_config() -> ReplConfig {
        let path = unique_path("fake_repl.sh");
        let script = "#!/bin/sh\nprintf 'FAKE REPL\\n>>> '\nwhile IFS= read -r line; do\n  printf 'ECHO:%s\\n>>> ' \"$line\"\ndone\n";
        fs::write(&path, script).expect("write fake repl script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        ReplConfig {
            program: PathBuf::from("/bin/sh"),
            args: vec![path.display().to_string()],
        }
    }

    #[test]
    fn parse_input_splits_name_and_file() {
        assert_eq!(
            parse_input("payload=data.json"),
            Ok(("payload".to_string(), PathBuf::from("data.json")))
        );
    }

    #[test]
    fn parse_input_rejects_missing_equals() {
        assert!(parse_input("payload").is_err());
    }

    #[test]
    fn parse_input_rejects_empty_name() {
        assert!(parse_input("=data.json").is_err());
    }

    #[test]
    fn parse_input_rejects_empty_file() {
        assert!(parse_input("payload=").is_err());
    }

    #[test]
    fn watched_paths_includes_script_and_inputs() {
        let target = WatchTarget {
            script: PathBuf::from("s.dwl"),
            inputs: vec![("payload".to_string(), PathBuf::from("p.json"))],
        };
        assert_eq!(
            watched_paths(&target),
            vec![PathBuf::from("s.dwl"), PathBuf::from("p.json")]
        );
    }

    #[test]
    fn mtimes_is_none_for_missing_file() {
        let missing = unique_path("does_not_exist");
        assert_eq!(mtimes(&[missing]), vec![None]);
    }

    #[test]
    fn mtimes_is_some_for_existing_file() {
        let path = unique_path("exists");
        fs::write(&path, "hi").unwrap();
        let result = mtimes(&[path]);
        assert!(result[0].is_some());
    }

    #[test]
    fn eval_once_reports_error_for_missing_script() {
        let target = WatchTarget {
            script: unique_path("missing.dwl"),
            inputs: vec![],
        };
        let mut sup = Supervisor::new(fake_config());
        let out = eval_once(&mut sup, &target);
        assert!(out.contains("error"));
    }

    #[test]
    fn eval_once_flattens_and_evals_script_content() {
        let script_path = unique_path("script.dwl");
        fs::write(&script_path, "hello\n// a comment\n").unwrap();
        let target = WatchTarget {
            script: script_path,
            inputs: vec![],
        };
        let mut sup = Supervisor::new(fake_config());
        let out = crate::colorize::strip_ansi(&eval_once(&mut sup, &target));
        assert!(out.contains("ECHO:hello"));
    }

    #[test]
    fn run_loop_reevaluates_on_script_change() {
        let script_path = unique_path("watched.dwl");
        fs::write(&script_path, "one").unwrap();
        let target = WatchTarget {
            script: script_path.clone(),
            inputs: vec![],
        };
        let sup = Supervisor::new(fake_config());

        let mut outputs: Vec<String> = Vec::new();
        let mut iterations = 0;
        run_loop(
            sup,
            &target,
            Duration::from_millis(20),
            |out| outputs.push(crate::colorize::strip_ansi(out)),
            || {
                iterations += 1;
                if iterations == 1 {
                    // ensure a distinct mtime from the initial write
                    std::thread::sleep(Duration::from_millis(20));
                    fs::write(&script_path, "two").unwrap();
                }
                iterations <= 3
            },
        );

        assert!(
            outputs.len() >= 2,
            "expected at least the initial eval plus one re-eval, got {}",
            outputs.len()
        );
        assert!(outputs[0].contains("ECHO:one"));
        assert!(outputs.last().unwrap().contains("ECHO:two"));
    }
}
