//! Sidecar input auto-discovery: for `script.dwl`, find files named
//! `script.<name>.<ext>` in the same directory and bind them as inputs
//! named `<name>`, so the common case needs zero `-i` flags. Explicit
//! `-i` flags (Wave 1) always take precedence when given — callers only
//! invoke this when no inputs were passed on the command line.

use std::fs;
use std::path::{Path, PathBuf};

/// The directory to search for `script`'s sidecars in.
///
/// `Path::parent()` on a bare filename like "orders.dwl" returns
/// `Some("")`, not `None` — an empty path fails `read_dir` outright
/// (`ENOENT`), which silently looked like "no sidecars found" for the
/// single most common invocation: `blazewvr watch script.dwl` run from
/// the script's own directory. Treat an empty (or absent) parent as `.`
/// instead.
fn sidecar_dir(script: &Path) -> &Path {
    match script.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

/// Finds sidecar input files next to `script` and returns them as
/// `(name, file)` pairs, sorted by name for deterministic ordering.
pub fn discover(script: &Path) -> Vec<(String, PathBuf)> {
    let Some(stem) = script.file_stem().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    let dir = sidecar_dir(script);
    let prefix = format!("{stem}.");

    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut found: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path == script {
                return None;
            }
            let fname = path.file_name()?.to_str()?;
            let rest = fname.strip_prefix(&prefix)?;
            let (name, _ext) = rest.split_once('.')?;
            if name.is_empty() {
                return None;
            }
            Some((name.to_string(), path))
        })
        .collect();

    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn test_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("blazewvr_sidecar_{}_{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    #[test]
    fn finds_a_single_sidecar_input() {
        let dir = test_dir();
        let script = dir.join("foo.dwl");
        fs::write(&script, "payload").unwrap();
        fs::write(dir.join("foo.payload.json"), "{}").unwrap();

        let found = discover(&script);
        assert_eq!(
            found,
            vec![("payload".to_string(), dir.join("foo.payload.json"))]
        );
    }

    #[test]
    fn sidecar_dir_of_a_bare_filename_is_cwd_not_empty() {
        assert_eq!(sidecar_dir(Path::new("orders.dwl")), Path::new("."));
    }

    #[test]
    fn sidecar_dir_of_a_qualified_path_is_its_parent() {
        assert_eq!(sidecar_dir(Path::new("a/b/orders.dwl")), Path::new("a/b"));
    }

    #[test]
    fn finds_sidecar_for_a_bare_filename_script_in_the_cwd() {
        let dir = test_dir();
        fs::write(dir.join("foo.dwl"), "payload").unwrap();
        fs::write(dir.join("foo.payload.json"), "{}").unwrap();

        let original_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let found = discover(Path::new("foo.dwl"));
        std::env::set_current_dir(original_cwd).unwrap();

        assert_eq!(
            found,
            vec![("payload".to_string(), PathBuf::from("./foo.payload.json"))]
        );
    }

    #[test]
    fn finds_multiple_sidecars_sorted_by_name() {
        let dir = test_dir();
        let script = dir.join("foo.dwl");
        fs::write(&script, "x").unwrap();
        fs::write(dir.join("foo.zeta.json"), "{}").unwrap();
        fs::write(dir.join("foo.alpha.xml"), "<a/>").unwrap();

        let found = discover(&script);
        assert_eq!(
            found,
            vec![
                ("alpha".to_string(), dir.join("foo.alpha.xml")),
                ("zeta".to_string(), dir.join("foo.zeta.json")),
            ]
        );
    }

    #[test]
    fn ignores_unrelated_files_in_the_same_directory() {
        let dir = test_dir();
        let script = dir.join("foo.dwl");
        fs::write(&script, "x").unwrap();
        fs::write(dir.join("bar.payload.json"), "{}").unwrap();
        fs::write(dir.join("README.md"), "hi").unwrap();

        assert_eq!(discover(&script), vec![]);
    }

    #[test]
    fn ignores_the_script_file_itself() {
        let dir = test_dir();
        let script = dir.join("foo.dwl");
        fs::write(&script, "x").unwrap();

        assert_eq!(discover(&script), vec![]);
    }

    #[test]
    fn ignores_sidecar_candidate_with_no_extension() {
        let dir = test_dir();
        let script = dir.join("foo.dwl");
        fs::write(&script, "x").unwrap();
        // "foo.payload" has no second '.' after the prefix, so no name/ext split
        fs::write(dir.join("foo.payload"), "{}").unwrap();

        assert_eq!(discover(&script), vec![]);
    }

    #[test]
    fn returns_empty_for_nonexistent_directory() {
        let missing = PathBuf::from("/nonexistent/blazewvr/dir/foo.dwl");
        assert_eq!(discover(&missing), vec![]);
    }
}
