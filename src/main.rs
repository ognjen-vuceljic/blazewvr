use clap::{Parser, Subcommand};
use colored::Colorize;

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

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run { script, input } => {
            println!("{} {script} (inputs: {input:?})", "[run] not yet implemented:".yellow());
        }
        Commands::Repl => {
            println!("{}", "[repl] not yet implemented".yellow());
        }
        Commands::Validate { script } => {
            println!("{} {script}", "[validate] not yet implemented:".yellow());
        }
    }

    Ok(())
}
