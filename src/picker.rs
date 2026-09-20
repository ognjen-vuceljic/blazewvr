//! fzf-backed fuzzy picking. `blazewvr` doesn't hard-depend on `fzf` being
//! installed — callers check `is_available()` first and fall back to
//! requiring an explicit argument with a clear error when it's missing.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Which picker binary to invoke.
pub struct PickerConfig {
    pub program: PathBuf,
    /// Extra args appended to every invocation, to make the fake picker
    /// script deterministic in tests without resorting to process-global
    /// env vars (which would race under parallel test execution).
    /// `#[cfg(test)]`-gated (rather than just documented as test-only) so
    /// it cannot exist as a field on the type real callers construct,
    /// ruling out any future mix-up with `run_picker`'s own internal
    /// `extra_flags` mechanism (used for `-m`).
    #[cfg(test)]
    pub extra_args: Vec<String>,
}

impl Default for PickerConfig {
    #[cfg(test)]
    fn default() -> Self {
        Self {
            program: PathBuf::from("fzf"),
            extra_args: Vec::new(),
        }
    }

    #[cfg(not(test))]
    fn default() -> Self {
        Self {
            program: PathBuf::from("fzf"),
        }
    }
}

/// How many times a transient spawn failure (see `spawn_retrying`) is
/// retried before giving up, and how long to wait between attempts.
///
/// Two immediate, back-to-back attempts (the original mitigation) turned
/// out not to be enough: real CI runs hit `ExecutableFileBusy` on *both*
/// attempts, on more than one test, in the same run (2026-09-20, PR #38).
/// An immediate retry doesn't actually wait for whatever transient
/// condition (page-cache writeback lag, a security scanner briefly
/// holding the file open — never reproduced locally to confirm which)
/// caused the busy state to clear. A short sleep between attempts gives
/// it a chance to.
const SPAWN_RETRY_ATTEMPTS: u32 = 4;
const SPAWN_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(20);

/// True if `program` can be executed at all (used to check `fzf` is on
/// `PATH` before relying on it). See `SPAWN_RETRY_ATTEMPTS`'s doc comment
/// for why this retries with a delay rather than just once.
pub fn is_available(program: &Path) -> bool {
    for attempt in 0..SPAWN_RETRY_ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(SPAWN_RETRY_DELAY);
        }
        let ok = Command::new(program)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok();
        if ok {
            return true;
        }
    }
    false
}

/// Runs `attempt`, retrying (with a short delay between attempts — see
/// `SPAWN_RETRY_ATTEMPTS`'s doc comment) on failure.
///
/// The same transient-spawn-failure class documented on `is_available`
/// above was observed hitting the *actual* picker spawn too — i.e. a
/// real `pick`/`pick_multi`/`pick_history` call, not just the
/// `is_available` probe that precedes it. Retrying there didn't help,
/// since the failure showed up moments later in this separate spawn.
/// Takes a closure rather than `&mut Command` directly so the retry
/// itself is testable with a deterministic fail-then-succeed stub.
fn spawn_retrying(mut attempt: impl FnMut() -> io::Result<Child>) -> io::Result<Child> {
    let mut last_err = None;
    for i in 0..SPAWN_RETRY_ATTEMPTS {
        if i > 0 {
            std::thread::sleep(SPAWN_RETRY_DELAY);
        }
        match attempt() {
            Ok(child) => return Ok(child),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.expect("SPAWN_RETRY_ATTEMPTS > 0, so at least one attempt ran"))
}

/// Spawns the picker with `extra_flags` appended before `config`'s own
/// `extra_args`, feeds it `candidates` on stdin, and returns its raw
/// stdout text.
///
/// Writes to the child's stdin on a separate thread while reading its
/// stdout on this one. Writing and reading sequentially on one thread
/// deadlocks once either pipe's OS buffer fills (parent blocked writing
/// while the child blocks writing its own output that nobody's reading
/// yet) — a real risk here since candidate lists can be large. A broken
/// pipe from the writer (the picker selecting and exiting before it has
/// consumed all input — the common case, not an edge case) is not
/// treated as an error.
fn run_picker(
    config: &PickerConfig,
    extra_flags: &[&str],
    candidates: &[String],
) -> io::Result<String> {
    let mut cmd = Command::new(&config.program);
    cmd.args(extra_flags);
    #[cfg(test)]
    cmd.args(&config.extra_args);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = spawn_retrying(|| cmd.spawn())?;
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
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Pipes `candidates` into the picker and returns the selected line, or
/// `None` if nothing was selected (empty list, or the user backed out).
pub fn pick(config: &PickerConfig, candidates: &[String]) -> io::Result<Option<String>> {
    if candidates.is_empty() {
        return Ok(None);
    }
    let selection = run_picker(config, &[], candidates)?.trim().to_string();
    if selection.is_empty() {
        Ok(None)
    } else {
        Ok(Some(selection))
    }
}

/// Like `pick`, but allows selecting multiple candidates (`fzf -m`),
/// returning each selected line in order.
pub fn pick_multi(config: &PickerConfig, candidates: &[String]) -> io::Result<Vec<String>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let text = run_picker(config, &["-m"], candidates)?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

pub(crate) const DATA_EXTENSIONS: &[&str] = &["json", "xml", "csv", "yaml", "yml", "txt"];

/// Recursively finds `.dwl` files under `root`. See `find_files` for the
/// shared traversal rules (symlinks, hidden dirs, `target/`, unreadable
/// dirs).
pub fn find_dwl_scripts(root: &Path) -> Vec<PathBuf> {
    find_files(root, |p| {
        p.extension().and_then(|e| e.to_str()) == Some("dwl")
    })
}

/// Recursively finds likely data files (json/xml/csv/yaml/txt, matched
/// case-insensitively so e.g. `payload.JSON` from a Windows tool or a
/// manual rename isn't silently invisible) under `root`, excluding
/// `exclude` (typically the script being run — it wouldn't make sense to
/// bind it as its own input).
pub fn find_data_files(root: &Path, exclude: &Path) -> Vec<PathBuf> {
    find_files(root, |p| {
        p != exclude
            && p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| DATA_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
    })
}

/// Recursively finds files under `root` matching `matches`, skipping
/// hidden directories, `target/`, and symlinks (both to avoid following a
/// symlink cycle into unbounded recursion, and because resolving them
/// correctly is more than this needs). Directories that can't be read
/// are skipped with a warning on stderr rather than silently, so a
/// truncated result isn't mistaken for a genuinely empty one.
fn find_files(root: &Path, matches: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut results = Vec::new();
    walk(root, &mut results, &matches);
    results.sort();
    results
}

fn walk(dir: &Path, results: &mut Vec<PathBuf>, matches: &impl Fn(&Path) -> bool) {
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
            walk(&path, results, matches);
        } else if file_type.is_file() && matches(&path) {
            results.push(path);
        }
    }
}

