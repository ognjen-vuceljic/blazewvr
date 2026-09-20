//! fzf-backed fuzzy picking. `blazewvr` doesn't hard-depend on `fzf` being
//! installed — callers check `is_available()` first and fall back to
//! requiring an explicit argument with a clear error when it's missing.

// ponytail: not yet wired into a command (lands with Wave 3 CLI wiring);
// allowed dead here so this PR can ship the picker standalone with full
// test coverage against a fake fzf-like script (fzf needs a real
// terminal for its UI, which CI doesn't have).
#![allow(dead_code)]

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Which picker binary to invoke.
pub struct PickerConfig {
    pub program: PathBuf,
}

impl Default for PickerConfig {
    fn default() -> Self {
        Self {
            program: PathBuf::from("fzf"),
        }
    }
}

/// True if `program` can be executed at all (used to check `fzf` is on
/// `PATH` before relying on it).
pub fn is_available(program: &Path) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Pipes `candidates` into the picker and returns the selected line, or
/// `None` if nothing was selected (empty list, or the user backed out).
///
/// Writes to the child's stdin on a separate thread while reading its
/// stdout on this one. Writing and reading sequentially on one thread
/// deadlocks once either pipe's OS buffer fills (parent blocked writing
/// while the child blocks writing its own output that nobody's reading
/// yet) — a real risk here since candidate lists can be large. A broken
/// pipe from the writer (the picker selecting and exiting before it has
/// consumed all input — the common case, not an edge case) is not
/// treated as an error.
pub fn pick(config: &PickerConfig, candidates: &[String]) -> io::Result<Option<String>> {
    if candidates.is_empty() {
        return Ok(None);
    }

    let mut child = Command::new(&config.program)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("picker stdin not piped"))?;
    let data = candidates.join("\n");
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(data.as_bytes());
    });

    let output = child.wait_with_output()?;
    let _ = writer.join();
    let selection = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if selection.is_empty() {
        Ok(None)
    } else {
        Ok(Some(selection))
    }
}

/// Recursively finds `.dwl` files under `root`, skipping hidden
/// directories, `target/`, and symlinks (both to avoid following a
/// symlink cycle into unbounded recursion, and because resolving them
/// correctly is more than this needs). Directories that can't be read
/// are skipped with a warning on stderr rather than silently, so a
/// truncated result isn't mistaken for a genuinely empty one.
pub fn find_dwl_scripts(root: &Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    walk(root, &mut results);
    results.sort();
    results
}

fn walk(dir: &Path, results: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            eprintln!("blazewvr: warning: couldn't read {}: {err}", dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.starts_with('.') {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if name_str == "target" {
                continue;
            }
            walk(&path, results);
        } else if file_type.is_file() && path.extension().and_then(|e| e.to_str()) == Some("dwl") {
            results.push(path);
        }
    }
}

