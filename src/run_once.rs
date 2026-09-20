//! Spawn-per-eval fallback: runs `dw run` once per evaluation instead of
//! driving a persistent `dw repl` child. Slower (~360ms/eval measured for
//! `dw run`, vs. near-zero for an already-warm `dw repl`), but simpler and
//! more robust — for cases the flattening/framing pipeline in repl.rs
//! can't handle cleanly (multi-doc scripts, --privileges, or REPL edge
//! cases). The script is written to a temp file untouched (no flattening
//! needed, since `dw run -f` reads a real multi-line file directly).

// ponytail: not yet wired into automatic fast-path-failure detection —
// that policy (when exactly to fall back) is left for when a real
// flattening/framing failure is observed, rather than guessed at now.
#![allow(dead_code)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// How to invoke `dw run` for one evaluation.
pub struct RunOnceConfig {
    pub program: PathBuf,
    pub inputs: Vec<(String, PathBuf)>,
    pub module_path: Option<String>,
}

impl RunOnceConfig {
    /// Builds the config for the real `dw run`.
    pub fn for_dw(dw_path: &Path, inputs: &[(String, PathBuf)], module_path: Option<&str>) -> Self {
        Self {
            program: dw_path.to_path_buf(),
            inputs: inputs.to_vec(),
            module_path: module_path.map(str::to_string),
        }
    }

    fn args(&self, script_file: &Path) -> Vec<String> {
        let mut args = vec![
            "run".to_string(),
            "-s".to_string(),
            "-f".to_string(),
            script_file.display().to_string(),
        ];
        for (name, file) in &self.inputs {
            args.push("-i".to_string());
            args.push(format!("{name}={}", file.display()));
        }
        if let Some(p) = &self.module_path {
            args.push(format!("--path={p}"));
        }
        args
    }
}

/// Writes `script_src` to a temp file and runs it via `dw run -f`,
/// returning stdout on success or stderr text as the error on failure.
pub fn eval_once(config: &RunOnceConfig, script_src: &str) -> io::Result<String> {
    let id = TMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    let tmp_file =
        std::env::temp_dir().join(format!("blazewvr_run_once_{}_{id}.dwl", std::process::id()));
    fs::write(&tmp_file, script_src)?;

    let result = Command::new(&config.program)
        .args(config.args(&tmp_file))
        .output();
    let _ = fs::remove_file(&tmp_file);
    let output = result?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering as StdOrdering};

    static SCRIPT_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Fake `dw run` that reads the file passed via `-f` and either echoes
    /// its content (prefixed) or, for the sentinel content "FAIL", exits
    /// non-zero with a message on stderr.
    fn fake_dw_run() -> PathBuf {
        let id = SCRIPT_COUNTER.fetch_add(1, StdOrdering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "blazewvr_fake_dw_run_{}_{id}.sh",
            std::process::id()
        ));
        let script = r#"#!/bin/sh
file=""
while [ $# -gt 0 ]; do
  case "$1" in
    -f) file="$2"; shift 2 ;;
    *) shift ;;
  esac
done
content=$(cat "$file")
if [ "$content" = "FAIL" ]; then
  echo "boom" 1>&2
  exit 1
fi
echo "RAN:$content"
"#;
        fs::write(&path, script).expect("write fake dw run script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    fn fake_config(inputs: &[(String, PathBuf)]) -> RunOnceConfig {
        RunOnceConfig {
            program: fake_dw_run(),
            inputs: inputs.to_vec(),
            module_path: None,
        }
    }

    #[test]
    fn eval_once_returns_stdout_on_success() {
        let config = fake_config(&[]);
        let result = eval_once(&config, "hello").unwrap();
        assert_eq!(result, "RAN:hello");
    }

    #[test]
    fn eval_once_returns_stderr_as_error_on_failure() {
        let config = fake_config(&[]);
        let err = eval_once(&config, "FAIL").unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn for_dw_builds_run_args_with_inputs_and_path() {
        let inputs = vec![("payload".to_string(), PathBuf::from("/tmp/p.json"))];
        let config = RunOnceConfig::for_dw(Path::new("dw"), &inputs, Some("dir1:dir2"));
        let args = config.args(Path::new("/tmp/script.dwl"));
        assert_eq!(
            args,
            vec![
                "run",
                "-s",
                "-f",
                "/tmp/script.dwl",
                "-i",
                "payload=/tmp/p.json",
                "--path=dir1:dir2"
            ]
        );
    }

    #[test]
    fn for_dw_with_no_inputs_or_path() {
        let config = RunOnceConfig::for_dw(Path::new("dw"), &[], None);
        let args = config.args(Path::new("/tmp/script.dwl"));
        assert_eq!(args, vec!["run", "-s", "-f", "/tmp/script.dwl"]);
    }
}