/// Builds the candidate strings to feed the picker plus a lookup from
/// each one back to its original `PathBuf`, built once so callers with
/// multiple selections (`bind_inputs_via_picker`) don't re-stringify
/// every candidate per selection.
///
/// Matching a selection back to its `PathBuf` this way (rather than by
/// re-parsing the picker's text output into a path) matters because
/// candidates are sent through `path.display()`, which is lossy for
/// non-UTF-8 filenames — re-parsing would silently produce a different,
/// nonexistent path for any such name.
fn candidate_index(paths: &[PathBuf]) -> (Vec<String>, std::collections::HashMap<String, PathBuf>) {
    let mut strs = Vec::with_capacity(paths.len());
    let mut by_display = std::collections::HashMap::with_capacity(paths.len());
    for p in paths {
        let s = p.display().to_string();
        strs.push(s.clone());
        by_display.insert(s, p.clone());
    }
    (strs, by_display)
}

/// Finds `.dwl` scripts under `root` and lets the user fuzzy-pick one.
/// Errs clearly if `fzf` isn't available, rather than silently doing
/// nothing.
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

    let (candidates, by_display) = candidate_index(&find_dwl_scripts(root));
    match pick(config, &candidates)? {
        Some(selected) => Ok(by_display.get(&selected).cloned()),
        None => Ok(None),
    }
}

