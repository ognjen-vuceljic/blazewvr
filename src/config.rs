//! `blazewvr.toml` project config: module resolution path and default
//! named inputs, so a playground project can be `cd`'d into and run
//! without repeating flags every time. CLI flags (Wave 1) always take
//! precedence over the config file when both are given.
//!
//! ponytail: deliberately does NOT include an "output format override"
//! (mentioned when this issue was first scoped) — `dw` has no flag to
//! force a script's output MIME, only the script's own `output`
//! directive decides it, so a config field for that would have no
//! engine mechanism behind it. Add it if/when `dw` actually grows one.

// ponytail: not yet wired into a command (lands with Wave 2 CLI wiring);
// allowed dead here so this PR can ship config loading standalone with
// full test coverage.
#![allow(dead_code)]

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "blazewvr.toml";

#[derive(Deserialize, Default)]
struct RawConfig {
    module_path: Option<String>,
    #[serde(default)]
    inputs: BTreeMap<String, String>,
}

/// Parsed `blazewvr.toml`: module resolution path and default inputs.
pub struct ProjectConfig {
    pub module_path: Option<String>,
    pub inputs: Vec<(String, PathBuf)>,
}

/// Loads `blazewvr.toml` from `dir`, if present. Returns `Ok(None)` when
/// no config file exists (not an error — it's optional), and `Err` for an
/// unreadable file or invalid TOML.
pub fn load(dir: &Path) -> io::Result<Option<ProjectConfig>> {
    let path = dir.join(FILE_NAME);
    if !path.exists() {
        return Ok(None);
    }

    let text = fs::read_to_string(&path)?;
    let raw: RawConfig = toml::from_str(&text)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let inputs = raw
        .inputs
        .into_iter()
        .map(|(name, file)| (name, PathBuf::from(file)))
        .collect();
    Ok(Some(ProjectConfig {
        module_path: raw.module_path,
        inputs,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn test_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("blazewvr_config_{}_{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    #[test]
    fn returns_none_when_no_config_file_exists() {
        let dir = test_dir();
        assert!(load(&dir).unwrap().is_none());
    }

    #[test]
    fn parses_module_path_and_inputs() {
        let dir = test_dir();
        fs::write(
            dir.join(FILE_NAME),
            r#"
module_path = "dir1:dir2"

[inputs]
payload = "data/payload.json"
headers = "data/headers.json"
"#,
        )
        .unwrap();

        let config = load(&dir).unwrap().unwrap();
        assert_eq!(config.module_path, Some("dir1:dir2".to_string()));
        assert_eq!(
            config.inputs,
            vec![
                ("headers".to_string(), PathBuf::from("data/headers.json")),
                ("payload".to_string(), PathBuf::from("data/payload.json")),
            ]
        );
    }

    #[test]
    fn works_with_no_module_path_or_inputs() {
        let dir = test_dir();
        fs::write(dir.join(FILE_NAME), "").unwrap();

        let config = load(&dir).unwrap().unwrap();
        assert_eq!(config.module_path, None);
        assert!(config.inputs.is_empty());
    }

    #[test]
    fn returns_err_for_invalid_toml() {
        let dir = test_dir();
        fs::write(dir.join(FILE_NAME), "this is not valid toml [[[").unwrap();
        assert!(load(&dir).is_err());
    }
}
