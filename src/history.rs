//! Records each `blazewvr watch` invocation to a small local history
//! file, and lets the user fuzzy-recall + re-run a past one via `fzf`.

// ponytail: not yet wired into a command (lands with Wave 3 CLI wiring);
// allowed dead here so this PR can ship history recording/recall
// standalone with full test coverage.
#![allow(dead_code)]

use crate::picker::{self, PickerConfig};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// One past `watch` invocation.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    pub script: PathBuf,
    pub inputs: Vec<(String, PathBuf)>,
    pub module_path: Option<String>,
    /// Unix seconds. Stored as a plain number rather than a formatted
    /// string so display formatting can change without invalidating
    /// already-recorded history.
    pub timestamp: u64,
}

impl HistoryEntry {
    pub fn now(
        script: PathBuf,
        inputs: Vec<(String, PathBuf)>,
        module_path: Option<String>,
    ) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            script,
            inputs,
            module_path,
            timestamp,
        }
    }
}

/// `$HOME/.local/state/blazewvr/history.jsonl`, or `None` if `$HOME`
/// isn't set.
pub fn default_history_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".local/state/blazewvr/history.jsonl"))
}

/// Loads history entries from `path`, oldest first. Missing file or
/// corrupt individual lines are treated as "no entry there" rather than
/// a hard error — history is a convenience, not something worth failing
/// a command over.
pub fn load(path: &Path) -> Vec<HistoryEntry> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Appends `entry` to the history at `path`, keeping only the most
/// recent `max_entries` (dropping the oldest first) so the file doesn't
/// grow unbounded.
pub fn append(path: &Path, entry: HistoryEntry, max_entries: usize) -> io::Result<()> {
    let mut entries = load(path);
    entries.push(entry);
    if entries.len() > max_entries {
        let excess = entries.len() - max_entries;
        entries.drain(0..excess);
    }

    let mut text = String::new();
    for e in &entries {
        let line = serde_json::to_string(e).map_err(io::Error::other)?;
        text.push_str(&line);
        text.push('\n');
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)
}

fn format_entry(e: &HistoryEntry) -> String {
    let inputs = if e.inputs.is_empty() {
        "no inputs".to_string()
    } else {
        e.inputs
            .iter()
            .map(|(name, file)| format!("{name}={}", file.display()))
            .collect::<Vec<_>>()
            .join(",")
    };
    format!("{} | {inputs} | {}", e.script.display(), e.timestamp)
}