/// Finds `.dwl` scripts under `root` and lets the user fuzzy-pick one.
/// Errs clearly if `fzf` isn't available, rather than silently doing
/// nothing.
///
/// Candidates are matched back to their original `PathBuf` by an exact
/// lookup on the same (possibly lossy, for non-UTF-8 names) display
/// string sent to the picker, rather than by re-parsing the picker's
/// text output into a path — the latter would silently produce a
/// different, nonexistent path for any filename containing invalid
/// UTF-8, since the lossy substitution isn't reversible.
pub fn pick_script(config: &PickerConfig, root: &Path) -> io::Result<Option<PathBuf>> {
    if !is_available(&config.program) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "{} not found on PATH; pass a script path explicitly",
                config.program.display()
            ),
        ));
    }

    let scripts = find_dwl_scripts(root);
    let candidates: Vec<String> = scripts.iter().map(|p| p.display().to_string()).collect();

    match pick(config, &candidates)? {
        Some(selected) => Ok(scripts
            .into_iter()
            .find(|p| p.display().to_string() == selected)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn test_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("blazewvr_picker_{}_{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    /// Fake fzf: reads candidates from stdin, prints the one matching
    /// $FAKE_PICK (or the first line if unset) to stdout.
    fn fake_fzf_script() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("blazewvr_fake_fzf_{}_{id}.sh", std::process::id()));
        let script = r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "fake-fzf 0.0.0"
  exit 0
fi
if [ -n "$FAKE_PICK" ]; then
  grep -F "$FAKE_PICK"
else
  head -n1
fi
"#;
        fs::write(&path, script).expect("write fake fzf script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    #[test]
    fn is_available_true_for_working_program() {
        assert!(is_available(&fake_fzf_script()));
    }

    #[test]
    fn is_available_false_for_missing_program() {
        assert!(!is_available(&PathBuf::from("/nonexistent/blazewvr/nope")));
    }

    #[test]
    fn pick_returns_none_for_empty_candidates() {
        let config = PickerConfig {
            program: fake_fzf_script(),
        };
        assert_eq!(pick(&config, &[]).unwrap(), None);
    }

    #[test]
    fn pick_returns_first_candidate_by_default() {
        let config = PickerConfig {
            program: fake_fzf_script(),
        };
        let candidates = vec!["a.dwl".to_string(), "b.dwl".to_string()];
        assert_eq!(
            pick(&config, &candidates).unwrap(),
            Some("a.dwl".to_string())
        );
    }

    #[test]
    fn find_dwl_scripts_recurses_and_skips_target_and_hidden_dirs() {
        let dir = test_dir();
        fs::write(dir.join("a.dwl"), "x").unwrap();
        fs::write(dir.join("b.txt"), "x").unwrap();
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/c.dwl"), "x").unwrap();
        fs::create_dir_all(dir.join("target")).unwrap();
        fs::write(dir.join("target/d.dwl"), "x").unwrap();
        fs::create_dir_all(dir.join(".hidden")).unwrap();
        fs::write(dir.join(".hidden/e.dwl"), "x").unwrap();

        let found = find_dwl_scripts(&dir);
        assert_eq!(found, vec![dir.join("a.dwl"), dir.join("sub/c.dwl")]);
    }

    #[test]
    fn pick_script_errs_when_fzf_unavailable() {
        let config = PickerConfig {
            program: PathBuf::from("/nonexistent/blazewvr/nope"),
        };
        let dir = test_dir();
        let result = pick_script(&config, &dir);
        assert!(result.is_err());
    }

    #[test]
    fn pick_script_returns_none_when_no_scripts_found() {
        let config = PickerConfig {
            program: fake_fzf_script(),
        };
        let dir = test_dir();
        assert_eq!(pick_script(&config, &dir).unwrap(), None);
    }

    #[test]
    fn pick_script_returns_selected_path() {
        let config = PickerConfig {
            program: fake_fzf_script(),
        };
        let dir = test_dir();
        fs::write(dir.join("only.dwl"), "x").unwrap();
        let result = pick_script(&config, &dir).unwrap();
        assert_eq!(result, Some(dir.join("only.dwl")));
    }

    #[test]
    fn find_dwl_scripts_does_not_follow_a_symlinked_directory_cycle() {
        let dir = test_dir();
        fs::write(dir.join("a.dwl"), "x").unwrap();
        std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();

        // Would recurse forever (stack overflow) before the file_type()
        // fix, since `loop` -> dir -> `loop` -> ... via path.is_dir().
        let found = find_dwl_scripts(&dir);
        assert_eq!(found, vec![dir.join("a.dwl")]);
    }

    #[test]
    fn find_dwl_scripts_does_not_follow_a_symlinked_file() {
        let dir = test_dir();
        fs::write(dir.join("real.dwl"), "x").unwrap();
        std::os::unix::fs::symlink(dir.join("real.dwl"), dir.join("link.dwl")).unwrap();

        let found = find_dwl_scripts(&dir);
        assert_eq!(found, vec![dir.join("real.dwl")]);
    }

    #[test]
    fn find_dwl_scripts_skips_unreadable_directory_without_panicking() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = test_dir();
        fs::write(dir.join("visible.dwl"), "x").unwrap();
        let locked = dir.join("locked");
        fs::create_dir_all(&locked).unwrap();
        fs::write(locked.join("hidden.dwl"), "x").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        // If tests run as root, chmod 000 doesn't actually block reads —
        // skip the assertion in that case rather than flake.
        let root_can_still_read = fs::read_dir(&locked).is_ok();
        let found = find_dwl_scripts(&dir);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

        if !root_can_still_read {
            assert_eq!(found, vec![dir.join("visible.dwl")]);
        }
    }

    #[test]
    fn pick_script_matches_non_utf8_filename_back_to_original_path() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = test_dir();
        // 0xFF is not valid UTF-8 on its own. Some filesystems (e.g.
        // macOS's APFS) reject such names outright at creation time,
        // unlike Linux's ext4 (what CI runs on) which allows arbitrary
        // bytes — skip rather than flake where it's rejected.
        let name = OsStr::from_bytes(b"bad_\xFF_name.dwl");
        if fs::write(dir.join(name), "x").is_err() {
            return;
        }

        let config = PickerConfig {
            program: fake_fzf_script(),
        };
        let result = pick_script(&config, &dir).unwrap();
        assert_eq!(result, Some(dir.join(name)));
    }

    #[test]
    fn pick_does_not_deadlock_or_error_when_picker_exits_before_reading_all_input() {
        // `head -n1` exits after its first line, so the parent's write
        // will hit a broken pipe partway through a large candidate list.
        // Before the fix (synchronous write-then-read on one thread),
        // large enough input risks a pipe-buffer deadlock; the broken
        // pipe from the early exit must also not surface as an Err.
        let config = PickerConfig {
            program: PathBuf::from("head"),
        };
        let candidates: Vec<String> = (0..5000)
            .map(|i| format!("candidate-{i}-{}", "x".repeat(100)))
            .collect();

        let result = pick(&config, &candidates);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }
}
