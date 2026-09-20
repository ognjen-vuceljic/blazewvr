//! Supervises a persistent `dw repl` child process: one spawn instead of
//! one spawn per eval (~360ms per `dw run` invocation measured, too slow
//! for live feedback), with transparent restart if the child dies.

// ponytail: not yet wired into a command (lands with watch mode, #3);
// allowed dead here so this PR can ship the supervisor standalone with
// full test coverage against a fake REPL script.
#![allow(dead_code)]

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread;

const PROMPT: &[u8] = b">>> ";

/// How to launch the REPL child process.
pub struct ReplConfig {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl ReplConfig {
    /// Builds the config for launching the real `dw repl` with the given
    /// named inputs (`-i name=file`).
    pub fn for_dw(dw_path: &Path, inputs: &[(String, PathBuf)]) -> Self {
        let mut args = vec!["repl".to_string(), "-s".to_string()];
        for (name, file) in inputs {
            args.push("-i".to_string());
            args.push(format!("{name}={}", file.display()));
        }
        Self {
            program: dw_path.to_path_buf(),
            args,
        }
    }
}

/// A single, live `dw repl` child process.
pub struct Repl {
    child: Child,
    stdout: BufReader<Box<dyn Read + Send>>,
    stdin: ChildStdin,
}

impl Repl {
    /// Spawns the child and consumes its startup banner + initial prompt.
    pub fn spawn(config: &ReplConfig) -> io::Result<Self> {
        let mut child = Command::new(&config.program)
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // Drain stderr on a background thread so JVM/netty warnings can't
        // fill the pipe buffer and block the child.
        if let Some(stderr) = child.stderr.take() {
            thread::spawn(move || {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 0 {
                    line.clear();
                }
            });
        }

        let stdout: Box<dyn Read + Send> = Box::new(
            child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("child stdout not piped"))?,
        );
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("child stdin not piped"))?;

        let mut repl = Repl {
            child,
            stdout: BufReader::new(stdout),
            stdin,
        };
        repl.read_until_prompt()?; // consume banner + first prompt
        Ok(repl)
    }

    /// Sends one flattened document and returns the text printed before
    /// the next prompt.
    pub fn eval(&mut self, doc: &str) -> io::Result<String> {
        writeln!(self.stdin, "{doc}")?;
        self.stdin.flush()?;
        self.read_until_prompt()
    }

    /// True if the child hasn't exited.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn read_until_prompt(&mut self) -> io::Result<String> {
        let mut acc: Vec<u8> = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = self.stdout.read(&mut byte)?;
            if n == 0 {
                break; // EOF: child closed stdout
            }
            acc.push(byte[0]);
            if acc.ends_with(PROMPT) {
                acc.truncate(acc.len() - PROMPT.len());
                break;
            }
        }
        Ok(String::from_utf8_lossy(&acc).trim().to_string())
    }
}

impl Drop for Repl {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Owns a `Repl`, respawning it transparently if the child has died.
pub struct Supervisor {
    config: ReplConfig,
    repl: Option<Repl>,
}

impl Supervisor {
    pub fn new(config: ReplConfig) -> Self {
        Self { config, repl: None }
    }