/// Loads history from `path`, most-recent-first, and lets the user
/// fuzzy-pick one to re-run.
pub fn pick_history(picker_config: &PickerConfig, path: &Path) -> io::Result<Option<HistoryEntry>> {
    let mut entries = load(path);
    entries.reverse();

    let candidates: Vec<String> = entries.iter().map(format_entry).collect();
    match picker::pick(picker_config, &candidates)? {
        Some(selected) => Ok(entries.into_iter().find(|e| format_entry(e) == selected)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn test_history_path() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "blazewvr_history_{}_{id}.jsonl",
            std::process::id()
        ))
    }

    fn entry(script: &str, ts: u64) -> HistoryEntry {
        HistoryEntry {
            script: PathBuf::from(script),
            inputs: Vec::new(),
            module_path: None,
            timestamp: ts,
        }
    }

    #[test]
    fn now_stamps_a_recent_unix_timestamp() {
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let e = HistoryEntry::now(PathBuf::from("s.dwl"), Vec::new(), None);
        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!((before..=after).contains(&e.timestamp));
        assert_eq!(e.script, PathBuf::from("s.dwl"));
    }

    #[test]
    fn load_returns_empty_for_missing_file() {
        assert_eq!(load(&test_history_path()), Vec::new());
    }

    #[test]
    fn append_and_load_round_trips() {
        let path = test_history_path();
        append(&path, entry("a.dwl", 1), 10).unwrap();
        append(&path, entry("b.dwl", 2), 10).unwrap();
        assert_eq!(load(&path), vec![entry("a.dwl", 1), entry("b.dwl", 2)]);
    }

    #[test]
    fn append_preserves_inputs_and_module_path() {
        let path = test_history_path();
        let e = HistoryEntry {
            script: PathBuf::from("s.dwl"),
            inputs: vec![("payload".to_string(), PathBuf::from("p.json"))],
            module_path: Some("dir1:dir2".to_string()),
            timestamp: 5,
        };
        append(&path, e.clone(), 10).unwrap();
        assert_eq!(load(&path), vec![e]);
    }

    #[test]
    fn append_drops_oldest_entries_beyond_max() {
        let path = test_history_path();
        for i in 0..5 {
            append(&path, entry(&format!("{i}.dwl"), i), 3).unwrap();
        }
        let loaded = load(&path);
        let scripts: Vec<String> = loaded
            .iter()
            .map(|e| e.script.display().to_string())
            .collect();
        assert_eq!(scripts, vec!["2.dwl", "3.dwl", "4.dwl"]);
    }

    #[test]
    fn load_skips_corrupt_lines() {
        let path = test_history_path();
        append(&path, entry("a.dwl", 1), 10).unwrap();
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("not valid json\n");
        fs::write(&path, &text).unwrap();
        append(&path, entry("b.dwl", 2), 10).unwrap();

        // The manually-injected corrupt line predates the second append,
        // which reads-modifies-writes the whole file; load() must not
        // choke on it if it somehow persists, and the two real entries
        // must both still be present.
        let loaded = load(&path);
        assert!(loaded.iter().any(|e| e.script == Path::new("a.dwl")));
        assert!(loaded.iter().any(|e| e.script == Path::new("b.dwl")));
    }

    #[test]
    fn default_history_path_uses_home() {
        let path = default_history_path();
        // $HOME is set in essentially every real environment (including
        // CI); only assert the shape when it resolves.
        if let Some(path) = path {
            assert!(path.ends_with(".local/state/blazewvr/history.jsonl"));
        }
    }

    /// Same echo-protocol fake fzf as picker.rs's tests: prints the
    /// candidate matching `--select=<substring>`, or the first line.
    fn fake_fzf_script() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "blazewvr_history_fake_fzf_{}_{id}.sh",
            std::process::id()
        ));
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
    fn pick_history_returns_none_for_empty_history() {
        let path = test_history_path();
        let config = PickerConfig {
            program: fake_fzf_script(),
            extra_args: Vec::new(),
        };
        assert_eq!(pick_history(&config, &path).unwrap(), None);
    }

    #[test]
    fn pick_history_defaults_to_most_recent_first() {
        let path = test_history_path();
        append(&path, entry("old.dwl", 1), 10).unwrap();
        append(&path, entry("new.dwl", 2), 10).unwrap();

        // No --select given -> fake fzf's `head -n1` picks whichever
        // candidate is listed first, which must be the most recent.
        let config = PickerConfig {
            program: fake_fzf_script(),
            extra_args: Vec::new(),
        };
        let picked = pick_history(&config, &path).unwrap().unwrap();
        assert_eq!(picked.script, PathBuf::from("new.dwl"));
    }

    #[test]
    fn pick_history_formats_inputs_in_the_candidate_line() {
        let path = test_history_path();
        let with_inputs = HistoryEntry {
            script: PathBuf::from("s.dwl"),
            inputs: vec![("payload".to_string(), PathBuf::from("p.json"))],
            module_path: None,
            timestamp: 1,
        };
        append(&path, with_inputs.clone(), 10).unwrap();

        let config = PickerConfig {
            program: fake_fzf_script(),
            extra_args: Vec::new(),
        };
        let picked = pick_history(&config, &path).unwrap().unwrap();
        assert_eq!(picked, with_inputs);
    }

    #[test]
    fn pick_history_returns_selected_entry() {
        let path = test_history_path();
        append(&path, entry("old.dwl", 1), 10).unwrap();
        append(&path, entry("new.dwl", 2), 10).unwrap();

        let config = PickerConfig {
            program: fake_fzf_script(),
            extra_args: vec!["--select=old.dwl".to_string()],
        };
        let picked = pick_history(&config, &path).unwrap().unwrap();
        assert_eq!(picked.script, PathBuf::from("old.dwl"));
    }
}
