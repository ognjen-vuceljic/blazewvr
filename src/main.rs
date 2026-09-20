use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod colorize;
mod config;
mod dwl_highlight;
mod flatten;
mod repl;
mod run_once;
mod sidecar;
mod status;
mod watch;

/// blazewvr — fast local playground for DataWeave scripts
#[derive(Parser)]
#[command(name = "blazewvr", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a DataWeave script against one or more inputs
    Run {
        script: String,
        #[arg(short, long)]
        input: Vec<String>,
        /// Module resolution path(s), e.g. --path=dir1:dir2
        #[arg(long)]
        path: Option<String>,
    },
    /// Interactive read-eval-print loop
    Repl {
        /// Module resolution path(s), e.g. --path=dir1:dir2
        #[arg(long)]
        path: Option<String>,
    },
    /// Validate a DataWeave script without running it
    Validate { script: String },
    /// Watch a script (and its inputs) and re-evaluate on every save
    Watch {
        script: String,
        #[arg(short, long)]
        input: Vec<String>,
        /// Module resolution path(s), e.g. --path=dir1:dir2
        #[arg(long)]
        path: Option<String>,
    },
}

/// Renders the (currently stubbed) response for a parsed command.
fn dispatch(command: Commands) -> String {
    match command {
        Commands::Run {
            script,
            input,
            path,
        } => {
            format!(
                "{} {script} (inputs: {input:?}, path: {path:?})",
                "[run] not yet implemented:".yellow()
            )
        }
        Commands::Repl { path } => {
            format!("{} (path: {path:?})", "[repl] not yet implemented".yellow())
        }
        Commands::Validate { script } => {
            format!("{} {script}", "[validate] not yet implemented:".yellow())
        }
        Commands::Watch { .. } => {
            unreachable!("Watch is handled directly in main(), not dispatch()")
        }
    }
}

/// Builds the supervisor + watch target and runs the poll loop forever
/// (until the process is killed), printing each eval's result.
fn run_watch(script: String, input: Vec<String>, path: Option<String>) -> anyhow::Result<()> {
    let inputs: Vec<(String, PathBuf)> = input
        .iter()
        .map(|s| watch::parse_input(s))
        .collect::<Result<_, _>>()
        .map_err(anyhow::Error::msg)?;

    let mut config = repl::ReplConfig::for_dw(Path::new("dw"), &inputs);
    if let Some(p) = &path {
        config = config.with_module_path(p);
    }

    let target = watch::WatchTarget {
        script: PathBuf::from(script),
        inputs,
    };
    watch::run_loop(
        repl::Supervisor::new(config),
        &target,
        Duration::from_millis(300),
        |out| println!("{out}\n"),
        || true,
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Commands::Watch {
            script,
            input,
            path,
        } => run_watch(script, input, path),
        other => {
            println!("{}", dispatch(other));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_reports_script_and_inputs() {
        let out = dispatch(Commands::Run {
            script: "output json --- payload".into(),
            input: vec!["payload.json".into()],
            path: None,
        });
        assert!(out.contains("output json --- payload"));
        assert!(out.contains("payload.json"));
    }

    #[test]
    fn run_reports_module_path() {
        let out = dispatch(Commands::Run {
            script: "payload".into(),
            input: vec![],
            path: Some("dir1:dir2".into()),
        });
        assert!(out.contains("dir1:dir2"));
    }

    #[test]
    fn repl_reports_stub() {
        let out = dispatch(Commands::Repl { path: None });
        assert!(out.contains("not yet implemented"));
    }

    #[test]
    fn repl_reports_module_path() {
        let out = dispatch(Commands::Repl {
            path: Some("dir1".into()),
        });
        assert!(out.contains("dir1"));
    }

    #[test]
    fn cli_parses_path_flag() {
        let cli = Cli::parse_from(["blazewvr", "run", "s.dwl", "--path", "a:b"]);
        match cli.command {
            Commands::Run { path, .. } => assert_eq!(path, Some("a:b".to_string())),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn validate_reports_script() {
        let out = dispatch(Commands::Validate {
            script: "foo.dwl".into(),
        });
        assert!(out.contains("foo.dwl"));
    }

    #[test]
    fn run_watch_rejects_invalid_input_format() {
        let result = run_watch("script.dwl".into(), vec!["bad-input".into()], None);
        assert!(result.is_err());
    }

    #[test]
    fn cli_parses_watch_subcommand() {
        let cli = Cli::parse_from(["blazewvr", "watch", "s.dwl", "-i", "a=b.json"]);
        match cli.command {
            Commands::Watch { script, input, .. } => {
                assert_eq!(script, "s.dwl");
                assert_eq!(input, vec!["a=b.json".to_string()]);
            }
            _ => panic!("expected Watch"),
        }
    }

    #[test]
    fn cli_parses_run_subcommand() {
        let cli = Cli::parse_from(["blazewvr", "run", "script.dwl", "-i", "a.json"]);
        match cli.command {
            Commands::Run { script, input, .. } => {
                assert_eq!(script, "script.dwl");
                assert_eq!(input, vec!["a.json".to_string()]);
            }
            _ => panic!("expected Run"),
        }
    }
}
