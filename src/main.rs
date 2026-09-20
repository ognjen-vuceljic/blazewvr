use clap::{Parser, Subcommand};
use colored::Colorize;

mod flatten;

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
    },
    /// Interactive read-eval-print loop
    Repl,
    /// Validate a DataWeave script without running it
    Validate { script: String },
}

/// Renders the (currently stubbed) response for a parsed command.
fn dispatch(command: Commands) -> String {
    match command {
        Commands::Run { script, input } => {
            format!(
                "{} {script} (inputs: {input:?})",
                "[run] not yet implemented:".yellow()
            )
        }
        Commands::Repl => format!("{}", "[repl] not yet implemented".yellow()),
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
        });
        assert!(out.contains("output json --- payload"));
        assert!(out.contains("payload.json"));
    }

    #[test]
    fn repl_reports_stub() {
        let out = dispatch(Commands::Repl);
        assert!(out.contains("not yet implemented"));
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
            Commands::Run { script, input } => {
                assert_eq!(script, "script.dwl");
                assert_eq!(input, vec!["a.json".to_string()]);
            }
            _ => panic!("expected Run"),
        }
    }
}