/// Fuzzy-multi-picks data files under `root` (excluding `script`) and
/// binds each selected file to an input name, via `name_for` — called
/// once per selected file with that file's stem as a default, returning
/// the name to bind it as (an empty/whitespace-only response falls back
/// to the default). Injectable so the real "prompt the user on stdin"
/// behavior and the fully-scripted test behavior share this same
/// resolution logic.
///
/// If two selected files would otherwise bind to the same name (e.g.
/// `sub_a/payload.json` and `sub_b/payload.csv` both default-naming to
/// "payload"), later collisions get a numeric suffix (`payload_2`) so
/// one binding doesn't silently shadow another — this only disambiguates
/// the common case of repeated collisions on the same base name, not
/// every possible pathological naming scheme.
pub fn bind_inputs_via_picker(
    config: &PickerConfig,
    script: &Path,
    root: &Path,
    mut name_for: impl FnMut(&str) -> io::Result<String>,
) -> io::Result<Vec<(String, PathBuf)>> {
    if !is_available(&config.program) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "{} not found on PATH; use -i/sidecar/config inputs instead",
                config.program.display()
            ),
        ));
    }

    let (candidate_strs, by_display) = candidate_index(&find_data_files(root, script));
    let selected = pick_multi(config, &candidate_strs)?;

    let mut bound = Vec::new();
    let mut name_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for selection in selected {
        let Some(path) = by_display.get(&selection) else {
            continue;
        };
        let default_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("input")
            .to_string();
        let name = name_for(&default_name)?;
        let base_name = if name.trim().is_empty() {
            default_name
        } else {
            name.trim().to_string()
        };

        let count = name_counts.entry(base_name.clone()).or_insert(0);
        *count += 1;
        let name = if *count > 1 {
            format!("{base_name}_{count}")
        } else {
            base_name
        };

        bound.push((name, path.clone()));
    }
    Ok(bound)
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

    #[test]
    fn spawn_retrying_succeeds_immediately_when_first_attempt_works() {
        let mut attempts = 0;
        let result = spawn_retrying(|| {
            attempts += 1;
            Command::new(fake_fzf_script()).spawn()
        });
        assert!(result.is_ok());
        assert_eq!(
            attempts, 1,
            "should not retry when the first attempt succeeds"
        );
    }

    #[test]
    fn spawn_retrying_propagates_error_when_all_attempts_fail() {
        let mut attempts = 0;
        let result = spawn_retrying(|| {
            attempts += 1;
            Command::new("/nonexistent/blazewvr/nope").spawn()
        });
        assert!(result.is_err());
        assert_eq!(attempts, SPAWN_RETRY_ATTEMPTS);
    }

    #[test]
    fn spawn_retrying_succeeds_after_repeated_transient_failures() {
        // Reproduces the observed CI failure shape: a given command fails
        // to spawn a few times in a row, but the exact same command
        // succeeds moments later — including the two-failures-in-a-row
        // case that a plain single retry wasn't enough to cover.
        let mut attempts = 0;
        let result = spawn_retrying(|| {
            attempts += 1;
            if attempts <= 2 {
                Err(io::Error::other("transient spawn failure"))
            } else {
                Command::new(fake_fzf_script()).spawn()
            }
        });
        assert!(result.is_ok());
        assert_eq!(attempts, 3);
    }

    /// Fake fzf: reads candidates from stdin, prints the ones matching a
    /// `--select=<substring>` arg (grep, so possibly more than one line —
    /// good enough to simulate multi-select), or just the first line if
    /// no `--select` was given. `--select` is argv-based (not an env var)
    /// specifically so it's per-Command and safe under parallel tests.
    fn fake_fzf_script() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("blazewvr_fake_fzf_{}_{id}.sh", std::process::id()));
        let script = r#"#!/bin/sh
select=""
for arg in "$@"; do
  case "$arg" in
    --version) echo "fake-fzf 0.0.0"; exit 0 ;;
    --select=*) select="${arg#--select=}" ;;
  esac