    /// Evaluates `doc`, spawning the child on first use and transparently
    /// respawning once if the existing child has died or errors mid-eval.
    pub fn eval(&mut self, doc: &str) -> io::Result<String> {
        if self.repl.is_none() {
            self.repl = Some(Repl::spawn(&self.config)?);
        }
        match self.repl.as_mut().expect("just ensured Some").eval(doc) {
            Ok(result) => Ok(result),
            Err(err) => {
                self.repl = Some(Repl::spawn(&self.config)?);
                self.repl
                    .as_mut()
                    .expect("just spawned")
                    .eval(doc)
                    .map_err(|_| err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Writes a tiny shell script that mimics `dw repl`'s prompt protocol:
    /// prints a banner + prompt, then for each input line either echoes
    /// it back (followed by the next prompt) or exits, simulating a crash.
    fn fake_repl_script() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("blazewvr_fake_repl_{}_{id}.sh", std::process::id()));
        let script = "#!/bin/sh\nprintf 'FAKE REPL\\n>>> '\nwhile IFS= read -r line; do\n  if [ \"$line\" = CRASH ]; then\n    exit 1\n  fi\n  printf 'ECHO:%s\\n>>> ' \"$line\"\ndone\n";
        fs::write(&path, script).expect("write fake repl script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    fn fake_config() -> ReplConfig {
        ReplConfig {
            program: PathBuf::from("/bin/sh"),
            args: vec![fake_repl_script().display().to_string()],
        }
    }

    #[test]
    fn spawn_consumes_banner_and_evals_a_line() {
        let mut repl = Repl::spawn(&fake_config()).expect("spawn");
        assert!(repl.is_alive());
        let result = repl.eval("hello").expect("eval");
        assert_eq!(result, "ECHO:hello");
    }

    #[test]
    fn eval_sees_fresh_prompt_each_time() {
        let mut repl = Repl::spawn(&fake_config()).expect("spawn");
        assert_eq!(repl.eval("one").unwrap(), "ECHO:one");
        assert_eq!(repl.eval("two").unwrap(), "ECHO:two");
    }

    #[test]
    fn is_alive_false_after_child_exits() {
        let mut repl = Repl::spawn(&fake_config()).expect("spawn");
        // Sending CRASH makes the fake script exit without printing a
        // trailing prompt, so eval() will hit EOF; that's expected here,
        // we only care about is_alive() afterwards.
        let _ = repl.stdin.write_all(b"CRASH\n");
        let _ = repl.stdin.flush();
        // give the child a moment to exit
        for _ in 0..50 {
            if !repl.is_alive() {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!repl.is_alive());
    }

    #[test]
    fn supervisor_spawns_lazily_and_evals() {
        let mut sup = Supervisor::new(fake_config());
        assert_eq!(sup.eval("hi").unwrap(), "ECHO:hi");
    }

    #[test]
    fn supervisor_restarts_after_crash() {
        let mut sup = Supervisor::new(fake_config());
        assert_eq!(sup.eval("first").unwrap(), "ECHO:first");

        // Force a crash by killing the underlying child directly.
        sup.repl.as_mut().unwrap().child.kill().unwrap();
        sup.repl.as_mut().unwrap().child.wait().unwrap();

        // Next eval should transparently respawn and succeed.
        let result = sup.eval("second").unwrap();
        assert_eq!(result, "ECHO:second");
    }

    #[test]
    fn for_dw_builds_expected_args() {
        let inputs = vec![("payload".to_string(), PathBuf::from("/tmp/p.json"))];
        let config = ReplConfig::for_dw(Path::new("/usr/local/bin/dw"), &inputs);
        assert_eq!(config.program, PathBuf::from("/usr/local/bin/dw"));
        assert_eq!(config.args, vec!["repl", "-s", "-i", "payload=/tmp/p.json"]);
    }

    #[test]
    fn drains_stderr_without_blocking() {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "blazewvr_fake_repl_stderr_{}_{id}.sh",
            std::process::id()
        ));
        let script = "#!/bin/sh\necho 'a warning' 1>&2\nprintf 'FAKE REPL\\n>>> '\nwhile IFS= read -r line; do\n  printf 'ECHO:%s\\n>>> ' \"$line\"\ndone\n";
        fs::write(&path, script).expect("write script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        let config = ReplConfig {
            program: PathBuf::from("/bin/sh"),
            args: vec![path.display().to_string()],
        };

        let mut repl = Repl::spawn(&config).expect("spawn");
        let result = repl.eval("x").expect("eval");
        // give the stderr-drain thread a moment to run before the process exits
        thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(result, "ECHO:x");
    }

    #[test]
    fn eval_returns_partial_output_on_eof_without_trailing_prompt() {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "blazewvr_fake_repl_eof_{}_{id}.sh",
            std::process::id()
        ));
        // Consumes exactly one line then exits without printing a next prompt,
        // forcing read_until_prompt to hit EOF instead of the ">>> " marker.
        let script = "#!/bin/sh\nprintf 'FAKE REPL\\n>>> '\nread -r line\nexit 0\n";
        fs::write(&path, script).expect("write script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        let config = ReplConfig {
            program: PathBuf::from("/bin/sh"),
            args: vec![path.display().to_string()],
        };

        let mut repl = Repl::spawn(&config).expect("spawn");
        let result = repl.eval("x").expect("eval should not error on EOF");
        assert_eq!(result, "");
    }

    #[test]
    fn for_dw_with_no_inputs() {
        let config = ReplConfig::for_dw(Path::new("dw"), &[]);
        assert_eq!(config.args, vec!["repl", "-s"]);
    }
}
