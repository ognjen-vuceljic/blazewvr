use clap::{Parser, Subcommand};
use colored::Colorize;

mod colorize;
mod flatten;
mod repl;
mod status;

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
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    println!("{}", dispatch(cli.command));
    Ok(())
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