done
if [ -n "$select" ]; then
  grep -F "$select"
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
            ..Default::default()
        };
        assert_eq!(pick(&config, &[]).unwrap(), None);
    }

    #[test]
    fn pick_returns_first_candidate_by_default() {
        let config = PickerConfig {
            program: fake_fzf_script(),
            ..Default::default()
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
            ..Default::default()
        };
        let dir = test_dir();
        let result = pick_script(&config, &dir);
        assert!(result.is_err());
    }

    #[test]
    fn pick_script_returns_none_when_no_scripts_found() {
        let config = PickerConfig {
            program: fake_fzf_script(),
            ..Default::default()
        };
        let dir = test_dir();
        assert_eq!(pick_script(&config, &dir).unwrap(), None);
    }

    #[test]
    fn pick_script_returns_selected_path() {
        let config = PickerConfig {
            program: fake_fzf_script(),
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
        };
        let candidates: Vec<String> = (0..5000)
            .map(|i| format!("candidate-{i}-{}", "x".repeat(100)))
            .collect();

        let result = pick(&config, &candidates);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    #[test]
    fn pick_multi_returns_empty_for_empty_candidates() {
        let config = PickerConfig {
            program: fake_fzf_script(),
            ..Default::default()
        };
        assert_eq!(pick_multi(&config, &[]).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn pick_multi_returns_multiple_matching_selections() {
        let config = PickerConfig {
            program: fake_fzf_script(),
            extra_args: vec!["--select=data".to_string()],
        };
        let candidates = vec![
            "a.data.json".to_string(),
            "b.data.xml".to_string(),
            "c.other.txt".to_string(),
        ];
        let result = pick_multi(&config, &candidates).unwrap();
        assert_eq!(
            result,
            vec!["a.data.json".to_string(), "b.data.xml".to_string()]
        );
    }

    #[test]
    fn find_data_files_matches_known_extensions_and_excludes_given_path() {
        let dir = test_dir();
        let script = dir.join("script.dwl");
        fs::write(&script, "x").unwrap();
        fs::write(dir.join("payload.json"), "{}").unwrap();
        fs::write(dir.join("headers.xml"), "<a/>").unwrap();
        fs::write(dir.join("notes.md"), "hi").unwrap();
        // A .json file that happens to equal `script`'s own extension
        // pattern isn't special-cased — only exact path equality excludes.
        fs::write(dir.join("other.dwl"), "y").unwrap();

        let found = find_data_files(&dir, &script);
        assert_eq!(
            found,
            vec![dir.join("headers.xml"), dir.join("payload.json")]
        );
    }

    #[test]
    fn find_data_files_matches_extensions_case_insensitively() {
        let dir = test_dir();
        let script = dir.join("script.dwl");
        fs::write(&script, "x").unwrap();
        fs::write(dir.join("payload.JSON"), "{}").unwrap();
        fs::write(dir.join("export.Csv"), "a,b").unwrap();

        let found = find_data_files(&dir, &script);
        assert_eq!(
            found,
            vec![dir.join("export.Csv"), dir.join("payload.JSON")]
        );
    }

    #[test]
    fn bind_inputs_via_picker_disambiguates_colliding_default_names() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "x").unwrap();
        fs::create_dir_all(dir.join("sub_a")).unwrap();
        fs::create_dir_all(dir.join("sub_b")).unwrap();
        fs::write(dir.join("sub_a/payload.json"), "{}").unwrap();
        fs::write(dir.join("sub_b/payload.csv"), "a").unwrap();

        // Both files default-name to "payload"; select both via a
        // substring common to both paths.
        let config = PickerConfig {
            program: fake_fzf_script(),
            extra_args: vec!["--select=payload".to_string()],
        };
        let mut result =
            bind_inputs_via_picker(&config, &script, &dir, |default| Ok(default.to_string()))
                .unwrap();
        result.sort();

        let names: Vec<&str> = result.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names.len(), 2, "expected two bound inputs, got {names:?}");
        assert!(
            names.contains(&"payload"),
            "first collision keeps the base name: {names:?}"
        );
        assert!(
            names.contains(&"payload_2"),
            "second collision gets a numeric suffix: {names:?}"
        );
    }

    #[test]
    fn bind_inputs_via_picker_errs_when_fzf_unavailable() {
        let config = PickerConfig {
            program: PathBuf::from("/nonexistent/blazewvr/nope"),
            ..Default::default()
        };
        let dir = test_dir();
        let script = dir.join("s.dwl");
        let result =
            bind_inputs_via_picker(&config, &script, &dir, |default| Ok(default.to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn bind_inputs_via_picker_binds_selected_files_with_default_names() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "x").unwrap();
        fs::write(dir.join("payload.json"), "{}").unwrap();

        let config = PickerConfig {
            program: fake_fzf_script(),
            extra_args: vec!["--select=payload".to_string()],
        };
        let result =
            bind_inputs_via_picker(&config, &script, &dir, |default| Ok(default.to_string()))
                .unwrap();
        assert_eq!(
            result,
            vec![("payload".to_string(), dir.join("payload.json"))]
        );
    }

    #[test]
    fn bind_inputs_via_picker_uses_custom_name_over_default() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "x").unwrap();
        fs::write(dir.join("payload.json"), "{}").unwrap();

        let config = PickerConfig {
            program: fake_fzf_script(),
            extra_args: vec!["--select=payload".to_string()],
        };
        let result =
            bind_inputs_via_picker(&config, &script, &dir, |_default| Ok("custom".to_string()))
                .unwrap();
        assert_eq!(
            result,
            vec![("custom".to_string(), dir.join("payload.json"))]
        );
    }

    #[test]
    fn bind_inputs_via_picker_falls_back_to_default_on_blank_name() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "x").unwrap();
        fs::write(dir.join("payload.json"), "{}").unwrap();

        let config = PickerConfig {
            program: fake_fzf_script(),
            extra_args: vec!["--select=payload".to_string()],
        };
        let result =
            bind_inputs_via_picker(&config, &script, &dir, |_default| Ok("   ".to_string()))
                .unwrap();
        assert_eq!(
            result,
            vec![("payload".to_string(), dir.join("payload.json"))]
        );
    }

    #[test]
    fn bind_inputs_via_picker_returns_empty_when_nothing_selected() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "x").unwrap();

        let config = PickerConfig {
            program: fake_fzf_script(),
            ..Default::default()
        };
        let result =
            bind_inputs_via_picker(&config, &script, &dir, |default| Ok(default.to_string()))
                .unwrap();
        assert_eq!(result, Vec::new());
    }
}
